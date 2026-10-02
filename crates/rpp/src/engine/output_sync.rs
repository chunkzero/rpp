//! Synchronization of the pack output directory with the build output.

use std::collections::BTreeSet;
use std::path::Path;

use crate::cache::ObjectStore;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::util::hash::xxh3;
use crate::util::path::{to_forward_slash, validate_relative};

use super::boundary::checked_path;
use super::output::{OutputContent, OutputSet};
use super::result::ChangeReport;

/// Make `output_dir` hold exactly `output`, keeping the squash release archive.
pub(super) fn sync_output(
    config: &Config,
    output_dir: &Path,
    output: &OutputSet,
    store: &ObjectStore,
) -> Result<ChangeReport> {
    checked_path(output_dir, Path::new("."))?;
    for rel in output.files().keys() {
        validate_relative(rel).map_err(Error::Build)?;
        if let Some(parent) = Path::new(rel).parent() {
            checked_path(output_dir, parent)?;
        }
    }
    std::fs::create_dir_all(output_dir).map_err(|e| Error::io(output_dir, e))?;

    let existing = list_existing(output_dir)?;
    let release_archive = (config.build.squash.enabled && config.build.squash.zip)
        .then(|| format!("{}.zip", config.pack.name));
    if std::fs::symlink_metadata(output_dir).is_ok_and(|metadata| metadata.is_symlink()) {
        return Err(Error::Build(format!(
            "output directory `{}` must not be a symlink",
            output_dir.display()
        )));
    }

    let mut report = ChangeReport::default();
    let stale = existing.iter().filter(|rel| {
        !output.files().contains_key(*rel) && release_archive.as_deref() != Some(rel.as_str())
    });
    for rel in stale {
        remove_file(output_dir, rel)?;
        report.removed.push(rel.clone());
    }
    for (rel, content) in output.files() {
        if write_output(output_dir, rel, content, store)? {
            report.written.push(rel.clone());
        }
    }

    report.written.sort();
    report.removed.sort();
    Ok(report)
}

/// Write one output unless the destination already holds its bytes. Returns whether it wrote.
fn write_output(
    output_dir: &Path,
    rel: &str,
    content: &OutputContent,
    store: &ObjectStore,
) -> Result<bool> {
    let path = output_dir.join(rel);
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_symlink()) {
        std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
    }
    let key = match content {
        // Verified against the destination during cache replay.
        OutputContent::Linked { .. } => return Ok(false),
        OutputContent::Object(key) => *key,
    };
    if std::fs::read(&path).is_ok_and(|existing| xxh3(&existing) == key) {
        return Ok(false);
    }
    store.copy_object(key, &path)?;
    Ok(true)
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
