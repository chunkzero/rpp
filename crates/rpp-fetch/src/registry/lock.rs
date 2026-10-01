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

use std::path::Path;

use semver::{Version, VersionReq};

use crate::error::Result;

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
        todo!("{}", path.display())
    }

    /// Serialize as shown in the module docs, packages sorted by name.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        todo!()
    }

    /// Write atomically (temp file + rename in the same directory).
    ///
    /// # Errors
    ///
    /// Returns an I/O error.
    pub fn save(&self, path: &Path) -> Result<()> {
        todo!("{}", path.display())
    }

    /// The pin for `name`, if any.
    pub fn get(&self, name: &str) -> Option<&LockedPackage> {
        todo!("{name}")
    }

    /// Insert or replace the pin for `package.name`, returning the previous pin.
    pub fn upsert(&mut self, package: LockedPackage) -> Option<LockedPackage> {
        todo!("{package:?}")
    }

    /// Drop pins whose name isn't in `names`. Returns whether anything was removed.
    pub fn retain_names(&mut self, names: &[&str]) -> bool {
        todo!("{names:?}")
    }

    /// All pins, sorted by name.
    pub fn packages(&self) -> &[LockedPackage] {
        &self.packages
    }
}
