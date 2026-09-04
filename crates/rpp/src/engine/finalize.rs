//! Output sync, manifest persistence, and lifecycle finalization.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::cache::{GeneratorMutation, Manifest, ObjectStore};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::BuildStats;
use crate::util::path::to_forward_slash;

use super::generator::{OutputContent, OutputSet};
use super::result::ChangeReport;

pub(crate) fn sync_output(
    config: &Config,
    output_dir: &Path,
    output: &OutputSet,
    store: &ObjectStore,
) -> Result<ChangeReport> {
    std::fs::create_dir_all(output_dir).map_err(|e| Error::io(output_dir, e))?;

    let existing = list_existing(output_dir)?;
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
        remove_file(output_dir, rel)?;
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
                Ok(existing) => existing != **bytes,
                Err(_) => true,
            },
            OutputContent::Object(key) => match std::fs::read(&path) {
                Ok(existing) => crate::util::hash::xxh3(&existing) != *key,
                Err(_) => true,
            },
            // Verified against the destination during cache replay.
            OutputContent::Linked(_) => false,
        };

        if needs_write {
            match content {
                OutputContent::Bytes(bytes) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                    }
                    crate::util::atomic::write(&path, bytes.as_slice())
                        .map_err(|e| Error::io(&path, e))?;
                }
                OutputContent::Object(key) | OutputContent::Linked(key) => {
                    store.copy_object(*key, &path)?;
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
            match mutation {
                GeneratorMutation::Emit(out) => {
                    live.insert(out.object);
                }
                GeneratorMutation::EmitExternal { object, .. } => {
                    live.insert(*object);
                }
                GeneratorMutation::Remove(_) => {}
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

/// List existing files in `output_dir` as forward-slash relative paths.
fn list_existing(output_dir: &Path) -> Result<BTreeSet<String>> {
    let mut set = BTreeSet::new();
    if !output_dir.exists() {
        return Ok(set);
    }
    walk(output_dir, output_dir, &mut set)?;
    Ok(set)
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<String>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let ft = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if ft.is_dir() {
            walk(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.insert(to_forward_slash(rel));
        }
    }
    Ok(())
}

/// Remove `rel` from `output_dir` and prune now-empty parent directories.
fn remove_file(output_dir: &Path, rel: &str) -> Result<()> {
    let path = output_dir.join(rel);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
    }
    // Prune empty parents up to (but not including) the output root.
    let mut parent = path.parent().map(Path::to_path_buf);
    while let Some(dir) = parent {
        if dir == output_dir || !dir.starts_with(output_dir) {
            break;
        }
        let is_empty = std::fs::read_dir(&dir)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if is_empty && std::fs::remove_dir(&dir).is_ok() {
            parent = dir.parent().map(Path::to_path_buf);
        } else {
            break;
        }
    }
    Ok(())
}
