//! Directory-wide optimization and reporting.

use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use globset::{Glob, GlobSetBuilder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::file::squash_file_collecting;
use crate::options::SquashOptions;

/// Per-file detail recorded during a directory squash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDetail {
    /// Path relative to the squashed directory (forward-slash).
    pub path: String,
    /// Byte size before optimization.
    pub before: u64,
    /// Byte size after optimization.
    pub after: u64,
}

impl FileDetail {
    /// Bytes saved (`before - after`).
    pub fn saved(&self) -> u64 {
        self.before.saturating_sub(self.after)
    }
}

/// Summary of a [`crate::squash_dir`] run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquashReport {
    /// Number of files whose contents were rewritten smaller.
    pub files_optimized: usize,
    /// Number of files deleted by `strip` globs.
    pub files_stripped: usize,
    /// Total bytes of optimized files before optimization.
    pub bytes_before: u64,
    /// Total bytes of optimized files after optimization.
    pub bytes_after: u64,
    /// Recoverable warnings (e.g. invalid JSON passed through).
    pub warnings: Vec<String>,
    /// Per-file detail for each optimized file.
    pub details: Vec<FileDetail>,
}

impl SquashReport {
    /// Total bytes saved across all optimized files.
    pub fn bytes_saved(&self) -> u64 {
        self.bytes_before.saturating_sub(self.bytes_after)
    }

    /// Savings as a fraction in `0.0..=1.0` (0.0 when nothing was optimized).
    pub fn ratio(&self) -> f64 {
        if self.bytes_before == 0 {
            0.0
        } else {
            self.bytes_saved() as f64 / self.bytes_before as f64
        }
    }
}

impl fmt::Display for SquashReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "squash: {} optimized, {} stripped, {} -> {} ({:.1}% smaller)",
            self.files_optimized,
            self.files_stripped,
            human_bytes(self.bytes_before),
            human_bytes(self.bytes_after),
            self.ratio() * 100.0,
        )?;
        if !self.warnings.is_empty() {
            write!(f, ", {} warning(s)", self.warnings.len())?;
        }
        Ok(())
    }
}

/// Format a byte count with a binary unit suffix.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Optimize files in-place within `dir`, honoring `opts`.
///
/// Walks `dir`, deletes files matching any `strip` glob, then applies
/// [`crate::squash_file`] to each remaining file, rewriting it in place when a
/// strictly smaller result is produced. PNG optimization is CPU-heavy, so the
/// per-file work is parallelized with rayon.
///
/// Returns a [`SquashReport`] aggregating counts, byte totals, warnings, and
/// per-file detail. Recoverable issues (invalid JSON, PNG failures) are recorded
/// as warnings rather than returned as errors. Hard I/O failures abort with an
/// [`Error`].
pub fn squash_dir(dir: &Path, opts: &SquashOptions) -> Result<SquashReport> {
    // Compile the strip globs once.
    let strip_set = build_glob_set(&opts.strip)?;

    // Collect candidate files first (walkdir is not Send-friendly to drive from
    // rayon directly, and we must resolve strips before optimizing).
    let mut to_strip = Vec::new();
    let mut to_optimize = Vec::new();

    for entry in WalkDir::new(dir).follow_links(false) {
        let entry = entry.map_err(|err| {
            let path = err
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| dir.to_path_buf());
            Error::io(path, err.into())
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path().to_path_buf();
        let rel = relative_forward_slash(dir, &abs);
        if strip_set.is_match(&rel) {
            to_strip.push(abs);
        } else {
            to_optimize.push((abs, rel));
        }
    }

    let mut report = SquashReport::default();

    // Delete stripped files (sequential; cheap).
    for path in &to_strip {
        fs::remove_file(path).map_err(|err| Error::io(path, err))?;
    }
    report.files_stripped = to_strip.len();

    // Optimize remaining files in parallel.
    let results: Mutex<Vec<FileResult>> = Mutex::new(Vec::with_capacity(to_optimize.len()));
    let first_error: Mutex<Option<Error>> = Mutex::new(None);

    to_optimize
        .par_iter()
        .for_each(|(abs, rel)| match optimize_one(abs, rel, opts) {
            Ok(result) => results.lock().expect("results mutex poisoned").push(result),
            Err(err) => {
                let mut slot = first_error.lock().expect("error mutex poisoned");
                if slot.is_none() {
                    *slot = Some(err);
                }
            }
        });

    if let Some(err) = first_error.into_inner().expect("error mutex poisoned") {
        return Err(err);
    }

    // Aggregate deterministically: sort by path so the report is stable.
    let mut results = results.into_inner().expect("results mutex poisoned");
    results.sort_by(|a, b| a.path.cmp(&b.path));

    for result in results {
        report.warnings.extend(result.warnings);
        if let Some(detail) = result.detail {
            report.files_optimized += 1;
            report.bytes_before += detail.before;
            report.bytes_after += detail.after;
            report.details.push(detail);
        }
    }

    Ok(report)
}

/// Outcome of optimizing one file.
struct FileResult {
    path: String,
    detail: Option<FileDetail>,
    warnings: Vec<String>,
}

/// Read, optimize, and (if smaller) rewrite a single file.
fn optimize_one(abs: &Path, rel: &str, opts: &SquashOptions) -> Result<FileResult> {
    let contents = fs::read(abs).map_err(|err| Error::io(abs, err))?;
    let before = contents.len() as u64;

    let mut warnings = Vec::new();
    let optimized = squash_file_collecting(rel, contents, opts, &mut warnings)?;

    let detail = match optimized {
        Some(bytes) => {
            let after = bytes.len() as u64;
            fs::write(abs, &bytes).map_err(|err| Error::io(abs, err))?;
            Some(FileDetail {
                path: rel.to_string(),
                before,
                after,
            })
        }
        None => None,
    };

    Ok(FileResult {
        path: rel.to_string(),
        detail,
        warnings,
    })
}

/// Build a [`globset::GlobSet`] from the configured patterns.
fn build_glob_set(patterns: &[String]) -> Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|source| Error::InvalidGlob {
            pattern: pattern.clone(),
            source,
        })?;
        builder.add(glob);
    }
    builder.build().map_err(|source| Error::InvalidGlob {
        pattern: patterns.join(", "),
        source,
    })
}

/// Compute the forward-slash relative path of `abs` under `root`.
fn relative_forward_slash(root: &Path, abs: &Path) -> String {
    let rel = abs.strip_prefix(root).unwrap_or(abs);
    let mut out = String::new();
    for (i, comp) in rel.components().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(&comp.as_os_str().to_string_lossy());
    }
    out
}

/// Expose the relative-path helper to sibling modules (zip).
pub(crate) fn rel_path(root: &Path, abs: &Path) -> String {
    relative_forward_slash(root, abs)
}
