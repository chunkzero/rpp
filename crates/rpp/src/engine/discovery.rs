//! Source discovery and fingerprinting (spec §7).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;

use crate::cache::Fingerprint;
use crate::error::{Error, Result};
use crate::util::hash::xxh3;
use crate::util::path::to_forward_slash;

/// A discovered source file.
pub(crate) struct SourceFile {
    /// Relative, forward-slash path under the source root.
    pub(crate) rel: String,
    /// Absolute path on disk.
    pub(crate) abs: PathBuf,
    /// File size in bytes.
    pub(crate) size: u64,
    /// Modification time in nanoseconds since the epoch.
    pub(crate) mtime_ns: u128,
}

impl SourceFile {
    /// Compute the full fingerprint (hashing the contents).
    pub(crate) fn fingerprint(&self) -> Result<(Fingerprint, Vec<u8>)> {
        let contents = std::fs::read(&self.abs).map_err(|e| Error::io(&self.abs, e))?;
        let fp = Fingerprint {
            mtime_ns: self.mtime_ns,
            size: self.size,
            xxh3: xxh3(&contents),
        };
        Ok((fp, contents))
    }

    /// Whether `prev` matches this file by the fast path (mtime + size).
    pub(crate) fn matches_fast(&self, prev: &Fingerprint) -> bool {
        prev.size == self.size && prev.mtime_ns == self.mtime_ns
    }
}

/// Walk `source`, honoring `.rppignore` and the standard ignore files.
pub(crate) fn discover(source: &Path) -> Result<Vec<SourceFile>> {
    if !source.exists() {
        return Err(Error::Build(format!(
            "source directory `{}` does not exist",
            source.display()
        )));
    }

    let mut files = Vec::new();
    let walker = WalkBuilder::new(source)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .add_custom_ignore_filename(".rppignore")
        .build();

    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let rel_path = path.strip_prefix(source).unwrap_or(path);
        let rel = to_forward_slash(rel_path);
        if rel.is_empty() {
            continue;
        }
        let meta = std::fs::metadata(path).map_err(|e| Error::io(path, e))?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        files.push(SourceFile {
            rel,
            abs: path.to_path_buf(),
            size: meta.len(),
            mtime_ns,
        });
    }

    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(files)
}
