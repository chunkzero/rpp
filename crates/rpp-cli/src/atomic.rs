//! Atomic replacement of user-maintained files (`rpp.toml`, `plugins.toml`).

use std::path::Path;

use anyhow::{Context, Result};

/// Write `contents` to `path` through a sibling temporary file and rename, so
/// a crash mid-write never leaves a truncated config behind.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::Builder::new()
        .prefix(".rpp-write-")
        .tempfile_in(parent)
        .with_context(|| format!("creating temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut file, contents.as_ref())
        .with_context(|| format!("writing {}", path.display()))?;
    file.persist(path)
        .map(|_| ())
        .with_context(|| format!("replacing {}", path.display()))
}
