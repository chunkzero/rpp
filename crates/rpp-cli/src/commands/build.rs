//! `rpp build`: resolve plugins, run an incremental build, then squash + zip.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Args;
use rpp::config::{PngSetting, SquashEngine};
use rpp::engine::{BuildResult, Engine};
use rpp_squash::{run_packsquash, squash_zip, PngLevel, SquashOptions, SquashReport};

use self::release::ReleaseInputs;
use crate::codegen;
use crate::project::Project;
use crate::ui;

mod release;

/// Arguments for `rpp build`.
#[derive(Debug, Clone, Default, Args)]
pub struct BuildArgs {
    /// Clean the cache first (a full rebuild).
    #[arg(long)]
    pub no_cache: bool,
    /// Skip the squash/zip phase.
    #[arg(long)]
    pub no_squash: bool,
    /// Worker thread count (0 / unset = available parallelism).
    #[arg(long)]
    pub jobs: Option<usize>,
}

/// Run the build command from the current directory.
pub fn run(dir: &Path, args: BuildArgs) -> Result<()> {
    let mut project = Project::discover(dir)?;
    ui::intro("Build resource pack");
    if let Some(jobs) = args.jobs {
        project.config.build.workers = jobs;
    }

    codegen::write_best_effort(&project.root);

    if args.no_cache {
        clean_cache(&project)?;
    }
    let squash = &project.config.build.squash;
    if args.no_squash && squash.enabled && squash.zip {
        remove_stale_zip(&project.release_zip())?;
    }

    let engine = resolve_plugins(&project)?;

    ui::phase("Building");
    let build_start = Instant::now();
    let result = engine.build().context("running the build")?;
    ui::detail(format!(
        "finished in {}",
        ui::fmt_duration(build_start.elapsed())
    ));
    report_build(&result);

    if !args.no_squash && project.config.build.squash.enabled {
        run_squash(&project, &result)?;
    } else {
        ui::detail("squash disabled");
    }

    ui::success(format!(
        "Done in {} -> {}",
        ui::fmt_duration(result.duration),
        project.output_dir().display()
    ));
    Ok(())
}

/// Remove only the cache and keep the output, which the engine resyncs.
fn clean_cache(project: &Project) -> Result<()> {
    ui::phase("Cleaning cache");
    let cache = release::cache_dir(project);
    if cache.exists() {
        std::fs::remove_dir_all(&cache).with_context(|| format!("removing {}", cache.display()))?;
    }
    Ok(())
}

/// Remove a release archive left by an earlier build so it cannot go stale.
fn remove_stale_zip(zip: &Path) -> Result<()> {
    match std::fs::remove_file(zip) {
        Ok(()) => ui::detail(format!("removed stale release archive {}", zip.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("removing {}", zip.display())),
    }
    Ok(())
}

fn resolve_plugins(project: &Project) -> Result<Engine> {
    ui::phase("Resolving plugins");
    let start = Instant::now();
    let engine = project.build_engine(&mut None)?;
    let plugin_count = engine.plugin_count();
    ui::detail(format!(
        "{plugin_count} plugin{} resolved in {}",
        if plugin_count == 1 { "" } else { "s" },
        ui::fmt_duration(start.elapsed())
    ));
    Ok(engine)
}

fn report_build(result: &BuildResult) {
    ui::detail(format!(
        "processed {}, cached {}, generated {}, dropped {}",
        result.processed, result.cached, result.generated, result.dropped
    ));
    ui::detail(format!(
        "{} written, {} removed",
        result.changes.written.len(),
        result.changes.removed.len()
    ));
    if !result.changes.external.written.is_empty() || !result.changes.external.removed.is_empty() {
        ui::detail(format!(
            "{} external written, {} external removed",
            result.changes.external.written.len(),
            result.changes.external.removed.len()
        ));
    }
}

/// Run the squash + zip phase against the materialized output directory.
fn run_squash(project: &Project, result: &BuildResult) -> Result<()> {
    let squash = &project.config.build.squash;
    if !squash.zip {
        ui::detail("release archive disabled; squash skipped");
        return Ok(());
    }
    match squash.engine {
        SquashEngine::Packsquash => squash_packsquash(project),
        SquashEngine::Builtin => squash_builtin(project, result),
    }
}

/// Hand the output directory to the external PackSquash binary.
fn squash_packsquash(project: &Project) -> Result<()> {
    let squash = &project.config.build.squash;
    let zip_path = project.release_zip();
    ui::phase("Squashing (packsquash)");
    let start = Instant::now();
    let options_file = squash
        .packsquash_options
        .as_ref()
        .map(|p| project.root.join(p));
    run_packsquash(
        &squash.packsquash_binary,
        &project.output_dir(),
        &zip_path,
        options_file.as_deref(),
    )
    .context("running packsquash")?;
    match &options_file {
        Some(file) => ui::detail(format!("output path controlled by {}", file.display())),
        None => ui::detail(format!("zip -> {}", zip_path.display())),
    }
    ui::detail(format!(
        "packsquash finished in {}",
        ui::fmt_duration(start.elapsed())
    ));
    Ok(())
}

/// Zip the output with in-memory optimization, leaving the loose output unsquashed.
///
/// The existing archive is kept when the build changed no output and the archive still
/// matches the record written with it for the same inputs, which include the engine's
/// digest of the output.
fn squash_builtin(project: &Project, result: &BuildResult) -> Result<()> {
    let squash = &project.config.build.squash;
    let zip_path = project.release_zip();
    ui::phase("Squashing (builtin)");
    let start = Instant::now();
    let inputs = ReleaseInputs::current(project, result)?;
    let unchanged_output = result.changes.written.is_empty() && result.changes.removed.is_empty();
    if unchanged_output && release::is_current(project, &inputs, &zip_path) {
        ui::detail(format!(
            "release archive unchanged -> {}",
            zip_path.display()
        ));
        return Ok(());
    }
    release::forget(project)?;
    let opts = SquashOptions {
        json: squash.json,
        png: png_level(squash.png),
        strip: squash.strip.clone(),
    };
    let report = squash_zip(&project.output_dir(), &zip_path, &opts)
        .with_context(|| format!("writing zip {}", zip_path.display()))?;
    report_squash(&report);
    release::remember(project, inputs, &zip_path)?;
    ui::detail(format!("zip -> {}", zip_path.display()));
    ui::detail(format!(
        "squash finished in {}",
        ui::fmt_duration(start.elapsed())
    ));
    Ok(())
}

fn report_squash(report: &SquashReport) {
    ui::detail(format!(
        "optimized {} file{}: {} -> {} ({} saved)",
        report.files_optimized,
        if report.files_optimized == 1 { "" } else { "s" },
        ui::fmt_bytes(report.bytes_before),
        ui::fmt_bytes(report.bytes_after),
        ui::fmt_savings(report.bytes_before, report.bytes_after),
    ));
    for warning in &report.warnings {
        ui::warn(warning);
    }
}

fn png_level(setting: PngSetting) -> PngLevel {
    match setting {
        PngSetting::Off => PngLevel::Off,
        PngSetting::Fast => PngLevel::Fast,
        PngSetting::Max => PngLevel::Max,
    }
}
