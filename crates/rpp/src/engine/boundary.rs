//! Filesystem boundaries shared by build and clean.

use std::path::{Component, Path, PathBuf};

use crate::config::Config;
use crate::error::{Error, Result};

pub(super) fn checked_path(project: &Path, path: &Path) -> Result<PathBuf> {
    let path = project.join(path);
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                resolved.pop();
                continue;
            }
            other => resolved.push(other.as_os_str()),
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
    checked_path(project, &config.build.source)?;
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

pub(super) fn external_root(config: &Config, project: &Path, root: &Path) -> Result<PathBuf> {
    let root = checked_path(project, root)?;
    for protected in [
        project.join(&config.build.source),
        project.join(&config.build.output),
        project.join(".rpp"),
    ] {
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
