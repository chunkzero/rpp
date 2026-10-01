//! Atomic replacement of user-maintained files (`rpp.json`).

use std::path::Path;

use anyhow::{Context, Result};

/// Write `contents` to `path` through a sibling temporary file and rename, so
/// a crash mid-write never leaves a truncated config behind.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut file = tempfile::Builder::new()
        .prefix(".rpp-write-")
        .tempfile_in(parent)
        .with_context(|| format!("creating temporary file in {}", parent.display()))?;
    if let Some(permissions) = permissions {
        file.as_file()
            .set_permissions(permissions)
            .with_context(|| format!("preserving permissions for {}", path.display()))?;
    }
    std::io::Write::write_all(&mut file, contents.as_ref())
        .with_context(|| format!("writing {}", path.display()))?;
    file.persist(path)
        .map(|_| ())
        .with_context(|| format!("replacing {}", path.display()))
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn replacement_preserves_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rpp.json");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

        write(&path, "new").unwrap();

        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
