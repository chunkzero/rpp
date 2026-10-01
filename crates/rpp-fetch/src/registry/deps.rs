//! Parsing of the `dependencies` object in a project's `rpp.json`.

use std::path::PathBuf;

use semver::VersionReq;

use crate::error::Result;

/// Where a dependency comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySpec {
    /// A registry version range. A bare version (`0.1.4`) means exactly that version;
    /// operators (`^0.1`, `~0.1.2`, `>=0.1, <0.3`) follow semver.
    Registry(VersionReq),
    /// `path:<dir>`, relative to the project root unless absolute. Never locked.
    Path(PathBuf),
}

impl DependencySpec {
    /// Parse one dependency value.
    ///
    /// # Errors
    ///
    /// [`crate::Error::InvalidDependency`] (with `name`) for an empty `path:`, an
    /// invalid range, or `*`/empty ranges (a range must be explicit).
    pub fn parse(name: &str, spec: &str) -> Result<Self> {
        todo!("{name} {spec}")
    }
}

/// One `"name": "spec"` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// Plugin name; must equal the name in the package's `rpp.json`.
    pub name: String,
    /// The parsed spec.
    pub spec: DependencySpec,
    /// The spec exactly as written, recorded in the lock as `requested`.
    pub raw: String,
}

/// Parse the `dependencies` object of an `rpp.json` document, in document order.
/// A missing `dependencies` key means no dependencies.
///
/// # Errors
///
/// [`crate::Error::InvalidManifest`] (with `path` `rpp.json`) when the text is not a
/// JSON object or `dependencies` is not an object of strings;
/// [`crate::Error::InvalidDependency`] for invalid names or specs.
pub fn parse_dependencies(manifest_json: &str) -> Result<Vec<Dependency>> {
    let _ = PathBuf::new();
    todo!("{manifest_json}")
}

/// Validate a plugin name: `^[a-z0-9][a-z0-9_-]*$`, at most 64 characters.
///
/// # Errors
///
/// [`crate::Error::InvalidDependency`] with an empty `spec`.
pub fn validate_name(name: &str) -> Result<()> {
    todo!("{name}")
}
