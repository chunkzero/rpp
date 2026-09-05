//! Durable ownership and synchronization for generated non-pack artifacts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cache::{GeneratorMutation, Manifest, ObjectStore};
use crate::config::Config;
use crate::error::{Error, Result};

use super::result::ExternalChangeReport;

const VERSION: u32 = 1;
const MANIFEST_PATH: &str = ".rpp/external-outputs.bin";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OwnedOutput {
    plugin: String,
    root: String,
    path: String,
    object: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OwnershipManifest {
    version: u32,
    outputs: Vec<OwnedOutput>,
}

impl OwnershipManifest {
    fn load(project_root: &Path) -> Result<Self> {
        let path = project_root.join(MANIFEST_PATH);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(Error::io(path, error)),
        };
        // A corrupt or outdated manifest must not brick `build` or `clean`.
        // Treat it as empty: previously generated files are then left in
        // place rather than removed.
        let config = bincode::config::standard();
        match bincode::serde::decode_from_slice::<Self, _>(&bytes, config) {
            Ok((manifest, _)) if manifest.version == VERSION => Ok(manifest),
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    "ignoring unreadable external-output ownership manifest {}",
                    path.display()
                );
                Ok(Self::default())
            }
        }
    }

    fn from_build(config: &Config, project_root: &Path, manifest: &Manifest) -> Result<Self> {
        let mut claimed = BTreeMap::<PathBuf, String>::new();
        let mut outputs = Vec::new();
        for (plugin, generator) in &manifest.generators {
            for mutation in &generator.mutations {
                let GeneratorMutation::EmitExternal { root, path, object } = mutation else {
                    continue;
                };
                let absolute = external_path(config, project_root, root, path)?;
                if let Some(previous) = claimed.insert(absolute.clone(), plugin.clone()) {
                    return Err(Error::Build(format!(
                        "plugins `{previous}` and `{plugin}` both emit external output `{}`",
                        absolute.display()
                    )));
                }
                outputs.push(OwnedOutput {
                    plugin: plugin.clone(),
                    root: root.clone(),
                    path: path.clone(),
                    object: *object,
                });
            }
        }
        outputs.sort_by(|a, b| (&a.root, &a.path, &a.plugin).cmp(&(&b.root, &b.path, &b.plugin)));
        Ok(Self {
            version: VERSION,
            outputs,
        })
    }

    fn save(&self, project_root: &Path) -> Result<()> {
        let path = project_root.join(MANIFEST_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let bytes = bincode::serde::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| Error::Build(e.to_string()))?;
        crate::util::atomic::write(&path, &bytes).map_err(|e| Error::io(&path, e))
    }

    fn by_path(
        &self,
        config: &Config,
        project_root: &Path,
    ) -> Result<BTreeMap<PathBuf, &OwnedOutput>> {
        self.outputs
            .iter()
            .map(|output| {
                Ok((
                    external_path(config, project_root, &output.root, &output.path)?,
                    output,
                ))
            })
            .collect()
    }
}

pub(crate) fn sync(
    config: &Config,
    project_root: &Path,
    next_build: &Manifest,
    store: &ObjectStore,
) -> Result<ExternalChangeReport> {
    let previous = OwnershipManifest::load(project_root)?;
    let next = OwnershipManifest::from_build(config, project_root, next_build)?;
    let previous_by_path = previous.by_path(config, project_root)?;
    let next_by_path = next.by_path(config, project_root)?;
    let mut changes = ExternalChangeReport::default();

    for path in previous_by_path.keys() {
        if next_by_path.contains_key(path) {
            continue;
        }
        if remove_owned_file(path)? {
            changes.removed.push(path.clone());
        }
    }

    for (path, output) in next_by_path {
        let needs_write = match std::fs::read(&path) {
            Ok(existing) => crate::util::hash::xxh3(&existing) != output.object,
            Err(_) => true,
        };
        if needs_write {
            store.copy_object(output.object, &path)?;
            changes.written.push(path);
        }
    }

    changes.written.sort();
    changes.removed.sort();
    next.save(project_root)?;
    Ok(changes)
}

pub(crate) fn clean(config: &Config, project_root: &Path) -> Result<()> {
    let ownership = OwnershipManifest::load(project_root)?;
    for path in ownership.by_path(config, project_root)?.into_keys() {
        remove_owned_file(&path)?;
    }
    Ok(())
}

fn remove_owned_file(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io(path, error)),
    }
}

pub(super) fn validate_previous(config: &Config, project_root: &Path) -> Result<()> {
    OwnershipManifest::load(project_root)?.by_path(config, project_root)?;
    Ok(())
}

pub(super) fn validate_next(
    config: &Config,
    project_root: &Path,
    manifest: &Manifest,
) -> Result<()> {
    OwnershipManifest::from_build(config, project_root, manifest)?;
    validate_previous(config, project_root)
}

fn external_path(config: &Config, project_root: &Path, root: &str, path: &str) -> Result<PathBuf> {
    crate::util::path::validate_relative(path).map_err(Error::Build)?;
    let root = super::boundary::external_root(config, project_root, Path::new(root))?;
    super::boundary::checked_path(&root, Path::new(path))
}
