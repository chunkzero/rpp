//! Directory-wide optimization and reporting.

use std::fs;
use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;

use crate::error::{Error, Result};
use crate::file::squash_file;
use crate::options::SquashOptions;
use crate::walk::walk_files;

/// Summary of a [`squash_dir`] run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
}

/// Optimize files in-place within `dir`, honoring `opts`.
///
/// Walks `dir`, deletes files matching any `strip` glob, then optimizes each
/// remaining JSON/PNG file, rewriting it in place when a strictly smaller
/// result is produced. PNG optimization is CPU-heavy, so the per-file work is
/// parallelized with rayon.
///
/// Recoverable issues (invalid JSON, PNG failures) are recorded as warnings in
/// the returned [`SquashReport`] rather than returned as errors. Hard I/O
/// failures abort with an [`Error`].
pub fn squash_dir(dir: &Path, opts: &SquashOptions) -> Result<SquashReport> {
    let strip_set = build_glob_set(&opts.strip)?;
    let (stripped, kept): (Vec<_>, Vec<_>) = walk_files(dir)?
        .into_iter()
        .partition(|file| strip_set.is_match(&file.rel));

    for file in &stripped {
        fs::remove_file(&file.abs).map_err(|err| Error::io(&file.abs, err))?;
    }

    // Collecting in path order keeps errors and warnings deterministic.
    let outcomes: Vec<Result<Outcome>> = kept
        .par_iter()
        .map(|file| optimize_one(&file.abs, &file.rel, opts))
        .collect();

    let mut report = SquashReport {
        files_stripped: stripped.len(),
        ..SquashReport::default()
    };
    for outcome in outcomes {
        let outcome = outcome?;
        report.warnings.extend(outcome.warnings);
        if let Some((before, after)) = outcome.sizes {
            report.files_optimized += 1;
            report.bytes_before += before;
            report.bytes_after += after;
        }
    }
    Ok(report)
}

/// Outcome of optimizing one file.
struct Outcome {
    /// Sizes before and after, when the file was rewritten.
    sizes: Option<(u64, u64)>,
    warnings: Vec<String>,
}

/// Read, optimize, and (if smaller) rewrite a single file.
fn optimize_one(abs: &Path, rel: &str, opts: &SquashOptions) -> Result<Outcome> {
    let contents = fs::read(abs).map_err(|err| Error::io(abs, err))?;
    let before = contents.len() as u64;

    let mut warnings = Vec::new();
    let sizes = match squash_file(rel, &contents, opts, &mut warnings) {
        Some(bytes) => {
            fs::write(abs, &bytes).map_err(|err| Error::io(abs, err))?;
            Some((before, bytes.len() as u64))
        }
        None => None,
    };
    Ok(Outcome { sizes, warnings })
}

fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
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
