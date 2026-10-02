//! The durable `.rpp/external-outputs.bin` record of RPP-owned external outputs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cache::{GeneratorMutation, Manifest};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::util::versioned::{self, Versioned};

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
        // A corrupt or outdated manifest must not brick `build` or `clean`.
        // Treat it as empty: previously generated files are then left in
        // place rather than removed.
        match versioned::load(&path) {
            Ok(Some(manifest)) => Ok(manifest),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(Error::io(path, error)),
            Ok(None) => {
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
        versioned::save(&project_root.join(MANIFEST_PATH), self)
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

impl Versioned for OwnershipManifest {
    const VERSION: u32 = VERSION;

    fn version(&self) -> u32 {
        self.version
    }
}

fn external_path(config: &Config, project_root: &Path, root: &str, path: &str) -> Result<PathBuf> {
    crate::util::path::validate_relative(path).map_err(Error::Build)?;
    let root = boundary::external_root(config, project_root, Path::new(root))?;
    boundary::checked_path(&root, Path::new(path))
}
