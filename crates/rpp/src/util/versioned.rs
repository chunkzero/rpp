//! Bincode files that carry a format version.

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{Error, Result};

/// A bincode record that stores its format version.
pub(crate) trait Versioned: Serialize + DeserializeOwned {
    /// The version this build reads and writes.
    const VERSION: u32;

    fn version(&self) -> u32;
}

/// Read the record at `path`. `Ok(None)` means the file does not decode or has another
/// version; read failures, including a missing file, are errors.
pub(crate) fn load<T: Versioned>(path: &Path) -> std::io::Result<Option<T>> {
    let bytes = std::fs::read(path)?;
    let decoded = bincode::serde::decode_from_slice::<T, _>(&bytes, bincode::config::standard());
    Ok(decoded
        .ok()
        .map(|(record, _)| record)
        .filter(|record| record.version() == T::VERSION))
}

/// Atomically replace `path` with `record`, creating its parent directories. A regular file that
/// already holds the encoded record is left untouched; a symlink is always replaced.
pub(crate) fn save<T: Versioned>(path: &Path, record: &T) -> Result<()> {
    let bytes = bincode::serde::encode_to_vec(record, bincode::config::standard())
        .map_err(|e| Error::Build(e.to_string()))?;
    if holds(path, &bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    super::atomic::write(path, &bytes).map_err(|e| Error::io(path, e))
}

fn holds(path: &Path, bytes: &[u8]) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == bytes.len() as u64)
        && std::fs::read(path).is_ok_and(|existing| existing == bytes)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Record {
        version: u32,
        value: u32,
    }

    impl Versioned for Record {
        const VERSION: u32 = 1;

        fn version(&self) -> u32 {
            self.version
        }
    }

    fn record(value: u32) -> Record {
        Record { version: 1, value }
    }

    #[test]
    fn save_skips_identical_bytes_and_replaces_changed_ones() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/record.bin");
        save(&path, &record(1)).unwrap();
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
        let first = std::fs::metadata(&path).unwrap();
        save(&path, &record(1)).unwrap();
        assert_eq!(
            first.modified().unwrap(),
            std::fs::metadata(&path).unwrap().modified().unwrap()
        );

        save(&path, &record(2)).unwrap();
        assert_eq!(load::<Record>(&path).unwrap().unwrap().value, 2);
    }

    #[cfg(unix)]
    #[test]
    fn save_replaces_a_symlink_even_when_its_target_matches() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.bin");
        let link = directory.path().join("link.bin");
        save(&target, &record(1)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        save(&link, &record(1)).unwrap();

        assert!(!std::fs::symlink_metadata(&link).unwrap().is_symlink());
    }
}
