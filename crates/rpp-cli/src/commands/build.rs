//! `rpp build`: resolve plugins, run an incremental build, then squash + zip.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rpp::config::PngSetting;
use rpp::engine::BuildResult;
use rpp_squash::{
    copy_tree, run_packsquash, squash_dir, write_zip, PngLevel, SquashOptions, SquashReport,
    ZipOptions,
};

use crate::luals;
use crate::project::Project;
use crate::ui;

/// Arguments for `rpp build`.
#[derive(Debug, Clone, Default)]
pub struct BuildArgs {
    /// Clean the cache before building (a full rebuild).
    pub no_cache: bool,
    /// Skip the squash/zip phase entirely.
    pub no_squash: bool,
    /// Override the worker count (`None` = config / available parallelism).
    pub jobs: Option<usize>,
}

/// Run the build command from the current directory.
pub fn run(dir: &Path, args: BuildArgs) -> Result<()> {
    let mut project = Project::discover(dir)?;
    if let Some(jobs) = args.jobs {
        project.config.build.workers = jobs;
    }

    // Refresh editor definitions if stale (best-effort; non-fatal).
    let _ = luals::write_if_stale(&project.root.join(".rpp").join("api"));

    if args.no_cache {
        ui::phase("Cleaning cache");
        // Remove only the cache, keep the output (the engine resyncs it).
        let cache = project.root.join(".rpp").join("cache");
        if cache.exists() {
            std::fs::remove_dir_all(&cache)
                .with_context(|| format!("removing {}", cache.display()))?;
        }
    }

    ui::phase("Resolving plugins");
    let resolve_start = Instant::now();
    let engine = project.build_engine()?;
    let plugin_count = engine.plugin_count();
    ui::detail(format!(
        "{plugin_count} plugin{} resolved in {}",
        if plugin_count == 1 { "" } else { "s" },
        ui::fmt_duration(resolve_start.elapsed())
    ));

    ui::phase("Building");
    let build_start = Instant::now();
    let result = engine.build().context("running the build")?;
    ui::detail(format!(
        "finished in {}",
        ui::fmt_duration(build_start.elapsed())
    ));
    report_build(&result);

    let output_dir = project.output_dir().clone();

    if !args.no_squash && project.config.build.squash.enabled {
        run_squash(&project, &output_dir)?;
    } else {
        ui::detail("squash disabled");
    }

    ui::success(format!(
        "Done in {} -> {}",
        ui::fmt_duration(result.duration),
        output_dir.display()
    ));
    Ok(())
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
fn run_squash(project: &Project, output_dir: &Path) -> Result<()> {
    let cfg = &project.config.build.squash;
    let pack_name = &project.config.pack.name;
    let zip_path = output_dir.join(format!("{pack_name}.zip"));

    if !cfg.zip {
        ui::detail("release archive disabled; squash skipped");
        return Ok(());
    }

    match cfg.engine.as_str() {
        "packsquash" => {
            ui::phase("Squashing (packsquash)");
            let start = Instant::now();
            let options_file = cfg
                .packsquash_options
                .as_ref()
                .map(|p| project.root.join(p));
            run_packsquash(
                &cfg.packsquash_binary,
                output_dir,
                &zip_path,
                options_file.as_deref(),
            )
            .context("running packsquash")?;
            ui::detail(format!("zip -> {}", zip_path.display()));
            ui::detail(format!(
                "packsquash finished in {}",
                ui::fmt_duration(start.elapsed())
            ));
        }
        _ => {
            ui::phase("Squashing (builtin)");
            let start = Instant::now();
            let staging = stage_release(project, output_dir, &zip_path)?;
            let opts = SquashOptions::builder()
                .json(cfg.json)
                .png(png_level(cfg.png))
                .strip(cfg.strip.clone())
                .build();
            let report = squash_dir(staging.path(), &opts).context("optimizing release files")?;
            report_squash(&report);

            write_release_zip(staging.path(), &zip_path)?;
            ui::detail(format!("zip -> {}", zip_path.display()));
            ui::detail(format!(
                "squash finished in {}",
                ui::fmt_duration(start.elapsed())
            ));
        }
    }
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

/// Copy the engine-owned loose output to a temporary release staging directory.
fn stage_release(
    project: &Project,
    output_dir: &Path,
    zip_path: &Path,
) -> Result<tempfile::TempDir> {
    let temp_root = project.root.join(".rpp");
    std::fs::create_dir_all(&temp_root)
        .with_context(|| format!("creating {}", temp_root.display()))?;
    let staging = tempfile::Builder::new()
        .prefix("release-")
        .tempdir_in(&temp_root)
        .context("creating release staging directory")?;
    copy_tree(output_dir, staging.path(), zip_path).context("staging release files")?;
    Ok(staging)
}

/// Write a deterministic release zip through a sibling temporary file.
fn write_release_zip(staging_dir: &Path, zip_path: &Path) -> Result<()> {
    let parent = zip_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".{}.zip.tmp",
        zip_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("pack")
    ));

    write_zip(staging_dir, &tmp, &ZipOptions::default())
        .with_context(|| format!("writing zip {}", zip_path.display()))?;
    replace_file(&tmp, zip_path)
        .with_context(|| format!("moving zip into {}", zip_path.display()))?;
    Ok(())
}

fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    if target.exists() {
        std::fs::remove_file(target)?;
    }
    std::fs::rename(source, target)
}

fn png_level(setting: PngSetting) -> PngLevel {
    match setting {
        PngSetting::Off => PngLevel::Off,
        PngSetting::Fast => PngLevel::Fast,
        PngSetting::Max => PngLevel::Max,
    }
}
