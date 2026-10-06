//! Same-filesystem atomic file publication helpers.

use std::io::Write;
use std::path::Path;

/// Copy `source` into a sibling temporary file and atomically publish it at
/// `destination`, replacing an existing file without an observable missing
/// destination between operations. The destination gets the default file mode, not the source's.
pub(crate) fn copy(source: &Path, destination: &Path) -> std::io::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let mut source_file = std::fs::File::open(source)?;
    let mut temporary = staging_file(parent)?;
    std::io::copy(&mut source_file, temporary.as_file_mut())?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(destination)
        .map(|_| ())
        .map_err(|error| error.error)
}

/// Write `contents` into a sibling temporary file and atomically publish it at
/// `destination`.
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
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let mut temporary = staging_file(parent)?;
    temporary.write_all(contents)?;
    if sync {
        temporary.as_file_mut().sync_all()?;
    }
    temporary
        .persist(destination)
        .map(|_| ())
        .map_err(|error| error.error)
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
    fn copy_replaces_existing_contents() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("nested/destination");
        std::fs::write(&source, b"new").unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&destination, b"old").unwrap();

        copy(&source, &destination).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"new");
    }

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

        let source = directory.path().join("source");
        std::fs::write(&source, b"cached").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
        let written = directory.path().join("written");
        let unsynced = directory.path().join("unsynced");
        let copied = directory.path().join("copied");
        write(&written, b"new").unwrap();
        write_unsynced(&unsynced, b"new").unwrap();
        copy(&source, &copied).unwrap();

        for path in [written, unsynced, copied] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, expected, "{}", path.display());
        }
    }
}
