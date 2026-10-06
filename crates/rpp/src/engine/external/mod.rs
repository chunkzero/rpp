//! Durable ownership and synchronization for generated non-pack artifacts.

mod ownership;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cache::{Manifest, ObjectStore};
use crate::config::Config;
use crate::error::{Error, Result};

use self::ownership::{OwnedOutput, OwnershipManifest};
use super::result::ExternalChangeReport;

/// Staged writes and durable recovery ownership for one external publication.
pub(super) struct PublicationPlan {
    next: OwnershipManifest,
    recovery: OwnershipManifest,
    stale: Vec<PathBuf>,
    writes: Vec<StagedWrite>,
}

struct StagedWrite {
    path: PathBuf,
    file: tempfile::NamedTempFile,
    output: OwnedOutput,
    /// The destination was neither owned nor present during preparation.
    claim: bool,
}

impl PublicationPlan {
    /// Validate every destination of `next_build`'s external outputs and stage the changed
    /// ones in sibling temporary files.
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
        reject_nested_outputs(&next_by_path)?;

        let stale = previous_by_path
            .keys()
            .filter(|path| !next_by_path.contains_key(*path))
            .cloned()
            .collect();
        let mut recovery_by_path = previous_by_path.clone();
        recovery_by_path.extend(
            next_by_path
                .iter()
                .map(|(path, output)| (path.clone(), *output)),
        );
        let recovery = OwnershipManifest::new(recovery_by_path.into_values().cloned().collect());

        let mut writes = Vec::new();
        for (path, output) in next_by_path {
            let owned = previous_by_path.contains_key(&path);
            if needs_write(&path, output.object, owned)? {
                writes.push(StagedWrite::stage(path, output.clone(), !owned, store)?);
            }
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

    pub(super) fn publish(mut self, project_root: &Path) -> Result<ExternalChangeReport> {
        let mut changes = ExternalChangeReport::default();
        for path in self.stale {
            if remove_owned_file(&path)? {
                changes.removed.push(path);
            }
        }
        for write in self.writes {
            let StagedWrite {
                path,
                file,
                output,
                claim,
            } = write;
            if claim {
                if let Err(e) = file.persist_noclobber(&path) {
                    if e.error.kind() != std::io::ErrorKind::AlreadyExists || path.is_dir() {
                        return Err(Error::io(&path, e.error));
                    }
                    // The file is not ours: stop recovery cleanup from deleting it.
                    self.recovery.outputs.retain(|o| !o.same_output(&output));
                    self.recovery.save(project_root)?;
                    return Err(Error::Build(format!(
                        "refusing to overwrite unowned file {}",
                        path.display()
                    )));
                }
            } else {
                file.persist(&path).map_err(|e| Error::io(&path, e.error))?;
            }
            changes.written.push(path);
        }
        self.next.save(project_root)?;
        Ok(changes)
    }
}

impl StagedWrite {
    fn stage(path: PathBuf, output: OwnedOutput, claim: bool, store: &ObjectStore) -> Result<Self> {
        let bytes = store
            .get(output.object)
            .ok_or_else(|| Error::Build(format!("missing cache object {:#x}", output.object)))?;
        let parent = path.parent().expect("external paths are absolute");
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        let mut file =
            crate::util::atomic::staging_file(parent).map_err(|e| Error::io(parent, e))?;
        // Not fsynced: every build hashes owned destinations in `needs_write` and rewrites
        // mismatches, so a file torn by a power failure is repaired by the next build.
        file.write_all(&bytes).map_err(|e| Error::io(&path, e))?;
        Ok(Self {
            path,
            file,
            output,
            claim,
        })
    }
}

/// An output may not sit inside a directory that another output writes as a file.
fn reject_nested_outputs(outputs: &BTreeMap<PathBuf, &OwnedOutput>) -> Result<()> {
    for path in outputs.keys() {
        if path
            .ancestors()
            .skip(1)
            .any(|parent| outputs.contains_key(parent))
        {
            return Err(Error::Build(format!(
                "external output `{}` conflicts with another output's parent directory",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Whether `path` must be written to hold `object`. An existing unowned file is adopted only
/// when its bytes already match.
fn needs_write(path: &Path, object: u64, owned: bool) -> Result<bool> {
    match std::fs::read(path) {
        Ok(existing) => {
            let same = crate::util::hash::xxh3(&existing) == object;
            if !same && !owned {
                return Err(Error::Build(format!(
                    "refusing to overwrite unowned file {}",
                    path.display()
                )));
            }
            Ok(!same)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(Error::io(path, error)),
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

#[cfg(test)]
mod tests {
    use super::ownership::{MANIFEST_PATH, VERSION};
    use super::*;
    use crate::cache::{GeneratorEntry, GeneratorMutation};

    fn fixture() -> (tempfile::TempDir, Config, ObjectStore) {
        let project = tempfile::tempdir().unwrap();
        let config = Config::new("test", 34);
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
        OwnershipManifest {
            version: VERSION,
            outputs: vec![OwnedOutput {
                plugin: "test".into(),
                root: "generated".into(),
                path: "a.txt".into(),
                object: 0,
            }],
        }
        .save(root)
        .unwrap();
        let manifest = build_manifest(&store, &["a.txt", "z.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &manifest, &store).unwrap();
        std::fs::remove_file(root.join(MANIFEST_PATH)).unwrap();
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

    #[test]
    fn external_publish_refuses_file_created_after_prepare() {
        let (project, config, store) = fixture();
        let root = project.path();
        let manifest = build_manifest(&store, &["a.txt", "b.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &manifest, &store).unwrap();
        plan.record_recovery(root).unwrap();
        std::fs::write(root.join("generated/b.txt"), "manual").unwrap();
        let error = plan.publish(root).unwrap_err().to_string();
        assert!(
            error.contains("refusing to overwrite unowned file"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(root.join("generated/b.txt")).unwrap(),
            b"manual"
        );
        assert_eq!(OwnershipManifest::load(root).unwrap().outputs.len(), 1);
        clean(&config, root).unwrap();
        assert!(!root.join("generated/a.txt").exists());
        assert_eq!(
            std::fs::read(root.join("generated/b.txt")).unwrap(),
            b"manual"
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_outputs_get_the_default_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (project, config, store) = fixture();
        let root = project.path();
        let reference = root.join("reference");
        std::fs::write(&reference, "").unwrap();
        let expected = std::fs::metadata(&reference).unwrap().permissions().mode() & 0o777;
        let manifest = build_manifest(&store, &["a.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &manifest, &store).unwrap();
        plan.record_recovery(root).unwrap();
        plan.publish(root).unwrap();
        let mode = std::fs::metadata(root.join("generated/a.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, expected);
    }

    #[cfg(unix)]
    #[test]
    fn external_final_ownership_failure_keeps_recovery_manifest() {
        use std::os::unix::fs::PermissionsExt;

        let (project, config, store) = fixture();
        let root = project.path();
        let old = build_manifest(&store, &["stale.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &old, &store).unwrap();
        plan.record_recovery(root).unwrap();
        plan.publish(root).unwrap();
        let next = build_manifest(&store, &["a.txt"]);
        let plan = PublicationPlan::prepare(&config, root, &next, &store).unwrap();
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
        assert!(!root.join("generated/stale.txt").exists());
        assert!(root.join("generated/a.txt").is_file());
        assert_eq!(OwnershipManifest::load(root).unwrap().outputs.len(), 2);
        clean(&config, root).unwrap();
        assert!(!root.join("generated/a.txt").exists());
    }
}
