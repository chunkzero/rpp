use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
        let file = fs::File::open(path).map_err(|e| BuildError::Cache(e.to_string()))?;
        let mut reader = BufReader::new(file);

        let cache: Self =
            bincode::serde::decode_from_std_read(&mut reader, bincode::config::standard())
                .map_err(|e| BuildError::Cache(e.to_string()))?;

        if cache.version != CACHE_VERSION {
            return Err(BuildError::Cache("Cache version mismatch".into()));
        }

        Ok(cache)
    }

    /// Save cache to file.
    pub fn save(&self, path: &Path) -> Result<(), BuildError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::Cache(e.to_string()))?;
        }

        let file = fs::File::create(path).map_err(|e| BuildError::Cache(e.to_string()))?;
        let mut writer = BufWriter::new(file);

        bincode::serde::encode_into_std_write(self, &mut writer, bincode::config::standard())
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
    fn default() -> Self {
        Self::new()
    }
}
