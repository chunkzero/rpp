//! The registry lock: `rpp.lock` version 3, next to `rpp.json`.
//!
//! ```toml
//! version = 3
//!
//! [[package]]
//! name = "window"
//! requested = "^0.1.0"
//! version = "0.1.4"
//! rpp = ">=0.2"
//! url = "https://github.com/chunkzero/window/releases/download/v0.1.4/window-0.1.4.rpp.tgz"
//! sha256 = "…"
//! ```

use std::io::Write;
use std::path::Path;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The lock format version written and accepted.
pub const PACKAGE_LOCK_VERSION: u32 = 3;

/// One pinned registry package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    /// Registry name.
    pub name: String,
    /// The dependency spec it was resolved for, exactly as written in `rpp.json`.
    pub requested: String,
    /// The chosen version.
    pub version: Version,
    /// That version's supported rpp range, so compatibility is checked offline.
    pub rpp: VersionReq,
    /// Archive URL.
    pub url: String,
    /// Lowercase hex SHA-256 of the archive.
    pub sha256: String,
}

/// The set of pinned packages, kept sorted by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageLock {
    packages: Vec<LockedPackage>,
}

impl PackageLock {
    /// An empty lock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load `path`; a missing file is an empty lock.
    ///
    /// # Errors
    ///
    /// [`crate::Error::UnsupportedLockVersion`] when `version` isn't 3 (this
    /// includes legacy `rpp.toml` locks), [`crate::Error::InvalidLockfile`] for
    /// malformed TOML or duplicate names, or an I/O error.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(Error::io(format!("reading {}", path.display()), e)),
        };
        let invalid = |reason: String| Error::InvalidLockfile {
            path: path.to_path_buf(),
            reason,
        };

        let header: RawHeader = toml::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        if header.version != PACKAGE_LOCK_VERSION {
            return Err(Error::UnsupportedLockVersion {
                found: header.version,
                supported: PACKAGE_LOCK_VERSION,
            });
        }

        let raw: RawLock = toml::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        let mut packages: Vec<LockedPackage> = raw.packages.into_iter().map(Into::into).collect();
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(pair) = packages.windows(2).find(|w| w[0].name == w[1].name) {
            return Err(invalid(format!("duplicate package `{}`", pair[0].name)));
        }
        Ok(Self { packages })
    }

    /// Serialize as shown in the module docs, packages sorted by name.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        let raw = RawLockRef {
            version: PACKAGE_LOCK_VERSION,
            packages: self.packages.iter().map(RawPackage::from).collect(),
        };
        let mut out = toml::to_string_pretty(&raw).map_err(|e| Error::InvalidLockfile {
            path: "rpp.lock".into(),
            reason: e.to_string(),
        })?;
        if !out.ends_with('\n') {
            out.push('\n');
        }
        Ok(out)
    }

    /// Write atomically (temp file + rename in the same directory).
    ///
    /// # Errors
    ///
    /// Returns an I/O error.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.to_toml()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| Error::io(format!("staging {}", path.display()), e))?;
        file.write_all(text.as_bytes())
            .map_err(|e| Error::io(format!("writing {}", path.display()), e))?;
        file.as_file()
            .sync_all()
            .map_err(|e| Error::io(format!("flushing {}", path.display()), e))?;
        file.persist(path)
            .map(|_| ())
            .map_err(|e| Error::io(format!("replacing {}", path.display()), e.error))
    }

    /// The pin for `name`, if any.
    pub fn get(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Insert or replace the pin for `package.name`, returning the previous pin.
    pub fn upsert(&mut self, package: LockedPackage) -> Option<LockedPackage> {
        match self
            .packages
            .binary_search_by(|p| p.name.cmp(&package.name))
        {
            Ok(index) => Some(std::mem::replace(&mut self.packages[index], package)),
            Err(index) => {
                self.packages.insert(index, package);
                None
            }
        }
    }

    /// Drop pins whose name isn't in `names`. Returns whether anything was removed.
    pub fn retain_names(&mut self, names: &[&str]) -> bool {
        let before = self.packages.len();
        self.packages.retain(|p| names.contains(&p.name.as_str()));
        self.packages.len() != before
    }

    /// All pins, sorted by name.
    pub fn packages(&self) -> &[LockedPackage] {
        &self.packages
    }
}

#[derive(Deserialize)]
struct RawHeader {
    #[serde(default)]
    version: u32,
}

#[derive(Deserialize)]
struct RawLock {
    #[serde(default, rename = "package")]
    packages: Vec<RawPackage>,
}

#[derive(Serialize)]
struct RawLockRef {
    version: u32,
    #[serde(rename = "package")]
    packages: Vec<RawPackage>,
}

#[derive(Serialize, Deserialize)]
struct RawPackage {
    name: String,
    requested: String,
    version: Version,
    rpp: VersionReq,
    url: String,
    sha256: String,
}

impl From<&LockedPackage> for RawPackage {
    fn from(p: &LockedPackage) -> Self {
        Self {
            name: p.name.clone(),
            requested: p.requested.clone(),
            version: p.version.clone(),
            rpp: p.rpp.clone(),
            url: p.url.clone(),
            sha256: p.sha256.clone(),
        }
    }
}

impl From<RawPackage> for LockedPackage {
    fn from(p: RawPackage) -> Self {
        Self {
            name: p.name,
            requested: p.requested,
            version: p.version,
            rpp: p.rpp,
            url: p.url,
            sha256: p.sha256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(name: &str, version: &str) -> LockedPackage {
        LockedPackage {
            name: name.to_string(),
            requested: "^0.1.0".to_string(),
            version: Version::parse(version).unwrap(),
            rpp: VersionReq::parse(">=0.2").unwrap(),
            url: format!("https://example.com/{name}-{version}.rpp.tgz"),
            sha256: "ab".repeat(32),
        }
    }

    #[test]
    fn round_trips_sorted() {
        let mut lock = PackageLock::new();
        lock.upsert(pin("window", "0.1.4"));
        assert!(lock.upsert(pin("anvil", "1.0.0")).is_none());
        let previous = lock.upsert(pin("window", "0.1.5")).unwrap();
        assert_eq!(previous.version, Version::new(0, 1, 4));

        let text = lock.to_toml().unwrap();
        assert!(text.starts_with("version = 3\n"));
        assert!(text.find("anvil").unwrap() < text.find("window").unwrap());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpp.lock");
        lock.save(&path).unwrap();
        let loaded = PackageLock::load(&path).unwrap();
        assert_eq!(loaded, lock);
        let names: Vec<_> = loaded.packages().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["anvil", "window"]);
    }

    #[test]
    fn rejects_legacy_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpp.lock");
        std::fs::write(
            &path,
            "version = 2\n\n[[plugin]]\nsource = \"github:a/b\"\n",
        )
        .unwrap();
        assert!(matches!(
            PackageLock::load(&path),
            Err(Error::UnsupportedLockVersion {
                found: 2,
                supported: 3
            })
        ));
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let lock = PackageLock::load(&dir.path().join("rpp.lock")).unwrap();
        assert!(lock.packages().is_empty());
    }

    #[test]
    fn retain_names_drops_others() {
        let mut lock = PackageLock::new();
        lock.upsert(pin("a", "1.0.0"));
        lock.upsert(pin("b", "1.0.0"));
        assert!(lock.retain_names(&["b", "c"]));
        assert!(!lock.retain_names(&["b"]));
        assert!(lock.get("a").is_none());
        assert!(lock.get("b").is_some());
    }
}
