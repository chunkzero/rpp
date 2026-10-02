//! Resolving a project's dependencies against the lock, the cache and the registry.

use std::path::{Path, PathBuf};

use semver::{Version, VersionReq};

use crate::error::{Error, Incompatibility, Result};

use super::{
    read_package_summary, Dependency, DependencySpec, IndexEntry, IndexVersion, LockedPackage,
    PackageLock, Registry,
};

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
/// - Otherwise the index entry is fetched, a version chosen with [`select`],
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
    let mut resolved = Vec::with_capacity(deps.len());
    for dep in deps {
        resolved.push(match &dep.spec {
            DependencySpec::Path(dir) => resolve_path(project_root, dep, dir)?,
            DependencySpec::Registry(requested) => {
                resolve_registry(registry, dep, requested, lock, rpp, update)?
            }
        });
    }
    let names: Vec<&str> = deps
        .iter()
        .filter(|d| matches!(d.spec, DependencySpec::Registry(_)))
        .map(|d| d.name.as_str())
        .collect();
    lock.retain_names(&names);
    Ok(resolved)
}

fn resolve_path(project_root: &Path, dep: &Dependency, dir: &Path) -> Result<ResolvedPackage> {
    let joined = project_root.join(dir);
    let root = joined
        .canonicalize()
        .ok()
        .filter(|p| p.is_dir())
        .ok_or(Error::PathNotFound(joined))?;
    let found = read_package_summary(&root)?;
    if found.name != dep.name {
        return Err(Error::PackageMismatch {
            path: root,
            expected: dep.name.clone(),
            found: found.name,
        });
    }
    Ok(ResolvedPackage {
        name: dep.name.clone(),
        root,
        version: None,
    })
}

fn resolve_registry(
    registry: &Registry,
    dep: &Dependency,
    requested: &VersionReq,
    lock: &mut PackageLock,
    rpp: &Version,
    update: &Update,
) -> Result<ResolvedPackage> {
    let selected = match update {
        Update::None => false,
        Update::All => true,
        Update::Only(names) => names.contains(&dep.name),
    };
    let pin = lock
        .get(&dep.name)
        .filter(|p| !selected && p.requested == dep.raw && requested.matches(&p.version))
        .cloned();

    let locked = match pin {
        Some(pin) => {
            if !pin.rpp.matches(&release_of(rpp)) {
                return Err(Error::Incompatible(Box::new(Incompatibility {
                    name: dep.name.clone(),
                    version: pin.version,
                    requires: pin.rpp,
                    rpp: rpp.clone(),
                    hint: format!(
                        "run `rpp update {}` to select a compatible version",
                        dep.name
                    ),
                })));
            }
            pin
        }
        None => {
            let entry = registry.entry(&dep.name)?;
            let chosen = select(&entry, requested, rpp)?;
            LockedPackage {
                name: dep.name.clone(),
                requested: dep.raw.clone(),
                version: chosen.version.clone(),
                rpp: chosen.rpp.clone(),
                url: chosen.url.clone(),
                sha256: chosen.sha256.clone(),
            }
        }
    };
    let root = registry.install(&dep.name, &locked.version, &locked.url, &locked.sha256)?;
    let version = locked.version.clone();
    lock.upsert(locked);
    Ok(ResolvedPackage {
        name: dep.name.clone(),
        root,
        version: Some(version),
    })
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

/// `rpp` with pre-release and build metadata removed, for compatibility checks.
fn release_of(rpp: &Version) -> Version {
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
}
