//! Exact output-directory synchronization (spec §7).

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::{Error, Result};
use crate::util::path::to_forward_slash;

/// List existing files in `output_dir` as forward-slash relative paths.
pub(crate) fn list_existing(output_dir: &Path) -> Result<BTreeSet<String>> {
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
pub(crate) fn remove_file(output_dir: &Path, rel: &str) -> Result<()> {
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
