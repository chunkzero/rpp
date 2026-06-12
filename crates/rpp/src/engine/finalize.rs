//! Output sync, manifest persistence, and lifecycle finalization.

use crate::cache::{GeneratorMutation, Manifest, ObjectStore};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::BuildStats;
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use super::generator::{OutputContent, OutputSet};
use super::result::ChangeReport;
use super::sync;

pub(crate) fn sync_output(
    config: &Config,
    output_dir: &Path,
    output: &OutputSet,
    store: &ObjectStore,
) -> Result<ChangeReport> {
    std::fs::create_dir_all(output_dir).map_err(|e| Error::io(output_dir, e))?;

    let existing = sync::list_existing(output_dir)?;
    let desired: BTreeSet<String> = output.files.keys().cloned().collect();
    let release_archive = (config.build.squash.enabled && config.build.squash.zip)
        .then(|| format!("{}.zip", config.pack.name));

    let mut report = ChangeReport::default();

    if std::fs::symlink_metadata(output_dir)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(Error::Build(format!(
            "output directory `{}` must not be a symlink",
            output_dir.display()
        )));
    }

    for rel in existing.difference(&desired) {
        if release_archive.as_deref() == Some(rel.as_str()) {
            continue;
        }
        sync::remove_file(output_dir, rel)?;
        report.removed.push(rel.clone());
    }

    for (rel, content) in &output.files {
        crate::util::path::validate_relative(rel).map_err(Error::Build)?;
        let path = output_dir.join(rel);
        if std::fs::symlink_metadata(&path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
        }

        let needs_write = match content {
            OutputContent::Bytes(bytes) => match std::fs::read(&path) {
                Ok(existing) => existing != *bytes,
                Err(_) => true,
            },
            OutputContent::Object(key) => match std::fs::read(&path) {
                Ok(existing) => crate::util::hash::xxh3(&existing) != *key,
                Err(_) => true,
            },
        };

        if needs_write {
            match content {
                OutputContent::Bytes(bytes) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                    }
                    std::fs::write(&path, bytes).map_err(|e| Error::io(&path, e))?;
                }
                OutputContent::Object(key) => {
                    store.link_or_copy_object(*key, &path)?;
                }
            }
            report.written.push(rel.clone());
        }
    }

    report.written.sort();
    report.removed.sort();
    Ok(report)
}

pub(crate) fn collect_live_objects(manifest: &Manifest) -> HashSet<u64> {
    let mut live = HashSet::new();
    for entry in manifest.files.values() {
        for out in &entry.outputs {
            live.insert(out.object);
        }
    }
    for entry in manifest.generators.values() {
        for mutation in &entry.mutations {
            if let GeneratorMutation::Emit(out) = mutation {
                live.insert(out.object);
            }
        }
    }
    live
}

pub(crate) fn finish_build(
    instances: &mut [Box<dyn crate::model::PluginInstance>],
    stats: BuildStats,
) -> Result<()> {
    for inst in instances.iter_mut() {
        inst.on_build_finish(&stats)?;
    }
    Ok(())
}
