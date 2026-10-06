//! Content-addressed object store for output contents (spec §7).
//!
//! Objects live at `.rpp/cache/objects/<xxh3-hex>`. Objects are immutable and
//! shared across builds; unreferenced objects are garbage-collected.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::util::hash::{u64_hex, xxh3};

/// A handle to the CAS object directory.
#[derive(Clone)]
pub(crate) struct ObjectStore {
    dir: PathBuf,
}

impl ObjectStore {
    /// Open (and create) the object store rooted at `dir`.
    pub(crate) fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        Ok(Self { dir })
    }

    fn object_path(&self, key: u64) -> PathBuf {
        self.dir.join(u64_hex(key))
    }

    /// Store `contents`, returning the object key. Idempotent.
    ///
    /// Objects are published atomically but not fsynced: they are regenerable, and every read
    /// verifies the hash, so an object lost or truncated by a power failure is treated as missing.
    pub(crate) fn put(&self, contents: &[u8]) -> Result<u64> {
        let key = xxh3(contents);
        let path = self.object_path(key);
        let valid = std::fs::read(&path)
            .map(|existing| xxh3(&existing) == key)
            .unwrap_or(false);
        if !valid {
            crate::util::atomic::write_unsynced(&path, contents)
                .map_err(|e| Error::io(&path, e))?;
        }
        Ok(key)
    }

    /// Whether a valid object exists for `key` (reads the object). A corrupt object is left for
    /// the next `put` to replace.
    pub(crate) fn contains(&self, key: u64) -> bool {
        self.get(key).is_some()
    }

    /// Read an object by key, or `None` if it is missing or corrupt.
    pub(crate) fn get(&self, key: u64) -> Option<Vec<u8>> {
        std::fs::read(self.object_path(key))
            .ok()
            .filter(|bytes| xxh3(bytes) == key)
    }

    /// Copy an object to a mutable destination without hard-linking it to the immutable CAS
    /// entry. Like objects, the copy is published atomically but not fsynced, so callers must
    /// verify the destination against `key` on every build.
    pub(crate) fn copy_object(&self, key: u64, dest: &std::path::Path) -> Result<()> {
        let bytes = self
            .get(key)
            .ok_or_else(|| Error::Build(format!("missing cache object {key:#x}")))?;
        crate::util::atomic::write_unsynced(dest, &bytes).map_err(|e| Error::io(dest, e))
    }

    /// Garbage-collect objects not present in `live`.
    pub(crate) fn gc(&self, live: &HashSet<u64>) -> Result<usize> {
        let mut removed = 0;
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return Ok(0),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // Skip stray temp files.
            if name.ends_with(".tmp") {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if let Ok(key) = u64::from_str_radix(name, 16) {
                if !live.contains(&key) && std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}
