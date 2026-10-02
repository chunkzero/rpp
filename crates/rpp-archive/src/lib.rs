//! The rpp plugin archive format (`.rpp.tgz`): a gzipped tar of files at the archive
//! root.
//!
//! [`pack`] writes deterministic archives and [`unpack`] extracts them safely. Both
//! enforce the same limits: at most 20,000 entries, 64 MiB per file and 512 MiB in total.

#![deny(missing_docs)]

mod error;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

pub use error::{Error, Result};

#[derive(Debug, Clone, Copy)]
struct Limits {
    entries: usize,
    file_bytes: u64,
    total_bytes: u64,
}

const LIMITS: Limits = Limits {
    entries: 20_000,
    file_bytes: 64 * 1024 * 1024,
    total_bytes: 512 * 1024 * 1024,
};

/// Entries and bytes seen so far, checked against [`Limits`].
struct Tally {
    limits: Limits,
    entries: usize,
    bytes: u64,
}

impl Tally {
    fn new(limits: Limits) -> Self {
        Tally {
            limits,
            entries: 0,
            bytes: 0,
        }
    }

    fn entry(&mut self) -> Result<()> {
        self.entries += 1;
        if self.entries > self.limits.entries {
            return Err(Error::TooManyEntries {
                max: self.limits.entries,
            });
        }
        Ok(())
    }

    fn file(&mut self, path: &str, size: u64) -> Result<()> {
        if size > self.limits.file_bytes {
            return Err(Error::FileTooLarge {
                path: path.to_string(),
                size,
                max: self.limits.file_bytes,
            });
        }
        self.bytes = self.bytes.saturating_add(size);
        if self.bytes > self.limits.total_bytes {
            return Err(Error::TooLarge {
                max: self.limits.total_bytes,
            });
        }
        Ok(())
    }
}

/// Pack `files`, keyed by `/`-separated relative path, into an archive.
///
/// Entries are sorted by path with mode 0644, mtime 0 and owner 0, so equal inputs
/// give byte-identical archives.
///
/// # Errors
///
/// [`Error::UnsafePath`] for a path that is not portable: it must be relative and
/// `/`-separated, with no empty, `.` or `..` components, and made only of printable ASCII
/// other than `< > : " \ | ? *`. No component may end in `.` or a space, or have a stem
/// that is a reserved Windows device name such as `CON` or `LPT1`. A limit error is
/// returned when the files exceed what [`unpack`] accepts.
pub fn pack(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    pack_with(files, LIMITS)
}

fn pack_with(files: &BTreeMap<String, Vec<u8>>, limits: Limits) -> Result<Vec<u8>> {
    let mut tally = Tally::new(limits);
    let mut builder = tar::Builder::new(Vec::new());
    for (path, contents) in files {
        if !is_portable(path) {
            return Err(Error::UnsafePath(path.clone()));
        }
        tally.entry()?;
        tally.file(path, contents.len() as u64)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder
            .append_data(&mut header, path, contents.as_slice())
            .map_err(Error::io(format!("adding `{path}` to the archive")))?;
    }
    let tar = builder
        .into_inner()
        .map_err(Error::io("writing the archive"))?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&tar)
        .map_err(Error::io("compressing the archive"))?;
    encoder
        .finish()
        .map_err(Error::io("compressing the archive"))
}

/// Unpack the archive `bytes` into `dest`, creating it if needed.
///
/// Only regular files and directories are extracted, and only beneath `dest`. Symlinks
/// already inside `dest` are followed, so unpack into a fresh directory. On error,
/// `dest` may hold a partial extraction.
///
/// # Errors
///
/// [`Error::UnsafePath`] for an absolute or escaping path, [`Error::UnsupportedEntry`]
/// for links and other special entries, a limit error, or [`Error::Io`].
pub fn unpack(bytes: &[u8], dest: &Path) -> Result<()> {
    unpack_with(bytes, dest, LIMITS)
}

fn unpack_with(bytes: &[u8], dest: &Path, limits: Limits) -> Result<()> {
    fs::create_dir_all(dest).map_err(Error::io(format!("creating {}", dest.display())))?;
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut tally = Tally::new(limits);
    let entries = archive
        .entries()
        .map_err(Error::io("reading the archive"))?;
    for entry in entries {
        tally.entry()?;
        let entry = entry.map_err(Error::io("reading the archive"))?;
        extract(entry, dest, &mut tally)?;
    }
    Ok(())
}

fn extract(mut entry: tar::Entry<'_, impl Read>, dest: &Path, tally: &mut Tally) -> Result<()> {
    let raw = entry
        .path()
        .map_err(Error::io("reading an archive path"))?
        .into_owned();
    let name = raw.to_string_lossy().into_owned();
    let relative = relative(&raw).ok_or_else(|| Error::UnsafePath(name.clone()))?;
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    let out = dest.join(&relative);
    let kind = entry.header().entry_type();
    if kind.is_dir() {
        return fs::create_dir_all(&out).map_err(Error::io(format!("creating {}", out.display())));
    }
    if !kind.is_file() {
        return Err(Error::UnsupportedEntry(name));
    }
    tally.file(&name, entry.size())?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(Error::io(format!("creating {}", parent.display())))?;
    }
    let mut file =
        fs::File::create(&out).map_err(Error::io(format!("creating {}", out.display())))?;
    std::io::copy(&mut entry, &mut file).map_err(Error::io(format!("extracting `{name}`")))?;
    Ok(())
}

