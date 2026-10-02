//! Deterministic zip archive creation.

use std::fs;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::error::{Error, Result};
use crate::walk::{walk_files, WalkedFile};

/// The pack metadata file Minecraft reads eagerly; placed first in the archive.
const PACK_MCMETA: &str = "pack.mcmeta";

/// Fixed unix mode for all regular file entries (`rw-r--r--`).
const FILE_MODE: u32 = 0o644;

const COMPRESSION_LEVEL: i64 = 9;

/// Write a deterministic zip archive of `dir` to `zip_path`, replacing any
/// existing file atomically.
///
/// The archive is staged in a temporary file beside `zip_path`, flushed, given
/// mode 0644 on unix, and then renamed into place; on failure the previous
/// archive is left untouched and no temporary file remains. When `zip_path` is
/// inside `dir` it is excluded from the archive.
///
/// Archive bytes are deterministic, see [`zip_to_vec`].
pub fn write_zip(dir: &Path, zip_path: &Path) -> Result<()> {
    let dir = std::path::absolute(dir).map_err(|err| Error::io(dir, err))?;
    let zip_abs = std::path::absolute(zip_path).map_err(|err| Error::io(zip_path, err))?;
    let entries = archive_entries(&dir, Some(&zip_abs))?;

    let parent = zip_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".rpp-release-")
        .tempfile_in(parent)
        .map_err(|err| Error::io(parent, err))?;

    write_archive(&entries, tmp.as_file_mut(), zip_path)?;
    tmp.as_file()
        .sync_all()
        .map_err(|err| Error::io(zip_path, err))?;
    set_shareable_permissions(tmp.as_file()).map_err(|err| Error::io(zip_path, err))?;
    tmp.persist(zip_path)
        .map_err(|err| Error::io(zip_path, err.error))?;
    Ok(())
}

/// Build the same deterministic zip archive as [`write_zip`] in memory.
///
/// Entries are sorted by forward-slash path with `pack.mcmeta` forced first
/// (Minecraft and some launchers read it eagerly), names use forward slashes,
/// the deflate method is used, every entry carries a fixed DOS timestamp
/// (1980-01-01 00:00:00) and identical unix permissions, and no extra fields are
/// written. Two runs over an unchanged tree produce byte-identical archives.
/// Directory entries are omitted.
pub fn zip_to_vec(dir: &Path) -> Result<Vec<u8>> {
    let dir = std::path::absolute(dir).map_err(|err| Error::io(dir, err))?;
    let entries = archive_entries(&dir, None)?;
    let cursor = write_archive(&entries, Cursor::new(Vec::new()), &dir)?;
    Ok(cursor.into_inner())
}

/// Regular files under `dir` in archive order, skipping `exclude` when present.
fn archive_entries(dir: &Path, exclude: Option<&Path>) -> Result<Vec<WalkedFile>> {
    let mut entries = walk_files(dir)?;
    entries.retain(|entry| Some(entry.abs.as_path()) != exclude);
    entries.sort_by(|a, b| {
        let a_first = a.rel == PACK_MCMETA;
        let b_first = b.rel == PACK_MCMETA;
        b_first.cmp(&a_first).then_with(|| a.rel.cmp(&b.rel))
    });
    Ok(entries)
}

/// Write `entries` as a zip archive into `sink`. `label` names the archive in errors.
fn write_archive<W: Write + Seek>(entries: &[WalkedFile], sink: W, label: &Path) -> Result<W> {
    let zip_err = |source| Error::Zip {
        path: label.to_path_buf(),
        source,
    };
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(COMPRESSION_LEVEL))
        .last_modified_time(DateTime::default())
        .unix_permissions(FILE_MODE)
        .large_file(false);

    let mut writer = ZipWriter::new(sink);
    for entry in entries {
        let contents = fs::read(&entry.abs).map_err(|err| Error::io(&entry.abs, err))?;
        writer.start_file(&entry.rel, options).map_err(zip_err)?;
        writer
            .write_all(&contents)
            .map_err(|err| Error::io(label, err))?;
    }
    writer.finish().map_err(zip_err)
}

/// Temporary files are private by default; the archive is a shareable artifact.
#[cfg(unix)]
fn set_shareable_permissions(file: &fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(FILE_MODE))
}

#[cfg(not(unix))]
fn set_shareable_permissions(_file: &fs::File) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_dir() -> tempfile::TempDir {
        let input = tempfile::tempdir().unwrap();
        fs::write(input.path().join("pack.mcmeta"), "{}").unwrap();
        input
    }

    #[test]
    fn failed_write_preserves_archive_and_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let input = pack_dir();
        let path = dir.path().join("pack.zip");
        write_zip(input.path(), &path).unwrap();
        let previous = fs::read(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o644
            );
        }

        assert!(write_zip(&input.path().join("missing"), &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_persist_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let input = tempfile::tempdir().unwrap();
        let path = dir.path().join("pack.zip");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "previous").unwrap();

        assert!(write_zip(input.path(), &path).is_err());
        assert_eq!(fs::read_to_string(path.join("keep")).unwrap(), "previous");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_writes_use_independent_staging() {
        let dir = tempfile::tempdir().unwrap();
        let input = pack_dir();
        let path = dir.path().join("pack.zip");
        let fixed_temp = dir.path().join(".pack.zip.tmp");
        fs::write(&fixed_temp, "unrelated").unwrap();
        write_zip(input.path(), &path).unwrap();
        let previous = fs::read(&path).unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let one = scope.spawn(|| {
                barrier.wait();
                write_zip(input.path(), &path).unwrap();
            });
            barrier.wait();
            write_zip(input.path(), &path).unwrap();
            one.join().unwrap();
        });
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(fs::read_to_string(fixed_temp).unwrap(), "unrelated");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn in_memory_archive_matches_written_archive() {
        let input = pack_dir();
        fs::create_dir(input.path().join("assets")).unwrap();
        fs::write(input.path().join("assets/a.json"), "{\"a\":1}").unwrap();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("pack.zip");
        write_zip(input.path(), &path).unwrap();
        assert_eq!(zip_to_vec(input.path()).unwrap(), fs::read(path).unwrap());
    }

    #[test]
    fn archive_inside_source_dir_is_excluded() {
        let input = pack_dir();
        let path = input.path().join("pack.zip");
        write_zip(input.path(), &path).unwrap();
        write_zip(input.path(), &path).unwrap();
        let archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(archive.file_names().collect::<Vec<_>>(), ["pack.mcmeta"]);
    }
}
