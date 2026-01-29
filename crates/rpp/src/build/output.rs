use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::build::{BuildError, CachedEntry, GeneratedFile, ProcessedFile};
use zip::write::{SimpleFileOptions, ZipWriter};
use zip::CompressionMethod;

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

    /// Create a zip archive of the output directory.
    pub fn create_zip_archive(&self, archive_path: &Path) -> Result<(), BuildError> {
        // Ensure parent directory exists
        if let Some(parent) = archive_path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::FileWrite {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        // Create the zip file
        let file = File::create(archive_path).map_err(|e| BuildError::FileWrite {
            path: archive_path.to_path_buf(),
            source: e,
        })?;

        let mut zip = ZipWriter::new(BufWriter::new(file));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);

        // Walk through the output directory and add all files
        self.add_directory_to_zip(&mut zip, &self.output_dir, &self.output_dir, &options)?;

        zip.finish().map_err(|e| BuildError::FileWrite {
            path: archive_path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::Other, e),
        })?;

        Ok(())
    }

    fn add_directory_to_zip<W: Write + std::io::Seek>(
        &self,
        zip: &mut ZipWriter<W>,
        dir: &Path,
        base_dir: &Path,
        options: &SimpleFileOptions,
    ) -> Result<(), BuildError> {
        let entries = fs::read_dir(dir).map_err(|e| BuildError::FileRead {
            path: dir.to_path_buf(),
            source: e,
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| BuildError::FileRead {
                path: dir.to_path_buf(),
                source: e,
            })?;

            let path = entry.path();
            let name = path
                .strip_prefix(base_dir)
                .map_err(|_| BuildError::Plugin("Invalid path in output directory".into()))?;

            // Skip the zip file itself if it's in the output directory
            if path.extension().and_then(|s| s.to_str()) == Some("zip") {
                continue;
            }

            if path.is_file() {
                let name_str = name
                    .to_str()
                    .ok_or_else(|| BuildError::Plugin("Invalid UTF-8 in file path".into()))?;

                zip.start_file(name_str, *options)
                    .map_err(|e| BuildError::FileWrite {
                        path: path.clone(),
                        source: std::io::Error::new(std::io::ErrorKind::Other, e),
                    })?;

                // Stream file contents instead of reading entire file into memory
                let mut file = File::open(&path).map_err(|e| BuildError::FileRead {
                    path: path.clone(),
                    source: e,
                })?;

                std::io::copy(&mut file, zip).map_err(|e| BuildError::FileWrite {
                    path: path.clone(),
                    source: e,
                })?;
            } else if path.is_dir() {
                self.add_directory_to_zip(zip, &path, base_dir, options)?;
            }
        }

        Ok(())
    }
}
