use std::fs;
use std::path::{Path, PathBuf};

use crate::build::{BuildError, CachedEntry, GeneratedFile, ProcessedFile};

/// Writes files to the output directory.
pub struct OutputWriter {
    output_dir: PathBuf,
}

impl OutputWriter {
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }

    /// Write a processed file to output.
    pub fn write_processed(&self, file: &ProcessedFile) -> Result<(), BuildError> {
        let output_path = self.output_dir.join(&file.output_path);
        self.write_file(&output_path, &file.content)
    }

    /// Write a generated file to output.
    pub fn write_generated(&self, file: &GeneratedFile) -> Result<(), BuildError> {
        let output_path = self.output_dir.join(&file.path);
        self.write_file(&output_path, &file.content)
    }

    /// Write a cached file to output.
    pub fn write_cached(&self, cached: &CachedEntry) -> Result<(), BuildError> {
        let output_path = self.output_dir.join(&cached.output_path);
        self.write_file(&output_path, &cached.content)
    }

    fn write_file(&self, path: &Path, content: &[u8]) -> Result<(), BuildError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::FileWrite {
                path: path.to_path_buf(),
                source: e,
            })?;
        }

        fs::write(path, content).map_err(|e| BuildError::FileWrite {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Clean the output directory.
    pub fn clean(&self) -> Result<(), BuildError> {
        if self.output_dir.exists() {
            fs::remove_dir_all(&self.output_dir).map_err(|e| BuildError::FileWrite {
                path: self.output_dir.clone(),
                source: e,
            })?;
        }
        Ok(())
    }
}
