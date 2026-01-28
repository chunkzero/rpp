use crate::build::BuildError;
use std::fs;
use std::path::{Path, PathBuf};

/// Sandboxed file API with path validation.
pub struct FileApi {
    source_root: PathBuf,
    output_root: PathBuf,
}

impl FileApi {
    pub fn new(source_root: PathBuf, output_root: PathBuf) -> Self {
        Self {
            source_root,
            output_root,
        }
    }

    /// Read a file from the source directory (read-only).
    pub fn read_source(&self, path: &str) -> Result<Vec<u8>, BuildError> {
        let full_path = self.validate_source_path(path)?;
        fs::read(&full_path).map_err(|e| BuildError::FileRead {
            path: full_path,
            source: e,
        })
    }

    /// Read a file from the output directory.
    pub fn read_output(&self, path: &str) -> Result<Vec<u8>, BuildError> {
        let full_path = self.validate_output_path(path)?;
        fs::read(&full_path).map_err(|e| BuildError::FileRead {
            path: full_path,
            source: e,
        })
    }

    /// Write a file to the output directory.
    pub fn write_file(&self, path: &str, content: &[u8]) -> Result<(), BuildError> {
        let full_path = self.validate_output_path(path)?;

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::FileWrite {
                path: full_path.clone(),
                source: e,
            })?;
        }

        fs::write(&full_path, content).map_err(|e| BuildError::FileWrite {
            path: full_path,
            source: e,
        })
    }

    /// Delete a file from the output directory.
    pub fn delete_file(&self, path: &str) -> Result<(), BuildError> {
        let full_path = self.validate_output_path(path)?;
        if full_path.exists() {
            fs::remove_file(&full_path).map_err(|e| BuildError::FileWrite {
                path: full_path,
                source: e,
            })?;
        }
        Ok(())
    }

    /// Check if a file exists in source.
    pub fn exists_source(&self, path: &str) -> bool {
        self.validate_source_path(path)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Check if a file exists in output.
    pub fn exists_output(&self, path: &str) -> bool {
        self.validate_output_path(path)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// List files in a source directory.
    pub fn list_source_dir(&self, path: &str) -> Result<Vec<String>, BuildError> {
        let full_path = self.validate_source_path(path)?;
        self.list_dir_impl(&full_path)
    }

    /// List files in an output directory.
    pub fn list_output_dir(&self, path: &str) -> Result<Vec<String>, BuildError> {
        let full_path = self.validate_output_path(path)?;
        self.list_dir_impl(&full_path)
    }

    fn list_dir_impl(&self, path: &Path) -> Result<Vec<String>, BuildError> {
        let entries = fs::read_dir(path).map_err(|e| BuildError::FileRead {
            path: path.to_path_buf(),
            source: e,
        })?;

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| BuildError::FileRead {
                path: path.to_path_buf(),
                source: e,
            })?;
            if let Some(name) = entry.file_name().to_str() {
                files.push(name.to_string());
            }
        }
        Ok(files)
    }

    fn validate_source_path(&self, path: &str) -> Result<PathBuf, BuildError> {
        self.validate_path_within(&self.source_root, path)
    }

    fn validate_output_path(&self, path: &str) -> Result<PathBuf, BuildError> {
        self.validate_path_within(&self.output_root, path)
    }

    /// Validate path is within root and doesn't escape via traversal.
    fn validate_path_within(&self, root: &Path, path: &str) -> Result<PathBuf, BuildError> {
        let requested = PathBuf::from(path);

        // Reject absolute paths
        if requested.is_absolute() {
            return Err(BuildError::Plugin(format!(
                "Absolute paths not allowed: {}",
                path
            )));
        }

        let full_path = root.join(&requested);

        // Check for directory traversal
        for component in requested.components() {
            use std::path::Component;
            match component {
                Component::ParentDir => {
                    return Err(BuildError::Plugin(format!(
                        "Path traversal not allowed: {}",
                        path
                    )));
                }
                Component::Prefix(_) => {
                    return Err(BuildError::Plugin(format!(
                        "Invalid path component: {}",
                        path
                    )));
                }
                _ => {}
            }
        }

        // If file exists, verify canonical path is within root
        if full_path.exists() {
            let canonical = full_path.canonicalize().map_err(|e| BuildError::FileRead {
                path: full_path.clone(),
                source: e,
            })?;
            let canonical_root = root.canonicalize().map_err(|e| BuildError::FileRead {
                path: root.to_path_buf(),
                source: e,
            })?;

            if !canonical.starts_with(&canonical_root) {
                return Err(BuildError::Plugin(format!(
                    "Path escapes sandbox: {}",
                    path
                )));
            }
        }

        Ok(full_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_path_traversal_blocked() {
        let source = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let api = FileApi::new(source.path().to_path_buf(), output.path().to_path_buf());

        // Should fail
        assert!(api.validate_source_path("../etc/passwd").is_err());
        assert!(api.validate_source_path("foo/../../bar").is_err());
        assert!(api.validate_source_path("/etc/passwd").is_err());

        // Should succeed
        assert!(api.validate_source_path("foo/bar.txt").is_ok());
    }
}
