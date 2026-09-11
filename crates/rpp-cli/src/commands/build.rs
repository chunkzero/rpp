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

    if args.no_squash && project.config.build.squash.enabled && project.config.build.squash.zip {
        let zip = project
            .output_dir()
            .join(format!("{}.zip", project.config.pack.name));
        match std::fs::remove_file(&zip) {
            Ok(()) => ui::detail(format!("removed stale release archive {}", zip.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("removing {}", zip.display())),
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
            match &options_file {
                Some(file) => ui::detail(format!("output path controlled by {}", file.display())),
                None => ui::detail(format!("zip -> {}", zip_path.display())),
            }
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
    let parent = zip_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::Builder::new()
        .prefix(".rpp-release-")
        .tempfile_in(parent)
        .with_context(|| format!("staging zip in {}", parent.display()))?;

    write_zip(staging_dir, tmp.path(), &ZipOptions::default())
        .with_context(|| format!("writing zip {}", zip_path.display()))?;
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("flushing zip {}", zip_path.display()))?;
    // Temporary files are private by default; the archive is a shareable artifact.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o644))
            .with_context(|| format!("setting permissions on {}", zip_path.display()))?;
    }
    tmp.persist(zip_path)
        .map_err(|error| error.error)
        .with_context(|| format!("replacing zip {}", zip_path.display()))?;
    Ok(())
}

fn png_level(setting: PngSetting) -> PngLevel {
    match setting {
        PngSetting::Off => PngLevel::Off,
        PngSetting::Fast => PngLevel::Fast,
        PngSetting::Max => PngLevel::Max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_zip_write_preserves_archive_and_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let input = tempfile::tempdir().unwrap();
        std::fs::write(input.path().join("pack.mcmeta"), "{}").unwrap();
        let path = dir.path().join("pack.zip");
        write_release_zip(input.path(), &path).unwrap();
        let previous = std::fs::read(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o644
            );
        }

        assert!(write_release_zip(&input.path().join("missing"), &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_zip_persist_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let input = tempfile::tempdir().unwrap();
        let path = dir.path().join("pack.zip");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), "previous").unwrap();

        assert!(write_release_zip(input.path(), &path).is_err());
        assert_eq!(
            std::fs::read_to_string(path.join("keep")).unwrap(),
            "previous"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_zip_writes_use_independent_staging() {
        let dir = tempfile::tempdir().unwrap();
        let input = tempfile::tempdir().unwrap();
        std::fs::write(input.path().join("pack.mcmeta"), "{}").unwrap();
        let path = dir.path().join("pack.zip");
        let fixed_temp = dir.path().join(".pack.zip.tmp");
        std::fs::write(&fixed_temp, "unrelated").unwrap();
        write_release_zip(input.path(), &path).unwrap();
        let previous = std::fs::read(&path).unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let one = scope.spawn(|| {
                barrier.wait();
                write_release_zip(input.path(), &path).unwrap();
            });
            barrier.wait();
            write_release_zip(input.path(), &path).unwrap();
            one.join().unwrap();
        });
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        assert_eq!(std::fs::read_to_string(fixed_temp).unwrap(), "unrelated");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
