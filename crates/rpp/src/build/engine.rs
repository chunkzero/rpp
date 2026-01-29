use std::path::PathBuf;
use std::sync::Arc;

use crate::build::{BuildCache, BuildError, BuildResult};
use crate::plugin::PluginRegistry;

use super::discovery::DiscoveryPhase;
use super::finalize::FinalizePhase;
use super::output::OutputWriter;
use super::process::ProcessPhase;

/// Main build orchestrator.
pub struct BuildEngine {
    source_dir: PathBuf,
    output_dir: PathBuf,
    cache_path: PathBuf,
    registry: Arc<PluginRegistry>,
    cache: BuildCache,
    num_workers: usize,
    config: toml::Value,
}

/// Builder for constructing a BuildEngine.
pub struct BuildEngineBuilder {
    source_dir: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    num_workers: Option<usize>,
    config: Option<toml::Value>,
}

impl BuildEngine {
    /// Create a new builder.
    pub fn builder() -> BuildEngineBuilder {
        BuildEngineBuilder::new()
    }

    /// Run a full build.
    pub fn build(&mut self) -> Result<BuildResult, BuildError> {
        let start = std::time::Instant::now();

        // Phase 1: Discovery
        let discovery = DiscoveryPhase::new(self.source_dir.clone());
        let index = discovery.run(&self.cache, &self.registry)?;

        // Phase 2: Process
        let process = ProcessPhase::new(self.num_workers);
        let process_result = process.run(index, &self.registry, &self.config)?;

        // Update cache with processed files
        for file in process_result.processed.iter() {
            // Look up the original FileEntry to get the fingerprint
            if let Some(entry) = process_result.file_entries.get(&file.source_path) {
                self.cache.record(entry, file);
            }
        }

        // Phase 3: Finalize
        let finalize = FinalizePhase::new(self.output_dir.clone());
        let result = finalize.run(process_result, &self.registry, &self.config)?;

        // Save cache
        self.cache.update_plugin_versions(&self.registry);
        self.cache.save(&self.cache_path)?;

        let duration = start.elapsed();
        Ok(BuildResult {
            files_processed: result.files_processed,
            files_cached: result.files_cached,
            files_generated: result.files_generated,
            files_cancelled: result.files_cancelled,
            duration,
        })
    }

    /// Clean the output directory and cache.
    pub fn clean(&self) -> Result<(), BuildError> {
        let writer = OutputWriter::new(self.output_dir.clone());
        writer.clean()?;

        if self.cache_path.exists() {
            std::fs::remove_file(&self.cache_path).map_err(|e| BuildError::Cache(e.to_string()))?;
        }

        Ok(())
    }

    /// Register a processor plugin.
    pub fn register_processor<P>(&mut self, plugin: P) -> Result<(), BuildError>
    where
        P: crate::plugin::ProcessorPlugin + 'static,
    {
        Arc::get_mut(&mut self.registry)
            .ok_or(BuildError::RegistryLocked)?
            .register_processor(Arc::new(plugin));
        Ok(())
    }

    /// Register a generator plugin.
    pub fn register_generator<G>(&mut self, plugin: G) -> Result<(), BuildError>
    where
        G: crate::plugin::GeneratorPlugin + 'static,
    {
        Arc::get_mut(&mut self.registry)
            .ok_or(BuildError::RegistryLocked)?
            .register_generator(Arc::new(plugin));
        Ok(())
    }

    /// Get a reference to the plugin registry.
    pub fn registry(&self) -> &Arc<PluginRegistry> {
        &self.registry
    }
}

impl BuildEngineBuilder {
    fn new() -> Self {
        Self {
            source_dir: None,
            output_dir: None,
            cache_dir: None,
            num_workers: None,
            config: None,
        }
    }

    /// Set the source directory (required).
    pub fn source_dir<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.source_dir = Some(path.into());
        self
    }

    /// Set the output directory (default: .rpp/build).
    pub fn output_dir<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.output_dir = Some(path.into());
        self
    }

    /// Set the cache directory (default: .rpp/cache).
    pub fn cache_dir<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.cache_dir = Some(path.into());
        self
    }

    /// Set the number of worker threads (default: available parallelism).
    pub fn num_workers(mut self, n: usize) -> Self {
        self.num_workers = Some(n);
        self
    }

    /// Set the build configuration.
    pub fn config(mut self, config: toml::Value) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the engine.
    pub fn build(self) -> Result<BuildEngine, BuildError> {
        let source_dir = self
            .source_dir
            .ok_or_else(|| BuildError::Plugin("source_dir is required".into()))?;

        let output_dir = self.output_dir.unwrap_or_else(|| PathBuf::from(".rpp/build"));

        let cache_dir = self.cache_dir.unwrap_or_else(|| PathBuf::from(".rpp/cache"));

        let cache_path = cache_dir.join("build.cache");

        let num_workers = self.num_workers.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(4)
        });

        let config = self
            .config
            .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()));

        let cache = BuildCache::load(&cache_path);

        Ok(BuildEngine {
            source_dir,
            output_dir,
            cache_path,
            registry: Arc::new(PluginRegistry::new()),
            cache,
            num_workers,
            config,
        })
    }
}
