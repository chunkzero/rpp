//! Same-filesystem atomic file publication helpers.

use std::io::Write;
use std::path::Path;

/// Copy `source` into a sibling temporary file and atomically publish it at
/// `destination`, replacing an existing file without an observable missing
/// destination between operations.
pub(crate) fn copy(source: &Path, destination: &Path) -> std::io::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let mut source_file = std::fs::File::open(source)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut source_file, temporary.as_file_mut())?;
    temporary.as_file_mut().sync_all()?;

    if let Ok(metadata) = source_file.metadata() {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary
        .persist(destination)
        .map(|_| ())
        .map_err(|error| error.error)
}

/// Write `contents` into a sibling temporary file and atomically publish it at
/// `destination`.
pub(crate) fn write(destination: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(contents)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(destination)
        .map(|_| ())
        .map_err(|error| error.error)
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
}
