//! Durable ownership and synchronization for generated non-pack artifacts.

use std::collections::BTreeMap;
use std::io::Write;
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

/// Staged writes and durable recovery ownership for one external publication.
pub(super) struct PublicationPlan {
    next: OwnershipManifest,
    recovery: OwnershipManifest,
    stale: Vec<PathBuf>,
    writes: Vec<(PathBuf, tempfile::NamedTempFile)>,
}

impl PublicationPlan {
    pub(super) fn prepare(
        config: &Config,
        project_root: &Path,
        next_build: &Manifest,
        store: &ObjectStore,
    ) -> Result<Self> {
        let previous = OwnershipManifest::load(project_root)?;
        let next = OwnershipManifest::from_build(config, project_root, next_build)?;
        let previous_by_path = previous.by_path(config, project_root)?;
        let next_by_path = next.by_path(config, project_root)?;
        for path in next_by_path.keys() {
            if path
                .ancestors()
                .skip(1)
                .any(|parent| next_by_path.contains_key(parent))
            {
                return Err(Error::Build(format!(
                    "external output `{}` conflicts with another output's parent directory",
                    path.display()
                )));
            }
        }
        let stale = previous_by_path
            .keys()
            .filter(|path| !next_by_path.contains_key(*path))
            .cloned()
            .collect();
        let mut recovery_by_path = previous_by_path;
        recovery_by_path.extend(
            next_by_path
                .iter()
                .map(|(path, output)| (path.clone(), *output)),
        );
        let recovery = OwnershipManifest {
            version: VERSION,
            outputs: recovery_by_path.into_values().cloned().collect(),
        };
        let mut writes = Vec::new();
        for (path, output) in next_by_path {
            let needs_write = match std::fs::read(&path) {
                Ok(existing) => crate::util::hash::xxh3(&existing) != output.object,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(error) => return Err(Error::io(&path, error)),
            };
            if !needs_write {
                continue;
            }
            let bytes = store.get(output.object).ok_or_else(|| {
                Error::Build(format!("missing cache object {:#x}", output.object))
            })?;
            let parent = path.parent().expect("external paths are absolute");
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            let mut staged =
                tempfile::NamedTempFile::new_in(parent).map_err(|e| Error::io(parent, e))?;
            staged.write_all(&bytes).map_err(|e| Error::io(&path, e))?;
            staged
                .as_file()
                .sync_all()
                .map_err(|e| Error::io(&path, e))?;
            writes.push((path, staged));
        }
        Ok(Self {
            next,
            recovery,
            stale,
            writes,
        })
    }

    /// Keep both generations owned until every write and stale removal succeeds.
    pub(super) fn record_recovery(&self, project_root: &Path) -> Result<()> {
        self.recovery.save(project_root)
    }

