//! The incremental build manifest (spec §7).
//!
//! Persisted as bincode at `.rpp/cache/manifest.bin`. A corrupt or
//! version-mismatched manifest is treated as empty (triggering a full rebuild).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::ReadKind;

/// Current on-disk manifest format version. Bumping forces a full rebuild.
pub(crate) const MANIFEST_VERSION: u32 = 2;

/// A file fingerprint used for fast change detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Fingerprint {
    /// Modification time in nanoseconds since the Unix epoch.
    pub(crate) mtime_ns: u128,
    /// File size in bytes.
    pub(crate) size: u64,
    /// xxh3-64 content hash.
    pub(crate) xxh3: u64,
}

/// A single output produced for a source file or generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OutputRef {
    /// Relative output path.
    pub(crate) path: String,
    /// CAS object key (xxh3-64 of the contents).
    pub(crate) object: u64,
}

/// Per-source-file cache entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileEntry {
    /// Source-file fingerprint.
    pub(crate) fingerprint: Fingerprint,
    /// Hash over the ordered processor chain that applied to this file.
    pub(crate) chain_key: u64,
    /// Output files (empty means the file was dropped).
    pub(crate) outputs: Vec<OutputRef>,
}

/// Serializable form of [`ReadKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReadKindRepr {
    List,
    File,
    Source,
}

impl From<ReadKind> for ReadKindRepr {
    fn from(k: ReadKind) -> Self {
        match k {
            ReadKind::List => ReadKindRepr::List,
            ReadKind::File => ReadKindRepr::File,
            ReadKind::Source => ReadKindRepr::Source,
        }
    }
}

/// A single recorded generator dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReadRecord {
    /// What kind of read this was.
    pub(crate) kind: ReadKindRepr,
    /// The query key (glob for List, path for File/Source).
    pub(crate) key: String,
    /// Hash of the read result, used to detect changes on replay.
    pub(crate) hash: u64,
}

/// Per-generator cache entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GeneratorEntry {
    /// The owning plugin's cache key.
    pub(crate) plugin_key: u64,
    /// The recorded read-set from the last run.
    pub(crate) read_set: Vec<ReadRecord>,
    /// Ordered output mutations produced by the generator.
    pub(crate) mutations: Vec<GeneratorMutation>,
}

/// One cached generator mutation, replayed in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum GeneratorMutation {
    /// Add or overwrite an output file.
    Emit(OutputRef),
    /// Remove an output file.
    Remove(String),
}

/// The full build manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    /// Format version.
    pub(crate) version: u32,
    /// Global cache key (rpp version + config + plugin keys).
    pub(crate) global_key: u64,
    /// Per-source-file entries, keyed by relative source path.
    pub(crate) files: BTreeMap<String, FileEntry>,
    /// Per-generator entries, keyed by plugin id.
    pub(crate) generators: BTreeMap<String, GeneratorEntry>,
}

impl Manifest {
    /// Create an empty manifest for the given global key.
    pub(crate) fn empty(global_key: u64) -> Self {
        Self {
            version: MANIFEST_VERSION,
            global_key,
            files: BTreeMap::new(),
            generators: BTreeMap::new(),
        }
    }

    /// Load a manifest from disk, returning `None` if it is missing, corrupt, or
    /// a version mismatch (all of which mean "rebuild from scratch").
    pub(crate) fn load(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let config = bincode::config::standard();
        let (manifest, _): (Manifest, usize) =
            bincode::serde::decode_from_slice(&bytes, config).ok()?;
        if manifest.version != MANIFEST_VERSION {
            return None;
        }
        Some(manifest)
    }

    /// Persist the manifest to disk (creating parent directories).
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let config = bincode::config::standard();
        let bytes =
            bincode::serde::encode_to_vec(self, config).map_err(|e| Error::Build(e.to_string()))?;
        std::fs::write(path, bytes).map_err(|e| Error::io(path, e))
    }
}