/// Printable ASCII characters Windows rejects in file names.
const WINDOWS_INVALID_CHARS: [char; 8] = ['<', '>', ':', '"', '\\', '|', '?', '*'];

/// Device names Windows reserves regardless of extension or case.
const WINDOWS_RESERVED_NAMES: [&str; 24] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

/// Whether `path` extracts to the same place on every supported platform.
fn is_portable(path: &str) -> bool {
    path.split('/').all(|component| {
        let stem = component
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ');
        !matches!(component, "" | "." | "..")
            && component
                .chars()
                .all(|c| matches!(c, ' '..='~') && !WINDOWS_INVALID_CHARS.contains(&c))
            && !component.ends_with(['.', ' '])
            && !WINDOWS_RESERVED_NAMES
                .iter()
                .any(|name| stem.eq_ignore_ascii_case(name))
    })
}

/// `path` without `.` components, or `None` if it is absolute or has `..` components.
fn relative(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(entries: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(path, contents)| (path.to_string(), contents.to_vec()))
            .collect()
    }

    fn limits(entries: usize, file_bytes: u64, total_bytes: u64) -> Limits {
        Limits {
            entries,
            file_bytes,
            total_bytes,
        }
    }

    /// A gzipped tar holding one entry whose raw name and type bypass `tar`'s checks.
    fn raw_archive(name: &str, kind: tar::EntryType, contents: &[u8]) -> Vec<u8> {
        let mut header = tar::Header::new_old();
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        header.set_entry_type(kind);
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        let mut builder = tar::Builder::new(Vec::new());
        builder.append(&header, contents).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&builder.into_inner().unwrap()).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn round_trips_deterministically() {
        let input = files(&[("rpp.json", b"{}"), ("dist/plugin.js", b"export {};\n")]);
        let archive = pack(&input).unwrap();
        assert_eq!(archive, pack(&input).unwrap());

        let dir = tempfile::tempdir().unwrap();
        unpack(&archive, dir.path()).unwrap();
        for (path, contents) in &input {
            assert_eq!(&fs::read(dir.path().join(path)).unwrap(), contents);
        }
    }

    #[test]
    fn limits_apply_to_pack_and_unpack() {
        let input = files(&[("a.js", &[0; 6]), ("b.js", &[0; 6])]);
        assert!(pack_with(&input, limits(2, 6, 12)).is_ok());

        let per_file = pack_with(&input, limits(2, 5, 100)).unwrap_err();
        assert!(
            per_file.to_string().contains("`a.js` is 6 bytes"),
            "{per_file}"
        );
        let total = pack_with(&input, limits(2, 6, 11)).unwrap_err();
        assert!(matches!(total, Error::TooLarge { max: 11 }), "{total}");
        let count = pack_with(&input, limits(1, 6, 12)).unwrap_err();
        assert!(matches!(count, Error::TooManyEntries { max: 1 }), "{count}");

        let archive = pack(&input).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let unpacked = unpack_with(&archive, dir.path(), limits(2, 6, 11)).unwrap_err();
        assert!(matches!(unpacked, Error::TooLarge { .. }), "{unpacked}");
    }

    #[test]
    fn pack_rejects_non_portable_paths() {
        for path in [
            "../escape",
            "/abs",
            "a/./b",
            "a//b",
            "",
            "..\\escape",
            "a\\b",
            "C:\\escape",
            "C:/escape",
            "a/C:",
            "tool?.wasm",
            "tool*.wasm",
            "a|b",
            "con.txt",
            "CON .wasm",
            "nul .txt",
            "CONIN$",
            "conout$.log",
            "dir/LPT1",
            "trailing.",
            "trailing ",
            "COM¹.wasm",
            "LPT².wasm",
            "conın$.wasm",
            "é.js",
            &format!("{}\0", "a".repeat(101)),
        ] {
            let result = pack(&files(&[(path, b"")]));
            assert!(matches!(result, Err(Error::UnsafePath(_))), "{path}");
        }
    }

    #[test]
    fn pack_accepts_ordinary_paths() {
        let input = files(&[
            ("dist/plugin.js", b"a"),
            ("components/math.wasm", b"b"),
            ("a.b.c", b"c"),
        ]);
        assert!(pack(&input).is_ok());
    }

    #[test]
    fn unpack_rejects_escaping_paths_and_links() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        for name in ["../escape", "/etc/escape"] {
            let archive = raw_archive(name, tar::EntryType::Regular, b"x");
            let result = unpack(&archive, &dest);
            assert!(matches!(result, Err(Error::UnsafePath(_))), "{name}");
        }
        let link = raw_archive("link", tar::EntryType::Symlink, b"");
        assert!(matches!(
            unpack(&link, &dest),
            Err(Error::UnsupportedEntry(_))
        ));
        assert!(!dir.path().join("escape").exists());
    }

    #[test]
    fn relative_drops_current_dir_components() {
        assert_eq!(relative(Path::new("./a/./b")), Some(PathBuf::from("a/b")));
        assert_eq!(relative(Path::new(".")), Some(PathBuf::new()));
        assert_eq!(relative(Path::new("a/../b")), None);
    }
}
