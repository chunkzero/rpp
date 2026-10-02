//! Types deserialized from the registry's index files.

use semver::{Version, VersionReq};
use serde::Deserialize;

/// One plugin's index file, `plugins/<name>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IndexEntry {
    /// Registry name; equals the `name` in each archive's `rpp.json`.
    pub name: String,
    /// Repository URL whose releases host the archives.
    pub repository: String,
    /// Short description shown by search.
    #[serde(default)]
    pub description: Option<String>,
    /// Published versions, oldest first.
    pub versions: Vec<IndexVersion>,
}

/// One published version of a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IndexVersion {
    /// The plugin version.
    pub version: Version,
    /// Absolute URL of the gzipped tar release archive.
    pub url: String,
    /// Lowercase hex SHA-256 of the archive bytes.
    pub sha256: String,
    /// rpp versions this plugin version supports.
    pub rpp: VersionReq,
    /// Yanked versions are skipped by new resolutions but still install when locked.
    #[serde(default)]
    pub yanked: bool,
}

/// One entry of the registry's `index.json` summary.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SearchHit {
    /// Registry name.
    pub name: String,
    /// Short description.
    #[serde(default)]
    pub description: Option<String>,
    /// Repository URL.
    pub repository: String,
    /// Newest non-yanked version.
    pub latest: Version,
}
