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

/// Atomically replace `path` with `record`, creating its parent directories.
pub(crate) fn save<T: Versioned>(path: &Path, record: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let bytes = bincode::serde::encode_to_vec(record, bincode::config::standard())
        .map_err(|e| Error::Build(e.to_string()))?;
    super::atomic::write(path, &bytes).map_err(|e| Error::io(path, e))
}
