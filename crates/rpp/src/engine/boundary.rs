//! Filesystem boundaries shared by build and clean.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::PluginFactory;

/// Lexically join `path` onto `base`, rejecting symlinks in components below `base`.
pub(super) fn checked_path(base: &Path, path: &Path) -> Result<PathBuf> {
    BoundaryChecker::new(base).check(path)
}

/// Checks many paths below one base, examining each shared ancestor once.
///
/// Only use it while nothing below `base` is being mutated.
pub(super) struct BoundaryChecker<'a> {
    base: &'a Path,
    checked: HashSet<PathBuf>,
}

impl<'a> BoundaryChecker<'a> {
    pub(super) fn new(base: &'a Path) -> Self {
        Self {
            base,
            checked: HashSet::new(),
        }
    }

    /// Like [`checked_path`], skipping components already checked by this checker.
    pub(super) fn check(&mut self, path: &Path) -> Result<PathBuf> {
        let base = self.base;
        // Every checked path's ancestors are checked too, so a known directory needs no walk.
        let joined = base.join(path);
        if self.checked.contains(&joined) {
            return Ok(joined);
        }
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
            if resolved == base || !resolved.starts_with(base) || self.checked.contains(&resolved) {
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
            self.checked.insert(resolved.clone());
        }
        Ok(resolved)
    }
}

/// Check that `build.source` and `build.output` are separate, normalized project-relative
/// directories outside `.rpp`.
pub(super) fn validate_layout(config: &Config, project: &Path) -> Result<()> {
    let invalid = |message: String| Error::Config {
        path: project.join("rpp.config.ts"),
        message,
    };
    let (source, output) = (&config.build.source, &config.build.output);
    for (label, path) in [("source", source), ("output", output)] {
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(invalid(format!(
                "`build.{label}` must be a normalized project-relative path"
            )));
        }
        if path.starts_with(".rpp") {
            return Err(invalid(format!(
                "`build.{label}` must not be inside `.rpp`"
            )));
        }
    }
    if source == output || source.starts_with(output) || output.starts_with(source) {
        return Err(invalid(
            "`build.source` and `build.output` must be separate directories".into(),
        ));
    }
    Ok(())
}

/// Check every write destination against the filesystem: pack output, bookkeeping files, and
/// the external roots declared in the config or by `factories`.
///
/// Runs before each mutation phase, since the filesystem may change between them.
pub(super) fn validate_destinations(
    config: &Config,
    project: &Path,
    factories: &[Arc<dyn PluginFactory>],
) -> Result<()> {
    checked_path(project, &config.build.output)?;
    checked_path(project, Path::new(".rpp/cache/objects"))?;
    checked_path(project, Path::new(".rpp/cache/manifest.bin"))?;
    checked_path(project, Path::new(".rpp/external-outputs.bin"))?;
    let configured = config
        .plugins
        .iter()
        .flat_map(|plugin| plugin.outputs.values().cloned());
    let declared = factories
        .iter()
        .flat_map(|factory| factory.output_roots().into_values());
    for root in configured.chain(declared) {
        external_root(config, project, &root)?;
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
