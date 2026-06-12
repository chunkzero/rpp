//! Copy a directory tree for release staging.

use std::path::Path;

use crate::error::{Error, Result};

/// Recursively copy files from `source` into `target`, skipping `skip` when
/// encountered (typically an existing release zip in the output directory).
///
/// # Errors
///
/// Returns an I/O error if the tree cannot be read or copied.
pub fn copy_tree(source: &Path, target: &Path, skip: &Path) -> Result<()> {
    for entry in std::fs::read_dir(source).map_err(|err| Error::io(source, err))? {
        let entry = entry.map_err(|err| Error::io(source, err))?;
        let source_path = entry.path();
        if source_path == skip {
            continue;
        }
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| Error::io(&source_path, err))?;
        if file_type.is_dir() {
            std::fs::create_dir_all(&target_path).map_err(|err| Error::io(&target_path, err))?;
            copy_tree(&source_path, &target_path, skip)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &target_path)
                .map_err(|err| Error::io(&source_path, err))?;
        }
    }
    Ok(())
}
