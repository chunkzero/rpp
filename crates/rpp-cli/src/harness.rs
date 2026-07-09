//! Isolated, structured project builds for plugin integration tests.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rpp::engine::BuildResult;
use sha2::{Digest, Sha256};

use crate::project::Project;

/// A reproducible project build host that does not load user-global plugins.
#[derive(Debug, Clone)]
pub struct BuildHarness {
    root: PathBuf,
    jobs: usize,
}

impl BuildHarness {
    /// Create a harness for the project rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            jobs: 1,
        }
    }

    /// Set a deterministic worker count. The default is one worker.
    pub fn jobs(mut self, jobs: usize) -> Self {
        self.jobs = jobs;
        self
    }

    /// Run one build and return structured output rather than CLI prose.
    pub fn build(&self) -> Result<HarnessBuild> {
        let mut project = Project::discover_isolated(&self.root)?;
        project.config.build.workers = self.jobs;
        let engine = project.build_engine()?;
        // Preserve the attributed RPP diagnostic as the top-level harness
        // error; plugin suites commonly assert it directly.
        let result = engine.build().map_err(anyhow::Error::new)?;
        let output_dir = project.output_dir();
        let output_digest = tree_digest(&output_dir)?;
        let external_digest = paths_digest(&result.changes.external.written)?;
        Ok(HarnessBuild {
            result,
            output_dir,
            output_digest,
            external_digest,
        })
    }

    /// Delete only the incremental cache, preserve materialized and externally
    /// owned outputs, and then run a full build. This matches `rpp build
    /// --no-cache` and deliberately retains the external ownership manifest so
    /// stale generated sources can still be removed.
    pub fn build_no_cache(&self) -> Result<HarnessBuild> {
        let cache = self.root.join(".rpp/cache");
        if cache.exists() {
            std::fs::remove_dir_all(&cache)
                .with_context(|| format!("removing incremental cache {}", cache.display()))?;
        }
        self.build()
    }

    /// Remove all RPP-owned artifacts without resolving plugins.
    pub fn clean(&self) -> Result<()> {
        Project::discover_isolated(&self.root)?.clean_artifacts()
    }

    /// Compare two cold builds and then require a no-op warm build.
    pub fn verify_reproducible(&self) -> Result<ReproducibilityReport> {
        self.clean()?;
        let first = self.build()?;
        self.clean()?;
        let second = self.build()?;
        if first.output_digest != second.output_digest
            || first.external_digest != second.external_digest
        {
            bail!("two clean builds produced different output digests");
        }

        let warm = self.build()?;
        if build_changed_outputs(&warm.result) {
            bail!("warm build rewrote or removed outputs");
        }

        Ok(ReproducibilityReport {
            output_digest: second.output_digest,
            external_digest: second.external_digest,
            cold: second.result,
            warm: warm.result,
        })
    }
}

fn build_changed_outputs(result: &BuildResult) -> bool {
    !result.changes.written.is_empty()
        || !result.changes.removed.is_empty()
        || !result.changes.external.written.is_empty()
        || !result.changes.external.removed.is_empty()
}

/// Structured result of one harness build.
#[derive(Debug, Clone)]
pub struct HarnessBuild {
    /// Core incremental build result.
    pub result: BuildResult,
    /// Materialized loose pack directory.
    pub output_dir: PathBuf,
    /// SHA-256 of sorted output paths and bytes.
    pub output_digest: [u8; 32],
    /// SHA-256 of external paths written by this build and their bytes.
    pub external_digest: [u8; 32],
}

/// Outcome of cold/cold/warm reproducibility verification.
#[derive(Debug, Clone)]
pub struct ReproducibilityReport {
    /// Stable loose-pack digest.
    pub output_digest: [u8; 32],
    /// Stable declared-external-output digest.
    pub external_digest: [u8; 32],
    /// Second cold build report.
    pub cold: BuildResult,
    /// Warm no-op build report.
    pub warm: BuildResult,
}

fn tree_digest(root: &Path) -> Result<[u8; 32]> {
    let mut paths = Vec::new();
    if root.exists() {
        collect_files(root, root, &mut paths)?;
    }
    paths.sort();
    digest_files(
        paths
            .iter()
            .map(|relative| (relative.as_path(), root.join(relative))),
    )
}

fn paths_digest(paths: &[PathBuf]) -> Result<[u8; 32]> {
    let mut paths = paths.to_vec();
    paths.sort();
    digest_files(paths.iter().map(|path| (path.as_path(), path.clone())))
}

fn digest_files<'a>(files: impl IntoIterator<Item = (&'a Path, PathBuf)>) -> Result<[u8; 32]> {
    let mut digest = Sha256::new();
    for (label, path) in files {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading generated output {}", path.display()))?;
        let label = label.to_string_lossy();
        digest.update((label.len() as u64).to_le_bytes());
        digest.update(label.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(digest.finalize().into())
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading {}", path.display()))?;
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}