    pub(super) fn publish(self, project_root: &Path) -> Result<ExternalChangeReport> {
        let mut changes = ExternalChangeReport::default();
        for path in self.stale {
            if remove_owned_file(&path)? {
                changes.removed.push(path);
            }
        }
        for (path, staged) in self.writes {
            staged
                .persist(&path)
                .map_err(|e| Error::io(&path, e.error))?;
            changes.written.push(path);
        }
        self.next.save(project_root)?;
        Ok(changes)
    }
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

fn external_path(config: &Config, project_root: &Path, root: &str, path: &str) -> Result<PathBuf> {
    crate::util::path::validate_relative(path).map_err(Error::Build)?;
    let root = super::boundary::external_root(config, project_root, Path::new(root))?;
    super::boundary::checked_path(&root, Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::GeneratorEntry;

    fn fixture() -> (tempfile::TempDir, Config, ObjectStore) {
        let project = tempfile::tempdir().unwrap();
        let config = Config::parse("[pack]\nname = 'test'\n", "rpp.toml").unwrap();
        let store = ObjectStore::open(project.path().join(".rpp/cache/objects")).unwrap();
        (project, config, store)
    }

    fn build_manifest(store: &ObjectStore, paths: &[&str]) -> Manifest {
        let mut manifest = Manifest::empty(0);
        manifest.generators.insert(
            "test".into(),
            GeneratorEntry {
                plugin_key: 0,
                read_set: vec![],
                mutations: paths
                    .iter()
                    .map(|path| GeneratorMutation::EmitExternal {
                        root: "generated".into(),
                        path: (*path).into(),
                        object: store.put(path.as_bytes()).unwrap(),
                    })
                    .collect(),
            },
        );
        manifest
    }

    #[test]
    fn external_staging_failure_preserves_previous_files() {
        let (project, config, store) = fixture();
        let root = project.path();
        std::fs::create_dir(root.join("generated")).unwrap();
        std::fs::write(root.join("generated/z"), "manual").unwrap();
        let manifest = build_manifest(&store, &["a.txt", "z/blocked.txt"]);
        assert!(PublicationPlan::prepare(&config, root, &manifest, &store).is_err());
        assert!(!root.join("generated/a.txt").exists());
        assert_eq!(std::fs::read(root.join("generated/z")).unwrap(), b"manual");
        assert_eq!(
            std::fs::read_dir(root.join("generated")).unwrap().count(),
            1
        );
    }

    #[test]
    fn external_second_publish_failure_retains_stale_and_new_ownership() {
        let (project, config, store) = fixture();
        let root = project.path();
        let old = build_manifest(&store, &["stale.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &old, &store).unwrap();
        plan.record_recovery(root).unwrap();
        plan.publish(root).unwrap();
        std::fs::write(root.join("generated/manual.txt"), "manual").unwrap();
        let next = build_manifest(&store, &["a.txt", "z.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &next, &store).unwrap();
        plan.record_recovery(root).unwrap();
        // Force the second rename to fail after staging and stale cleanup.
        std::fs::create_dir(root.join("generated/z.txt")).unwrap();
        assert!(plan.publish(root).is_err());
        assert!(!root.join("generated/stale.txt").exists());
        assert!(root.join("generated/a.txt").is_file());
        assert_eq!(OwnershipManifest::load(root).unwrap().outputs.len(), 3);
        std::fs::remove_dir(root.join("generated/z.txt")).unwrap();
        clean(&config, root).unwrap();
        assert!(!root.join("generated/a.txt").exists());
        assert!(root.join("generated/manual.txt").is_file());
    }

    #[test]
    fn external_ownership_persistence_failure_leaves_files_unchanged() {
        let (project, config, store) = fixture();
        let root = project.path();
        std::fs::create_dir(root.join("generated")).unwrap();
        std::fs::write(root.join("generated/a.txt"), "previous").unwrap();
        let manifest = build_manifest(&store, &["a.txt", "z.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &manifest, &store).unwrap();
        std::fs::create_dir(root.join(MANIFEST_PATH)).unwrap();
        assert!(plan.record_recovery(root).is_err());
        drop(plan);
        assert_eq!(
            std::fs::read(root.join("generated/a.txt")).unwrap(),
            b"previous"
        );
        assert!(!root.join("generated/z.txt").exists());
        assert_eq!(
            std::fs::read_dir(root.join("generated")).unwrap().count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_final_ownership_failure_keeps_recovery_manifest() {
        use std::os::unix::fs::PermissionsExt;

        let (project, config, store) = fixture();
        let root = project.path();
        let manifest = build_manifest(&store, &["a.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &manifest, &store).unwrap();
        plan.record_recovery(root).unwrap();
        let bookkeeping = root.join(".rpp");
        let permissions = std::fs::metadata(&bookkeeping).unwrap().permissions();
        std::fs::set_permissions(&bookkeeping, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Privileged users can bypass directory permissions.
        if tempfile::NamedTempFile::new_in(&bookkeeping).is_ok() {
            std::fs::set_permissions(&bookkeeping, permissions).unwrap();
            return;
        }
        let result = plan.publish(root);
        std::fs::set_permissions(&bookkeeping, permissions).unwrap();
        assert!(result.is_err());
        assert!(root.join("generated/a.txt").is_file());
        assert_eq!(OwnershipManifest::load(root).unwrap().outputs.len(), 1);
        clean(&config, root).unwrap();
        assert!(!root.join("generated/a.txt").exists());
    }
}
