# Commit 4: Discovery Phase

**Goal**: Implement file discovery with cache checking.

## Files to Create

```
crates/rpp/src/
├── build/
│   ├── discovery.rs
│   └── cache.rs
```

## `build/discovery.rs`

```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use ignore::WalkBuilder;
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
            .hidden(false)                              // Include hidden files
            .git_ignore(true)                           // Respect .gitignore
            .git_global(false)                          // Don't use global gitignore
            .git_exclude(true)                          // Respect .git/info/exclude
            .add_custom_ignore_filename(".rppignore")   // Custom ignore file
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
```

## `build/cache.rs`

```rust
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use serde::{Deserialize, Serialize};

use crate::build::{BuildError, Fingerprint, ProcessedFile};
use crate::plugin::PluginRegistry;

/// Cache format version. Increment when format changes.
const CACHE_VERSION: u32 = 1;

/// Build cache for incremental builds.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildCache {
    pub version: u32,
    pub created_at: SystemTime,
    pub files: HashMap<PathBuf, CachedFile>,
    pub plugin_versions: HashMap<String, String>,
}

/// Cached information about a processed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFile {
    pub fingerprint: Fingerprint,
    pub output_path: PathBuf,
    /// Processed output content (cached to avoid reprocessing)
    pub output_content: Vec<u8>,
    pub transformations: Vec<TransformationRecord>,
    pub dependencies: Vec<PathBuf>,
}

/// Record of a transformation for cache invalidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformationRecord {
    pub processor: String,
    pub version: String,
}

impl BuildCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            created_at: SystemTime::now(),
            files: HashMap::new(),
            plugin_versions: HashMap::new(),
        }
    }

    /// Load cache from file, or return empty cache if not found/invalid.
    pub fn load(path: &Path) -> Self {
        Self::try_load(path).unwrap_or_else(|_| Self::new())
    }

    fn try_load(path: &Path) -> Result<Self, BuildError> {
        let file = fs::File::open(path)
            .map_err(|e| BuildError::Cache(e.to_string()))?;
        let reader = BufReader::new(file);

        let cache: Self = bincode::serde::decode_from_std_read(
            reader,
            bincode::config::standard(),
        ).map_err(|e| BuildError::Cache(e.to_string()))?;

        if cache.version != CACHE_VERSION {
            return Err(BuildError::Cache("Cache version mismatch".into()));
        }

        Ok(cache)
    }

    /// Save cache to file.
    pub fn save(&self, path: &Path) -> Result<(), BuildError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| BuildError::Cache(e.to_string()))?;
        }

        let file = fs::File::create(path)
            .map_err(|e| BuildError::Cache(e.to_string()))?;
        let writer = BufWriter::new(file);

        bincode::serde::encode_into_std_write(self, writer, bincode::config::standard())
            .map_err(|e| BuildError::Cache(e.to_string()))?;

        Ok(())
    }

    /// Check if cached entry is still valid.
    pub fn is_valid(
        &self,
        path: &Path,
        current_fingerprint: &Fingerprint,
        registry: &PluginRegistry,
    ) -> bool {
        let Some(cached) = self.files.get(path) else {
            return false;
        };

        // Check fingerprint (mtime, size, hash)
        if cached.fingerprint != *current_fingerprint {
            return false;
        }

        // Check if any processor versions changed
        for tx in &cached.transformations {
            match registry.plugin_version(&tx.processor) {
                Some(version) if version == tx.version => continue,
                _ => return false,
            }
        }

        // Check dependencies
        for dep in &cached.dependencies {
            if !self.files.contains_key(dep) {
                return false;
            }
        }

        true
    }

    /// Get cached file data.
    pub fn get_cached(&self, path: &Path) -> Option<&CachedFile> {
        self.files.get(path)
    }

    /// Record a processed file in the cache.
    pub fn record(&mut self, file: &crate::build::FileEntry, processed: &ProcessedFile) {
        let transformations: Vec<TransformationRecord> = processed
            .transformations
            .iter()
            .map(|t| TransformationRecord {
                processor: t.processor.clone(),
                version: t.version.clone(),
            })
            .collect();

        self.files.insert(
            file.relative_path.clone(),
            CachedFile {
                fingerprint: file.fingerprint.clone(),
                output_path: processed.output_path.clone(),
                output_content: processed.content.clone(),
                transformations,
                dependencies: Vec::new(),
            },
        );
    }

    /// Update plugin versions from registry.
    pub fn update_plugin_versions(&mut self, registry: &PluginRegistry) {
        self.plugin_versions.clear();
        for proc in registry.all_processors() {
            self.plugin_versions
                .insert(proc.name().to_string(), proc.version().to_string());
        }
        for gen in registry.generators() {
            self.plugin_versions
                .insert(gen.name().to_string(), gen.version().to_string());
        }
    }
}

impl Default for BuildCache {
    fn default() -> Self { Self::new() }
}
```

## Cache Invalidation Rules

A cached file is invalidated when:

1. **Source file changed** - mtime, size, or content hash differs
2. **Processor version changed** - any processor in the chain has a new version
3. **Dependency changed** - any file this depends on was invalidated
4. **Config changed** - (handled externally by clearing cache)

## Update `build/mod.rs`

```rust
//! Build pipeline types and orchestration.

mod cache;
mod discovery;
mod error;
mod types;

pub use cache::BuildCache;
pub use discovery::DiscoveryPhase;
pub use error::BuildError;
pub use types::{
    BuildResult, CachedEntry, FileEntry, FileIndex, Fingerprint,
    GeneratedFile, ProcessedFile, Transformation,
};
```

## Verification

```bash
cargo check -p rpp
cargo test -p rpp discovery
cargo test -p rpp cache
```
