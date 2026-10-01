//! Safe extraction of plugin archives into the plugin cache.
//!
//! We optionally strip a single top-level wrapper directory, reject any entry that would escape
//! the destination (path traversal / absolute paths / symlinks pointing out),
//! and extract atomically: contents land in a sibling temp directory that is
//! renamed into place only once extraction fully succeeds.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::read::GzDecoder;
use tar::Archive;

use crate::error::{Error, Result};

/// The most entries an installed archive may have.
pub const MAX_ENTRIES: usize = 20_000;
/// The largest file an installed archive may contain, in bytes.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// The most bytes an installed archive may unpack to.
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// Extract a gzipped tarball (`bytes`) into `dest`, stripping the single top-level
/// directory when `strip_wrapper` is set. Extraction is atomic: if `dest` already
/// exists nothing is done, otherwise the archive is unpacked into a sibling temp
/// directory and renamed into place. `validate` runs on the unpacked temp directory before
/// it is renamed into place; if it fails nothing is published.
pub(crate) fn extract_archive(
    bytes: &[u8],
    dest: &Path,
    strip_wrapper: bool,
    validate: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }

    let parent = dest.parent().ok_or_else(|| {
        Error::io(
            "computing cache parent directory",
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "destination has no parent",
            ),
        )
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|e| Error::io(format!("creating cache dir {}", parent.display()), e))?;

    // Unique temp dir alongside the destination so the final rename is atomic on
    // the same filesystem.
    let tmp = unique_temp_dir(parent)?;

    // Clean up the temp dir on any failure.
    let result = unpack_into(bytes, &tmp, strip_wrapper).and_then(|()| validate(&tmp));
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    // Atomic publish. If another process won the race, our temp dir is redundant.
    match std::fs::rename(&tmp, dest) {
        Ok(()) => Ok(()),
        Err(_) if dest.exists() => {
            let _ = std::fs::remove_dir_all(&tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(Error::io(
                format!("publishing cache entry {}", dest.display()),
                e,
            ))
        }
    }
}

/// Unpack `bytes` into `root`, stripping the leading top-level directory when
/// `strip_wrapper` is set.
fn unpack_into(bytes: &[u8], root: &Path, strip_wrapper: bool) -> Result<()> {
    std::fs::create_dir_all(root)
        .map_err(|e| Error::io(format!("creating temp dir {}", root.display()), e))?;

    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    archive.set_preserve_permissions(false);
    archive.set_overwrite(true);

    let entries = archive
        .entries()
        .map_err(|e| Error::io("reading tar entries", e))?;

    let mut entry_count = 0usize;
    let mut total_bytes = 0u64;
    for entry in entries {
        entry_count += 1;
        if entry_count > MAX_ENTRIES {
            return Err(Error::UnsafeTarEntry("archive has too many entries".into()));
        }
        let mut entry = entry.map_err(|e| Error::io("reading tar entry", e))?;
        let raw_path = entry
            .path()
            .map_err(|e| Error::io("reading tar entry path", e))?
            .into_owned();

        let stripped = if strip_wrapper {
            strip_top_level(&raw_path)
        } else {
            Some(raw_path.clone())
        };
        let Some(stripped) = stripped.filter(|p| !is_root(p)) else {
            // The top-level directory entry itself, or an empty path: skip.
            continue;
        };

        // Validate the stripped path: only normal components allowed.
        let safe = sanitize(&stripped)
            .ok_or_else(|| Error::UnsafeTarEntry(raw_path.to_string_lossy().into_owned()))?;

        let out_path = root.join(&safe);

        // Refuse symlinks/hardlinks entirely: their targets could escape the root
        // and they are not needed for plugin packages.
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(Error::UnsafeTarEntry(
                raw_path.to_string_lossy().into_owned(),
            ));
        }

        if entry_type.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| Error::io(format!("creating {}", out_path.display()), e))?;
            continue;
        }

        let size = entry.size();
        total_bytes = total_bytes.saturating_add(size);
        if size > MAX_FILE_BYTES || total_bytes > MAX_TOTAL_BYTES {
            return Err(Error::UnsafeTarEntry(format!(
                "archive size limit exceeded at {}",
                raw_path.display()
            )));
        }

        if let Some(p) = out_path.parent() {
            std::fs::create_dir_all(p)
                .map_err(|e| Error::io(format!("creating {}", p.display()), e))?;
        }

        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| Error::io(format!("reading file entry {}", safe.display()), e))?;
        std::fs::write(&out_path, &buf)
            .map_err(|e| Error::io(format!("writing {}", out_path.display()), e))?;
    }

    Ok(())
}

/// Strip the first path component (the wrapper directory). Returns
/// `None` for the top-level directory entry itself or empty paths.
fn strip_top_level(path: &Path) -> Option<PathBuf> {
    let mut comps = path.components();
    comps.next()?; // drop the wrapper directory
    let rest: PathBuf = comps.as_path().to_path_buf();
    if rest.as_os_str().is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// Whether `path` names the extraction root itself (`.`, `./`).
fn is_root(path: &Path) -> bool {
    path.components().all(|c| c == Component::CurDir)
}

/// Ensure a relative path is composed only of normal components — no `..`, no
/// absolute prefix, no root. Returns the normalized path, or `None` if unsafe.
fn sanitize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            // Current-dir markers are harmless; drop them.
            Component::CurDir => {}
            // Anything else (ParentDir, RootDir, Prefix) escapes the root.
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Create a unique, freshly-made temp directory under `parent`.
fn unique_temp_dir(parent: &Path) -> Result<PathBuf> {
    // Combine pid + a monotonic-ish counter to avoid collisions within a process
    // and a nanosecond timestamp across processes.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();

    for attempt in 0..1024u64 {
        let name = format!(".tmp-{pid}-{nanos}-{n}-{attempt}");
        let candidate = parent.join(name);
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(Error::io(
                    format!("creating temp dir under {}", parent.display()),
                    e,
                ))
            }
        }
    }
    Err(Error::io(
        format!("creating temp dir under {}", parent.display()),
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exhausted temp names"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_wrapper_directory() {
        assert_eq!(
            strip_top_level(Path::new("repo-sha/rpp.json")),
            Some(PathBuf::from("rpp.json"))
        );
        assert_eq!(strip_top_level(Path::new("repo-sha/")), None);
        assert_eq!(strip_top_level(Path::new("repo-sha")), None);
    }

    #[test]
    fn sanitize_rejects_traversal() {
        assert!(sanitize(Path::new("../escape")).is_none());
        assert!(sanitize(Path::new("/etc/passwd")).is_none());
        assert_eq!(sanitize(Path::new("a/./b")), Some(PathBuf::from("a/b")));
    }
}
