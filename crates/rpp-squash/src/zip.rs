//! Deterministic release archive creation.

use std::fs;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use rayon::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::error::{Error, Result};
use crate::optimize::{build_glob_set, optimize, Outcome, SquashReport};
use crate::options::SquashOptions;
use crate::walk::{walk_files, WalkedFile};

/// The pack metadata file Minecraft reads eagerly; placed first in the archive.
const PACK_MCMETA: &str = "pack.mcmeta";

/// Fixed unix mode for all regular file entries (`rw-r--r--`).
const FILE_MODE: u32 = 0o644;

const COMPRESSION_LEVEL: i64 = 9;

/// Staging files beside the destination archive are named
/// `<STAGING_PREFIX><STAGING_RANDOM alphanumerics><STAGING_SUFFIX>`.
const STAGING_PREFIX: &str = ".rpp-release-";
const STAGING_RANDOM: usize = 6;
const STAGING_SUFFIX: &str = ".tmp";

/// Limits on the files prepared concurrently per batch; a single larger file forms its
/// own batch. At most two batches are held in memory.
const BATCH_FILES: usize = 256;
const BATCH_BYTES: u64 = 32 * 1024 * 1024;

/// Write a deterministic, optimized release archive of `dir` to `zip_path`,
/// replacing any existing file atomically. `dir` itself is never modified.
///
/// Files matching a `strip` glob are left out of the archive. Every other file
/// is read once and optimized in memory per `opts`; the optimized bytes are
/// archived only when strictly smaller than the original. Files are prepared
/// in parallel and written in archive order.
///
/// The archive is staged in a temporary file beside `zip_path`, flushed, given
/// mode 0644 on unix, and then renamed into place; on failure the previous
/// archive is left untouched and no temporary file remains. When `zip_path` is
/// inside `dir`, it and any staging files beside it are excluded from the archive;
/// paths are compared canonically, so `..` components and symlinks do not matter.
///
/// The archive layout is deterministic, see [`zip_to_vec`]. Recoverable per-file
/// problems (invalid JSON, PNG failures) are reported as warnings in the returned
/// [`SquashReport`]; hard I/O failures abort with an [`Error`].
pub fn squash_zip(dir: &Path, zip_path: &Path, opts: &SquashOptions) -> Result<SquashReport> {
    let strip = build_glob_set(&opts.strip)?;
    let parent = zip_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent
        .canonicalize()
        .map_err(|err| Error::io(parent, err))?;
    let file_name = zip_path.file_name().ok_or_else(|| {
        let err = std::io::Error::from(std::io::ErrorKind::InvalidInput);
        Error::io(zip_path, err)
    })?;
    let dir = dir.canonicalize().map_err(|err| Error::io(dir, err))?;
    let (stripped, kept): (Vec<_>, Vec<_>) = archive_entries(&dir, Some(&parent.join(file_name)))?
        .into_iter()
        .partition(|file| strip.is_match(&file.rel));

    let mut tmp = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .rand_bytes(STAGING_RANDOM)
        .suffix(STAGING_SUFFIX)
        .tempfile_in(&parent)
        .map_err(|err| Error::io(&parent, err))?;

    let (_, mut report) = write_archive(&kept, tmp.as_file_mut(), zip_path, |file| {
        optimize(file, opts)
    })?;
    report.files_stripped = stripped.len();
    tmp.as_file()
        .sync_all()
        .map_err(|err| Error::io(zip_path, err))?;
    set_shareable_permissions(tmp.as_file()).map_err(|err| Error::io(zip_path, err))?;
    tmp.persist(zip_path)
        .map_err(|err| Error::io(zip_path, err.error))?;
    Ok(report)
}

/// Build an unoptimized deterministic zip archive of `dir` in memory.
///
/// Entries are sorted by forward-slash path with `pack.mcmeta` forced first
/// (Minecraft and some launchers read it eagerly), names use forward slashes,
/// the deflate method is used, every entry carries a fixed DOS timestamp
/// (1980-01-01 00:00:00) and identical unix permissions, and no extra fields are
/// written. Two runs over an unchanged tree produce byte-identical archives.
/// Directory entries are omitted. [`squash_zip`] uses the same layout.
pub fn zip_to_vec(dir: &Path) -> Result<Vec<u8>> {
    let dir = std::path::absolute(dir).map_err(|err| Error::io(dir, err))?;
    let entries = archive_entries(&dir, None)?;
    let read = |file: &WalkedFile| {
        let contents = fs::read(&file.abs).map_err(|err| Error::io(&file.abs, err))?;
        Ok((contents, Outcome::default()))
    };
    let (cursor, _) = write_archive(&entries, Cursor::new(Vec::new()), &dir, read)?;
    Ok(cursor.into_inner())
}

