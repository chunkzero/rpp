//! Resolving a project's dependencies against the lock, the cache and the registry.

use std::path::{Path, PathBuf};

use semver::Version;

use crate::error::Result;

use super::{Dependency, PackageLock, Registry};

/// Which registry dependencies may move to a newer version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// Keep every usable pin.
    None,
    /// Re-select every registry dependency.
    All,
    /// Re-select only these names.
    Only(Vec<String>),
}

/// A dependency resolved to a local package directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    /// Dependency name.
    pub name: String,
    /// Directory containing the package's `rpp.json`.
    pub root: PathBuf,
    /// The registry version; `None` for `path:` dependencies.
    pub version: Option<Version>,
}

/// Resolve `deps` in order, updating `lock` in place.
///
/// - `path:` dependencies resolve to the canonicalized directory (relative to
///   `project_root`), whose `rpp.json` name must equal the dependency name. They are
///   never locked.
/// - A registry dependency reuses its pin when the dependency isn't selected by
///   `update`, the pin's `requested` equals the dependency's `raw` spec, and the pinned
///   version still satisfies the range. A reused pin makes no network request when
///   its archive is cached, and its `rpp` range must accept `rpp` (else
///   [`crate::Error::Incompatible`] with a hint to run `rpp update <name>`).
/// - Otherwise the index entry is fetched, a version chosen with [`super::select`],
///   installed, and pinned.
/// - Pins for names no longer among the registry dependencies are dropped.
///
/// # Errors
///
/// Any error from selection, installation, or path validation, naming the dependency.
pub fn resolve(
    registry: &Registry,
    project_root: &Path,
    deps: &[Dependency],
    lock: &mut PackageLock,
    rpp: &Version,
    update: &Update,
) -> Result<Vec<ResolvedPackage>> {
    todo!(
        "{} {deps:?} {lock:?} {rpp} {update:?} {}",
        project_root.display(),
        std::ptr::addr_of!(*registry) as usize
    )
}
