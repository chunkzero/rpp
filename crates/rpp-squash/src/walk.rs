//! Regular-file discovery for archive creation.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{Error, Result};

/// A regular file found under a walked root.
pub(crate) struct WalkedFile {
    pub(crate) abs: PathBuf,
    /// Forward-slash path relative to the walked root.
    pub(crate) rel: String,
    /// Size in bytes when walked.
    pub(crate) len: u64,
}

/// Collect every regular file under `root` (symlinks are not followed), sorted
/// by relative path.
pub(crate) fn walk_files(root: &Path) -> Result<Vec<WalkedFile>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|err| {
            let path = err.path().unwrap_or(root).to_path_buf();
            Error::io(path, err.into())
        })?;
        if entry.file_type().is_file() {
            let len = entry
                .metadata()
                .map_err(|err| Error::io(entry.path(), err.into()))?
                .len();
            let abs = entry.into_path();
            let rel = relative_forward_slash(root, &abs);
            files.push(WalkedFile { abs, rel, len });
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(files)
}

fn relative_forward_slash(root: &Path, abs: &Path) -> String {
    let rel = abs.strip_prefix(root).unwrap_or(abs);
    let parts: Vec<_> = rel
        .components()
        .map(|comp| comp.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}
