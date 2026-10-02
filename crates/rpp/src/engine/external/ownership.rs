//! The durable `.rpp/external-outputs.bin` record of RPP-owned external outputs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cache::{GeneratorMutation, Manifest};
use crate::config::Config;
use crate::error::{Error, Result};

use super::super::boundary;

pub(super) const VERSION: u32 = 1;
pub(super) const MANIFEST_PATH: &str = ".rpp/external-outputs.bin";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OwnedOutput {
    pub(super) plugin: String,
    pub(super) root: String,
    pub(super) path: String,
    pub(super) object: u64,
}

impl OwnedOutput {
    pub(super) fn same_output(&self, other: &Self) -> bool {
        (&self.root, &self.path, &self.plugin) == (&other.root, &other.path, &other.plugin)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct OwnershipManifest {
    pub(super) version: u32,
    pub(super) outputs: Vec<OwnedOutput>,
}

impl OwnershipManifest {
    pub(super) fn new(outputs: Vec<OwnedOutput>) -> Self {
        Self {
            version: VERSION,
            outputs,
        }
    }

    pub(super) fn load(project_root: &Path) -> Result<Self> {
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

    /// The external outputs emitted by `manifest`'s generators; two plugins may not emit the
    /// same destination.
    pub(super) fn from_build(
        config: &Config,
        project_root: &Path,
        manifest: &Manifest,
    ) -> Result<Self> {
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
        Ok(Self::new(outputs))
    }

    pub(super) fn save(&self, project_root: &Path) -> Result<()> {
        let path = project_root.join(MANIFEST_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let bytes = bincode::serde::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| Error::Build(e.to_string()))?;
        crate::util::atomic::write(&path, &bytes).map_err(|e| Error::io(&path, e))
    }

    /// Owned outputs keyed by their checked absolute destination.
    pub(super) fn by_path(
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

fn external_path(config: &Config, project_root: &Path, root: &str, path: &str) -> Result<PathBuf> {
    crate::util::path::validate_relative(path).map_err(Error::Build)?;
    let root = boundary::external_root(config, project_root, Path::new(root))?;
    boundary::checked_path(&root, Path::new(path))
}
