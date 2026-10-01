//! Plugin packages resolved from the git-based registry (`chunkzero/rpp-registry`)
//! or from local `path:` directories, as declared in a project's `rpp.json`.
//!
//! The registry is a set of static JSON files: `plugins/<name>.json` lists every
//! published version of one plugin, and `index.json` summarizes all plugins for
//! search. Versions point at release archives verified by SHA-256.

mod deps;
mod install;
mod lock;
mod resolve;

use std::path::{Path, PathBuf};

use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::http::HttpConfig;

pub use deps::{parse_dependencies, validate_name, Dependency, DependencySpec};
pub use lock::{LockedPackage, PackageLock, PACKAGE_LOCK_VERSION};
pub use resolve::{resolve, ResolvedPackage, Update};

/// File name of the manifest shared by projects and plugin packages.
pub const PACKAGE_MANIFEST: &str = "rpp.json";

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

/// The `name` and `version` of a plugin package's `rpp.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSummary {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
}

/// Client for the registry index plus the local archive cache.
pub struct Registry {
    http: HttpConfig,
    agent: ureq::Agent,
    cache_root: PathBuf,
}

impl Registry {
    /// A registry client reading the index at `http.registry_base` and caching
    /// extracted archives under `cache_root`.
    pub fn new(http: HttpConfig, cache_root: impl Into<PathBuf>) -> Self {
        todo!("{http:?} {:?}", cache_root.into())
    }

    /// A client using [`HttpConfig::default`] and `<rpp cache>/registry`, where the
    /// rpp cache is `RPP_CACHE_DIR` if set, else `~/.cache/rpp`.
    ///
    /// # Errors
    ///
    /// Returns an error if no cache directory can be determined.
    pub fn from_env() -> Result<Self> {
        todo!()
    }

    /// Fetch `plugins/<name>.json`. A 404 is [`Error::UnknownPackage`].
    ///
    /// # Errors
    ///
    /// Returns an error for invalid names, network failures, or malformed index files.
    pub fn entry(&self, name: &str) -> Result<IndexEntry> {
        todo!("{name}")
    }

    /// Fetch `index.json` and keep entries whose name or description contains
    /// `query`, case-insensitively, sorted by name.
    ///
    /// # Errors
    ///
    /// Returns an error for network failures or a malformed summary.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        todo!("{query}")
    }

    /// Ensure the archive for `name` `version` is extracted in the cache and return
    /// its directory. A cache hit makes no network request. Otherwise the archive is
    /// downloaded from `url`, its SHA-256 checked against `sha256`, extracted, and
    /// its `rpp.json` checked to declare exactly `name` and `version`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::HashMismatch`], [`Error::PackageMismatch`], an unsafe-archive
    /// error, or a network/I/O error. Nothing is left in the cache on failure.
    pub fn install(
        &self,
        name: &str,
        version: &Version,
        url: &str,
        sha256: &str,
    ) -> Result<PathBuf> {
        todo!("{name} {version} {url} {sha256}")
    }
}

/// Choose the newest non-yanked version of `entry` matching `requested` whose
/// `rpp` range accepts `rpp`.
///
/// Compatibility ignores pre-release and build metadata of `rpp`, so rpp
/// `0.2.0-alpha.1` satisfies a plugin's `>=0.2`. Pre-release plugin versions match
/// only when `requested` names a pre-release (semver's default).
///
/// # Errors
///
/// [`Error::NoMatchingVersion`] when nothing non-yanked matches `requested`;
/// [`Error::Incompatible`] (naming the newest match and its requirement) when
/// versions match but none supports `rpp`.
pub fn select<'a>(
    entry: &'a IndexEntry,
    requested: &VersionReq,
    rpp: &Version,
) -> Result<&'a IndexVersion> {
    todo!("{entry:?} {requested} {rpp}")
}

/// Read `name` and `version` from the `rpp.json` in `dir`, ignoring other fields.
///
/// # Errors
///
/// [`Error::MissingPackageManifest`] when absent; [`Error::InvalidManifest`] when
/// it is not JSON, lacks either field, or has an invalid name or version.
pub fn read_package_summary(dir: &Path) -> Result<PackageSummary> {
    todo!("{}", dir.display())
}

/// `rpp` with pre-release and build metadata removed, for compatibility checks.
pub(crate) fn release_of(rpp: &Version) -> Version {
    Version::new(rpp.major, rpp.minor, rpp.patch)
}

#[allow(dead_code)]
fn _unused(_: Error) {}
