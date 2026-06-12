//! Deterministic zip archive creation.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::dir::rel_path;
use crate::error::{Error, Result};
use crate::options::ZipOptions;

/// The pack metadata file Minecraft reads eagerly; placed first in the archive.
const PACK_MCMETA: &str = "pack.mcmeta";

/// Fixed unix mode for all regular file entries (`rw-r--r--`).
const FILE_MODE: u32 = 0o644;

/// Write a deterministic zip archive of `dir` to `zip_path`.
///
/// Determinism guarantees: entries are sorted by forward-slash path with
/// `pack.mcmeta` forced first (Minecraft and some launchers read it eagerly),
/// names use forward slashes, the deflate method is used, every entry carries a
/// fixed DOS timestamp (1980-01-01 00:00:00) and identical unix permissions, and
/// no extra fields are written. Two runs over an unchanged tree produce
/// byte-identical archives. Directory entries are omitted.
pub fn write_zip(dir: &Path, zip_path: &Path, opts: &ZipOptions) -> Result<()> {
    // Collect all regular files as (archive-name, absolute-path).
    let mut entries: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in WalkDir::new(dir).follow_links(false) {
        let entry = entry.map_err(|err| {
            let path = err
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| dir.to_path_buf());
            Error::io(path, err.into())
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path().to_path_buf();
        let name = rel_path(dir, &abs);
        entries.push((name, abs));
    }

    // Deterministic order: pack.mcmeta first, then lexicographic by name.
    entries.sort_by(|a, b| {
        let a_first = a.0 == PACK_MCMETA;
        let b_first = b.0 == PACK_MCMETA;
        b_first.cmp(&a_first).then_with(|| a.0.cmp(&b.0))
    });

    let file = File::create(zip_path).map_err(|err| Error::io(zip_path, err))?;
    let mut writer = ZipWriter::new(file);

    let mut file_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(FILE_MODE)
        .large_file(false);
    if let Some(level) = opts.compression_level {
        file_options = file_options.compression_level(Some(level));
    }

    for (name, abs) in &entries {
        let contents = fs::read(abs).map_err(|err| Error::io(abs, err))?;
        writer
            .start_file(name, file_options)
            .map_err(|source| Error::Zip {
                path: zip_path.to_path_buf(),
                source,
            })?;
        writer
            .write_all(&contents)
            .map_err(|err| Error::io(zip_path, err))?;
    }

    if !opts.comment.is_empty() {
        writer
            .set_comment(opts.comment.clone())
            .map_err(|source| Error::Zip {
                path: zip_path.to_path_buf(),
                source,
            })?;
    }

    writer.finish().map_err(|source| Error::Zip {
        path: zip_path.to_path_buf(),
        source,
    })?;

    Ok(())
}
