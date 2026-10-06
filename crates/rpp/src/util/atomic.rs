//! Same-filesystem atomic file publication helpers.

use std::io::Write;
use std::path::Path;

/// Write `contents` into a sibling temporary file, flush it to stable storage, atomically
/// publish it at `destination`, and flush the parent directory so the rename survives a power
/// failure. Windows cannot flush directories, so there only the contents are flushed.
pub(crate) fn write(destination: &Path, contents: &[u8]) -> std::io::Result<()> {
    publish(destination, contents, true)
}

/// Like [`write`], but without flushing contents to stable storage before publication. Concurrent
/// readers still never observe a partial file, but after a power failure the destination may be
/// missing, empty, or partial, so callers must only use it for regenerable data they verify on read.
pub(crate) fn write_unsynced(destination: &Path, contents: &[u8]) -> std::io::Result<()> {
    publish(destination, contents, false)
}

fn publish(destination: &Path, contents: &[u8], sync: bool) -> std::io::Result<()> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let mut temporary = staging_file(parent)?;
    temporary.write_all(contents)?;
    if sync {
        temporary.as_file_mut().sync_all()?;
    }
    temporary
        .persist(destination)
        .map_err(|error| error.error)?;
    if sync {
        sync_dir(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// A temporary file in `parent` created with the mode a plain `File::create` would get (0o666 minus the
/// umask), rather than tempfile's private 0o600.
pub(crate) fn staging_file(parent: &Path) -> std::io::Result<tempfile::NamedTempFile> {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    builder.tempfile_in(parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_replaces_existing_contents() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("manifest.bin");
        std::fs::write(&destination, b"old").unwrap();

        write(&destination, b"new").unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn published_files_get_the_default_mode() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let reference = directory.path().join("reference");
        std::fs::write(&reference, b"").unwrap();
        let expected = std::fs::metadata(&reference).unwrap().permissions().mode() & 0o777;

        let written = directory.path().join("written");
        let unsynced = directory.path().join("unsynced");
        write(&written, b"new").unwrap();
        write_unsynced(&unsynced, b"new").unwrap();

        for path in [written, unsynced] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, expected, "{}", path.display());
        }
    }
}
