use ignore::WalkBuilder;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use twox_hash::XxHash3_64;

use crate::build::{BuildError, CachedEntry, FileEntry, FileIndex, Fingerprint};
use crate::plugin::PluginRegistry;

use super::cache::BuildCache;

/// Discovery phase: walks the source directory and determines what needs processing.
pub struct DiscoveryPhase {
    source_dir: PathBuf,
}

impl DiscoveryPhase {
    pub fn new(source_dir: PathBuf) -> Self {
        Self { source_dir }
    }

    /// Run discovery, returning an index of files to process and files to copy from cache.
    pub fn run(
        &self,
        cache: &BuildCache,
        registry: &PluginRegistry,
    ) -> Result<FileIndex, BuildError> {
        let mut index = FileIndex::default();

        let walker = WalkBuilder::new(&self.source_dir)
            .hidden(false) // Include hidden files
            .git_ignore(true) // Respect .gitignore
            .git_global(false) // Don't use global gitignore
            .git_exclude(true) // Respect .git/info/exclude
            .add_custom_ignore_filename(".rppignore") // Custom ignore file
            .build();

        for entry in walker {
            let entry = entry.map_err(|e| BuildError::Discovery(e.to_string()))?;

            // Skip directories
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(true) {
                continue;
            }

            let path = entry.path();
            let relative_path = path
                .strip_prefix(&self.source_dir)
                .unwrap_or(path)
                .to_path_buf();

            // Compute fingerprint
            let fingerprint = self.compute_fingerprint(path)?;

            // Check if we can use cached version
            if cache.is_valid(&relative_path, &fingerprint, registry) {
                if let Some(cached_file) = cache.get_cached(&relative_path) {
                    index.cached.push(CachedEntry {
                        source_path: path.to_path_buf(),
                        output_path: cached_file.output_path.clone(),
                        content: cached_file.output_content.clone(),
                    });
                    continue;
                }
            }

            // Need to process this file - load content
            let content = fs::read(path).map_err(|e| BuildError::FileRead {
                path: path.to_path_buf(),
                source: e,
            })?;

            index.entries.push(FileEntry {
                source_path: path.to_path_buf(),
                relative_path,
                fingerprint,
                content,
            });
        }

        Ok(index)
    }

    fn compute_fingerprint(&self, path: &Path) -> Result<Fingerprint, BuildError> {
        let metadata = fs::metadata(path).map_err(|e| BuildError::FileRead {
            path: path.to_path_buf(),
            source: e,
        })?;

        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let size = metadata.len();

        // Compute content hash
        let content = fs::read(path).map_err(|e| BuildError::FileRead {
            path: path.to_path_buf(),
            source: e,
        })?;
        let hash = XxHash3_64::oneshot(&content);

        Ok(Fingerprint { mtime, size, hash })
    }
}
