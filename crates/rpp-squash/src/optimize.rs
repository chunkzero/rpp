//! Per-file release optimization and reporting.

use std::fs;

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::error::{Error, Result};
use crate::file::squash_file;
use crate::options::SquashOptions;
use crate::walk::WalkedFile;

/// Summary of a [`crate::squash_zip`] run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SquashReport {
    /// Number of files whose archived contents are smaller than the source.
    pub files_optimized: usize,
    /// Number of files left out of the archive by `strip` globs.
    pub files_stripped: usize,
    /// Total bytes of optimized files before optimization.
    pub bytes_before: u64,
    /// Total bytes of optimized files after optimization.
    pub bytes_after: u64,
    /// Recoverable warnings (e.g. invalid JSON passed through).
    pub warnings: Vec<String>,
}

impl SquashReport {
    pub(crate) fn record(&mut self, outcome: Outcome) {
        self.warnings.extend(outcome.warnings);
        if let Some((before, after)) = outcome.sizes {
            self.files_optimized += 1;
            self.bytes_before += before;
            self.bytes_after += after;
        }
    }
}

/// Outcome of optimizing one file.
#[derive(Default)]
pub(crate) struct Outcome {
    /// Sizes before and after, when the contents shrank.
    sizes: Option<(u64, u64)>,
    warnings: Vec<String>,
}

/// Read `file` and return the contents to archive: the optimized bytes when
/// strictly smaller, otherwise the original bytes.
pub(crate) fn optimize(file: &WalkedFile, opts: &SquashOptions) -> Result<(Vec<u8>, Outcome)> {
    let contents = fs::read(&file.abs).map_err(|err| Error::io(&file.abs, err))?;
    let mut outcome = Outcome::default();
    match squash_file(&file.rel, &contents, opts, &mut outcome.warnings) {
        Some(optimized) => {
            outcome.sizes = Some((contents.len() as u64, optimized.len() as u64));
            Ok((optimized, outcome))
        }
        None => Ok((contents, outcome)),
    }
}

pub(crate) fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
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
