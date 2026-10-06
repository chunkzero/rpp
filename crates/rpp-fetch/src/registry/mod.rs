//! Plugin packages resolved from the git-based registry (`chunkzero/rpp-registry`)
//! or from local `path:` directories, as declared in a project's `rpp.json`.
//!
//! The registry is a set of static JSON files: `plugins/<name>.json` lists every
//! published version of one plugin, and `index.json` summarizes all plugins for
//! search. Versions point at release archives verified by SHA-256.

mod deps;
mod index;
mod install;
mod lock;
mod resolve;

use std::path::{Path, PathBuf};

use semver::Version;
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::http::{HttpConfig, RegistryClient};

pub use deps::{parse_dependencies, validate_name, Dependency, DependencySpec};
pub use index::{IndexEntry, IndexVersion, SearchHit};
pub use lock::{LockedPackage, PackageLock, PACKAGE_LOCK_VERSION};
pub use resolve::{resolve, select, ResolvedPackage, Update};

/// File name of the manifest shared by projects and plugin packages.
pub const PACKAGE_MANIFEST: &str = "rpp.json";

/// The `name` and `version` of a plugin package's `rpp.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSummary {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
}

/// The user-wide rpp cache directory: `RPP_CACHE_DIR` if set and non-empty, else
/// `rpp` under the platform cache directory (`~/.cache/rpp` on Linux).
///
/// # Errors
///
/// Returns an error if `RPP_CACHE_DIR` is unset and no platform cache directory exists.
pub fn cache_root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("RPP_CACHE_DIR").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    dirs::cache_dir().map(|dir| dir.join("rpp")).ok_or_else(|| {
        Error::io(
            "determining user cache directory",
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no cache directory available; set RPP_CACHE_DIR",
            ),
        )
    })
}

/// Client for the registry index plus the local archive cache.
pub struct Registry {
    client: RegistryClient,
    cache_root: PathBuf,
}

impl Registry {
    /// A registry client reading the index at `http.registry_base` and caching
    /// extracted archives under `cache_root`.
    pub fn new(http: HttpConfig, cache_root: impl Into<PathBuf>) -> Self {
        Registry {
            client: RegistryClient::new(http),
            cache_root: cache_root.into(),
        }
    }

    /// A client using [`HttpConfig::default`] and `<rpp cache>/registry`, where the
    /// rpp cache is [`cache_root`].
    ///
    /// # Errors
    ///
    /// Returns an error if no cache directory can be determined.
    pub fn from_env() -> Result<Self> {
        Ok(Self::new(
            HttpConfig::default(),
            cache_root()?.join("registry"),
        ))
    }

    /// Fetch `plugins/<name>.json`. A 404 is [`Error::UnknownPackage`].
    ///
    /// # Errors
    ///
    /// Returns an error for invalid names, network failures, or malformed index files.
    pub fn entry(&self, name: &str) -> Result<IndexEntry> {
        validate_name(name)?;
        let url = format!("{}/plugins/{name}.json", self.client.config.registry_base);
        let entry: IndexEntry = self.client.get_json(&url).map_err(|f| {
            if f.status == Some(404) {
                Error::UnknownPackage(name.to_string())
            } else {
                f.into_error(&url)
            }
        })?;
        if entry.name != name {
            return Err(Error::Registry {
                url,
                reason: format!("index entry is for `{}`, expected `{name}`", entry.name),
            });
        }
        Ok(entry)
    }

    /// Fetch `index.json` and keep entries whose name or description contains
    /// `query`, case-insensitively, sorted by name.
    ///
    /// # Errors
    ///
    /// Returns an error for network failures or a malformed summary.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let url = format!("{}/index.json", self.client.config.registry_base);
        let hits: Vec<SearchHit> = self.client.get_json(&url).map_err(|f| f.into_error(&url))?;
        let needle = query.to_lowercase();
        let mut hits: Vec<SearchHit> = hits
            .into_iter()
            .filter(|hit| {
                hit.name.to_lowercase().contains(&needle)
                    || hit
                        .description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&needle))
            })
            .collect();
        hits.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(hits)
    }

    /// Ensure the archive for `name` `version` is extracted in the cache and return
    /// its directory. A cache hit makes no network request. Otherwise the archive is
    /// downloaded from `url`, its SHA-256 checked against `sha256`, extracted, and
    /// its `rpp.json` checked to declare exactly `name` and `version`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::HashMismatch`], [`Error::PackageMismatch`], [`Error::Archive`],
    /// or a network/I/O error. Nothing is left in the cache on failure.
    pub fn install(
        &self,
        name: &str,
        version: &Version,
        url: &str,
        sha256: &str,
    ) -> Result<PathBuf> {
        install::install(&self.client, &self.cache_root, name, version, url, sha256)
    }
}

/// Read `name` and `version` from the `rpp.json` in `dir`, ignoring other fields.
///
/// # Errors
///
/// [`Error::MissingPackageManifest`] when absent; [`Error::InvalidManifest`] when
/// it is not JSON, lacks either field, or has an invalid name or version.
pub fn read_package_summary(dir: &Path) -> Result<PackageSummary> {
    let path = dir.join(PACKAGE_MANIFEST);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::MissingPackageManifest(dir.to_path_buf()));
        }
        Err(e) => return Err(Error::io(format!("reading {}", path.display()), e)),
    };
    let invalid = |reason: String| Error::InvalidManifest {
        path: path.clone(),
        reason,
    };

    #[derive(Deserialize)]
    struct Fields {
        name: String,
        version: String,
    }
    let fields: Fields = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    validate_name(&fields.name).map_err(|e| invalid(e.to_string()))?;
    let version = Version::parse(&fields.version)
        .map_err(|e| invalid(format!("invalid version `{}`: {e}", fields.version)))?;
    Ok(PackageSummary {
        name: fields.name,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_package_summary() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_package_summary(dir.path()),
            Err(Error::MissingPackageManifest(_))
        ));
        let manifest = dir.path().join(PACKAGE_MANIFEST);
        std::fs::write(
            &manifest,
            r#"{"name": "window", "version": "0.1.4", "extra": 1}"#,
        )
        .unwrap();
        let summary = read_package_summary(dir.path()).unwrap();
        assert_eq!(summary.name, "window");
        assert_eq!(summary.version, Version::new(0, 1, 4));

        std::fs::write(&manifest, r#"{"name": "Window", "version": "0.1.4"}"#).unwrap();
        assert!(matches!(
            read_package_summary(dir.path()),
            Err(Error::InvalidManifest { .. })
        ));
    }
}