/// Regular files under `dir` in archive order, skipping the destination archive
/// `exclude` and the staging files beside it when present.
fn archive_entries(dir: &Path, exclude: Option<&Path>) -> Result<Vec<WalkedFile>> {
    let mut entries = walk_files(dir)?;
    entries.retain(|entry| !exclude.is_some_and(|zip| is_destination_artifact(&entry.abs, zip)));
    entries.sort_by(|a, b| {
        let a_first = a.rel == PACK_MCMETA;
        let b_first = b.rel == PACK_MCMETA;
        b_first.cmp(&a_first).then_with(|| a.rel.cmp(&b.rel))
    });
    Ok(entries)
}

/// Whether `path` is the destination archive or a staging file beside it.
fn is_destination_artifact(path: &Path, zip_path: &Path) -> bool {
    path == zip_path
        || (path.parent() == zip_path.parent()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_staging_name))
}

/// Whether `name` has the exact shape of a staging file name.
fn is_staging_name(name: &str) -> bool {
    name.strip_prefix(STAGING_PREFIX)
        .and_then(|rest| rest.strip_suffix(STAGING_SUFFIX))
        .is_some_and(|random| {
            random.len() == STAGING_RANDOM && random.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

/// Write `entries` as a zip archive into `sink`, archiving the contents `prepare`
/// returns for each. Batches are prepared and deflated in parallel while the
/// previous batch is copied into the archive in order. `label` names the archive
/// in errors.
fn write_archive<W, F>(
    entries: &[WalkedFile],
    sink: W,
    label: &Path,
    prepare: F,
) -> Result<(W, SquashReport)>
where
    W: Write + Seek + Send,
    F: Fn(&WalkedFile) -> Result<(Vec<u8>, Outcome)> + Sync,
{
    let zip_err = |source| Error::Zip {
        path: label.to_path_buf(),
        source,
    };
    let deflate = |file: &WalkedFile| {
        let (contents, outcome) = prepare(file)?;
        let entry = deflate_entry(&file.rel, &contents).map_err(zip_err)?;
        Ok((entry, outcome))
    };
    let prepare_batch =
        |batch: &[WalkedFile]| -> Vec<Result<_>> { batch.par_iter().map(deflate).collect() };

    let mut writer = ZipWriter::new(sink);
    let mut report = SquashReport::default();
    let mut write_batch = |prepared: Vec<Result<(Vec<u8>, Outcome)>>| {
        for prepared in prepared {
            let (entry, outcome) = prepared?;
            report.record(outcome);
            let mut entry = ZipArchive::new(Cursor::new(entry)).map_err(zip_err)?;
            let file = entry.by_index_raw(0).map_err(zip_err)?;
            writer.raw_copy_file(file).map_err(zip_err)?;
        }
        Ok::<_, Error>(())
    };

    let mut batches = batches(entries, BATCH_FILES, BATCH_BYTES).into_iter();
    let mut pending = batches.next().map(prepare_batch);
    while let Some(prepared) = pending {
        let next = batches.next();
        let (written, next) = rayon::join(|| write_batch(prepared), || next.map(prepare_batch));
        written?;
        pending = next;
    }
    let sink = writer.finish().map_err(zip_err)?;
    Ok((sink, report))
}

/// Deflate one entry into a single-entry archive, so it can be compressed off
/// the writer thread and raw-copied into the release archive unchanged.
fn deflate_entry(name: &str, contents: &[u8]) -> zip::result::ZipResult<Vec<u8>> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(COMPRESSION_LEVEL))
        .last_modified_time(DateTime::default())
        .unix_permissions(FILE_MODE)
        .large_file(false);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file(name, options)?;
    writer.write_all(contents)?;
    Ok(writer.finish()?.into_inner())
}

/// Split `entries` into consecutive batches of at most `max_files` files and
/// `max_bytes` bytes, except that a file larger than `max_bytes` forms its own batch.
fn batches(entries: &[WalkedFile], max_files: usize, max_bytes: u64) -> Vec<&[WalkedFile]> {
    let mut batches = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (index, entry) in entries.iter().enumerate() {
        if index > start && (index - start == max_files || bytes + entry.len > max_bytes) {
            batches.push(&entries[start..index]);
            (start, bytes) = (index, 0);
        }
        bytes += entry.len;
    }
    if start < entries.len() {
        batches.push(&entries[start..]);
    }
    batches
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
    use std::path::PathBuf;

    use super::*;
    use crate::file::squash_file;
    use crate::options::PngLevel;

    fn write_zip(dir: &Path, zip_path: &Path) -> Result<()> {
        squash_zip(dir, zip_path, &SquashOptions::default()).map(drop)
    }

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

    #[cfg(unix)]
    #[test]
    fn unreadable_input_preserves_archive_and_cleans_staging() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let input = pack_dir();
        let path = dir.path().join("pack.zip");
        write_zip(input.path(), &path).unwrap();
        let previous = fs::read(&path).unwrap();
        let locked = input.path().join("locked.json");
        fs::write(&locked, "{}").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&locked).is_ok() {
            // Running with privileges that bypass file modes.
            return;
        }

        assert!(write_zip(input.path(), &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["pack.zip"]);
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

    #[test]
    fn staging_files_beside_destination_are_excluded() {
        let input = pack_dir();
        let path = input.path().join("pack.zip");
        fs::write(input.path().join(".rpp-release-a1B2c3.tmp"), "in flight").unwrap();
        fs::write(input.path().join(".rpp-release-notes.txt"), "ordinary").unwrap();
        let sub = input.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join(".rpp-release-a1B2c3.tmp"), "ordinary").unwrap();
        write_zip(input.path(), &path).unwrap();
        let archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        let mut names: Vec<_> = archive.file_names().collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                ".rpp-release-notes.txt",
                "pack.mcmeta",
                "sub/.rpp-release-a1B2c3.tmp"
            ]
        );
    }

    #[test]
    fn dotdot_alias_of_destination_excludes_staging_files() {
        let input = pack_dir();
        fs::create_dir(input.path().join("sub")).unwrap();
        fs::write(input.path().join(".rpp-release-a1B2c3.tmp"), "in flight").unwrap();
        let path = input.path().join("sub/../pack.zip");
        write_zip(input.path(), &path).unwrap();
        let archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(archive.file_names().collect::<Vec<_>>(), ["pack.mcmeta"]);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_destination_parent_excludes_staging_files() {
        let input = pack_dir();
        fs::write(input.path().join(".rpp-release-a1B2c3.tmp"), "in flight").unwrap();
        let links = tempfile::tempdir().unwrap();
        let alias = links.path().join("alias");
        std::os::unix::fs::symlink(input.path(), &alias).unwrap();
        let path = alias.join("pack.zip");
        write_zip(input.path(), &path).unwrap();
        let archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(archive.file_names().collect::<Vec<_>>(), ["pack.mcmeta"]);
    }

    /// A 16x16 RGBA PNG that oxipng can shrink.
    fn make_png() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut encoder = png::Encoder::new(&mut buf, 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().unwrap();
        let data: Vec<u8> = (0..16 * 16 * 4).map(|i| (i % 7 * 30) as u8).collect();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
        buf
    }

    /// The archive the staged pipeline produced: strip, optimize each file, then
    /// deflate every entry in order through one sequential `ZipWriter`.
    fn reference_archive(dir: &Path, opts: &SquashOptions) -> Vec<u8> {
        let strip = build_glob_set(&opts.strip).unwrap();
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(9))
            .last_modified_time(DateTime::default())
            .unix_permissions(0o644)
            .large_file(false);
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for entry in archive_entries(dir, None).unwrap() {
            if entry.rel == "pack.zip" || strip.is_match(&entry.rel) {
                continue;
            }
            let original = fs::read(&entry.abs).unwrap();
            let contents = squash_file(&entry.rel, &original, opts, &mut Vec::new());
            writer.start_file(&entry.rel, options).unwrap();
            writer
                .write_all(contents.as_deref().unwrap_or(&original))
                .unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn release_archive_matches_staged_pipeline() {
        let input = tempfile::tempdir().unwrap();
        let root = input.path();
        fs::create_dir_all(root.join("assets/minecraft/textures")).unwrap();
        fs::write(
            root.join("pack.mcmeta"),
            "{\n  \"pack\": {\"pack_format\": 84}\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("assets/minecraft/a.json"),
            "{\n  \"x\": [1, 2]\n}\n",
        )
        .unwrap();
        fs::write(root.join("assets/minecraft/broken.json"), "{ nope").unwrap();
        fs::write(root.join("assets/minecraft/textures/t.png"), make_png()).unwrap();
        fs::write(root.join("assets/minecraft/notes.txt"), "text ".repeat(200)).unwrap();
        fs::write(root.join("assets/minecraft/art.psd"), "layers").unwrap();
        fs::write(root.join(".rpp-release-notes.txt"), "notes").unwrap();
        let path = root.join("pack.zip");

        for png in [PngLevel::Off, PngLevel::Fast] {
            let opts = SquashOptions {
                png,
                strip: vec!["**/*.psd".into()],
                ..SquashOptions::default()
            };
            let report = squash_zip(root, &path, &opts).unwrap();
            assert_eq!(fs::read(&path).unwrap(), reference_archive(root, &opts));
            assert_eq!(report.files_stripped, 1);
            assert_eq!(
                report.files_optimized,
                if png == PngLevel::Off { 2 } else { 3 }
            );
            assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        }
    }

    #[test]
    fn batches_are_bounded_by_files_and_bytes() {
        let entries: Vec<_> = [5, 5, 5, 20, 1, 1, 1, 1, 1]
            .into_iter()
            .map(|len| WalkedFile {
                abs: PathBuf::new(),
                rel: String::new(),
                len,
            })
            .collect();
        let lens: Vec<Vec<u64>> = batches(&entries, 3, 10)
            .into_iter()
            .map(|batch| batch.iter().map(|entry| entry.len).collect())
            .collect();
        assert_eq!(
            lens,
            [vec![5, 5], vec![5], vec![20], vec![1, 1, 1], vec![1, 1]]
        );
    }
}
