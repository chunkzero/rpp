//! Filesystem boundaries shared by build and clean.

use std::path::{Component, Path, PathBuf};

use crate::config::Config;
use crate::error::{Error, Result};

/// Lexically join `path` onto `base`, rejecting symlinks in components below `base`.
pub(super) fn checked_path(base: &Path, path: &Path) -> Result<PathBuf> {
    let mut resolved = base.to_path_buf();
    for component in path.components() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                resolved.pop();
                continue;
            }
            other => resolved.push(other.as_os_str()),
        }
        if resolved == base || !resolved.starts_with(base) {
            continue;
        }
        match std::fs::symlink_metadata(&resolved) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::Build(format!(
                    "filesystem boundary `{}` must not contain symlinks",
                    resolved.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(&resolved, error)),
        }
    }
    Ok(resolved)
}

pub(super) fn validate_project(config: &Config, project: &Path) -> Result<()> {
    checked_path(project, &config.build.output)?;
    checked_path(project, Path::new(".rpp/cache/objects"))?;
    checked_path(project, Path::new(".rpp/cache/manifest.bin"))?;
    checked_path(project, Path::new(".rpp/external-outputs.bin"))?;
    for plugin in &config.plugins {
        for root in plugin.outputs.values() {
            external_root(config, project, root)?;
        }
    }
    Ok(())
}

/// Resolve an external root. Roots inside the project must not pass through
/// symlinks; roots outside it are resolved through their existing ancestors so
/// a symlinked parent cannot alias a protected directory.
pub(super) fn external_root(config: &Config, project: &Path, root: &Path) -> Result<PathBuf> {
    let root = checked_path(project, root)?;
    let root = if root.starts_with(project) {
        root
    } else {
        resolve_existing(&root)?
    };
    for protected in [
        project.join(&config.build.source),
        project.join(&config.build.output),
        project.join(".rpp"),
    ] {
        let protected = std::fs::canonicalize(&protected).unwrap_or(protected);
        if root.starts_with(&protected) || protected.starts_with(&root) {
            return Err(Error::Build(format!(
                "external output root `{}` overlaps protected directory `{}`",
                root.display(),
                protected.display()
            )));
        }
    }
    Ok(root)
}

fn resolve_existing(path: &Path) -> Result<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = existing.file_name() else {
                    break;
                };
                missing.push(name.to_os_string());
                existing.pop();
            }
            Err(error) => return Err(Error::io(&existing, error)),
        }
    }
    let mut resolved = std::fs::canonicalize(&existing).map_err(|e| Error::io(&existing, e))?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}
