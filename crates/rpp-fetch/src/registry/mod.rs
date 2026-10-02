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

use crate::error::{Error, Incompatibility, Result};
use crate::http::{HttpConfig, RegistryClient};

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
    /// rpp cache is `RPP_CACHE_DIR` if set, else `~/.cache/rpp`.
    ///
    /// # Errors
    ///
    /// Returns an error if no cache directory can be determined.
    pub fn from_env() -> Result<Self> {
        let base = match std::env::var_os("RPP_CACHE_DIR").filter(|dir| !dir.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => dirs::cache_dir()
                .ok_or_else(|| {
                    Error::io(
                        "determining user cache directory",
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "no cache directory available; set RPP_CACHE_DIR",
                        ),
                    )
                })?
                .join("rpp"),
        };
        Ok(Self::new(HttpConfig::default(), base.join("registry")))
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
    let mut matching: Vec<&IndexVersion> = entry
        .versions
        .iter()
        .filter(|v| !v.yanked && requested.matches(&v.version))
        .collect();
    matching.sort_by(|a, b| b.version.cmp(&a.version));
    let Some(newest) = matching.first() else {
        return Err(Error::NoMatchingVersion {
            name: entry.name.clone(),
            requested: requested.to_string(),
        });
    };
    let release = release_of(rpp);
    matching
        .iter()
        .find(|v| v.rpp.matches(&release))
        .copied()
        .ok_or_else(|| {
            Error::Incompatible(Box::new(Incompatibility {
                name: entry.name.clone(),
                version: newest.version.clone(),
                requires: newest.rpp.clone(),
                rpp: rpp.clone(),
                hint: format!(
                    "request an older version of `{}` or upgrade rpp",
                    entry.name
                ),
            }))
        })
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

/// `rpp` with pre-release and build metadata removed, for compatibility checks.
pub(crate) fn release_of(rpp: &Version) -> Version {
    Version::new(rpp.major, rpp.minor, rpp.patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(v: &str, rpp: &str, yanked: bool) -> IndexVersion {
        IndexVersion {
            version: Version::parse(v).unwrap(),
            url: format!("https://example.com/{v}.tgz"),
            sha256: "00".repeat(32),
            rpp: VersionReq::parse(rpp).unwrap(),
            yanked,
        }
    }

    fn entry(versions: Vec<IndexVersion>) -> IndexEntry {
        IndexEntry {
            name: "window".to_string(),
            repository: "https://example.com/window".to_string(),
            description: None,
            versions,
        }
    }

    fn req(text: &str) -> VersionReq {
        VersionReq::parse(text).unwrap()
    }

    fn rpp(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn selects_highest_compatible() {
        let e = entry(vec![
            version("0.1.0", ">=0.1", false),
            version("0.1.2", ">=0.1", false),
            version("0.1.3", ">=0.3", false),
        ]);
        let chosen = select(&e, &req("^0.1"), &rpp("0.2.0")).unwrap();
        assert_eq!(chosen.version, rpp("0.1.2"));
    }

    #[test]
    fn skips_yanked() {
        let e = entry(vec![
            version("0.1.0", ">=0.1", false),
            version("0.1.1", ">=0.1", true),
        ]);
        assert_eq!(
            select(&e, &req("^0.1"), &rpp("0.2.0")).unwrap().version,
            rpp("0.1.0")
        );
        let only_yanked = entry(vec![version("0.1.1", ">=0.1", true)]);
        assert!(matches!(
            select(&only_yanked, &req("^0.1"), &rpp("0.2.0")),
            Err(Error::NoMatchingVersion { .. })
        ));
    }

    #[test]
    fn incompatible_names_newest_match() {
        let e = entry(vec![
            version("0.1.0", ">=0.5", false),
            version("0.1.1", ">=0.6", false),
        ]);
        match select(&e, &req("^0.1"), &rpp("0.2.0")) {
            Err(Error::Incompatible(info)) => {
                assert_eq!(info.version, rpp("0.1.1"));
                assert_eq!(info.requires, req(">=0.6"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn prerelease_needs_explicit_range() {
        let e = entry(vec![version("0.2.0-beta.1", ">=0.1", false)]);
        assert!(matches!(
            select(&e, &req("^0.2"), &rpp("0.2.0")),
            Err(Error::NoMatchingVersion { .. })
        ));
        assert!(select(&e, &req(">=0.2.0-beta.1"), &rpp("0.2.0")).is_ok());
    }

    #[test]
    fn prerelease_rpp_is_compatible() {
        let e = entry(vec![version("0.1.0", ">=0.2", false)]);
        assert!(select(&e, &req("^0.1"), &rpp("0.2.0-alpha.1")).is_ok());
    }

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
        assert_eq!(summary.version, rpp("0.1.4"));

        std::fs::write(&manifest, r#"{"name": "Window", "version": "0.1.4"}"#).unwrap();
        assert!(matches!(
            read_package_summary(dir.path()),
            Err(Error::InvalidManifest { .. })
        ));
    }
}
