# Commit 7: Build Engine Orchestrator

**Goal**: Tie all phases together into a single build engine.

## Files to Create

```
crates/rpp/src/
├── build/
│   └── engine.rs
```

## `build/engine.rs`

```rust
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
        // Phase 1: Discovery
        let discovery = DiscoveryPhase::new(self.source_dir.clone());
        let index = discovery.run(&self.cache, &self.registry)?;

        // Phase 2: Process
        let process = ProcessPhase::new(self.num_workers);
        let process_result = process.run(index, &self.registry)?;

        // Update cache with processed files
        for (i, file) in process_result.processed.iter().enumerate() {
            // Need the original FileEntry for fingerprint
            // This is simplified - real impl would track this
            self.cache.record_processed(file);
        }

        // Phase 3: Finalize
        let finalize = FinalizePhase::new(self.output_dir.clone());
        let result = finalize.run(process_result, &self.registry, &self.config)?;

        // Save cache
        self.cache.update_plugin_versions(&self.registry);
        self.cache.save(&self.cache_path)?;

        Ok(result)
    }

    /// Clean the output directory and cache.
    pub fn clean(&self) -> Result<(), BuildError> {
        let writer = OutputWriter::new(self.output_dir.clone());
        writer.clean()?;

        if self.cache_path.exists() {
            std::fs::remove_file(&self.cache_path)
                .map_err(|e| BuildError::Cache(e.to_string()))?;
        }

        Ok(())
    }

    /// Register a processor plugin.
    pub fn register_processor<P>(&mut self, plugin: P)
    where
        P: crate::plugin::ProcessorPlugin + 'static,
    {
        Arc::get_mut(&mut self.registry)
            .expect("Cannot modify registry after build starts")
            .register_processor(Arc::new(plugin));
    }

    /// Register a generator plugin.
    pub fn register_generator<G>(&mut self, plugin: G)
    where
        G: crate::plugin::GeneratorPlugin + 'static,
    {
        Arc::get_mut(&mut self.registry)
            .expect("Cannot modify registry after build starts")
            .register_generator(Arc::new(plugin));
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

    /// Set the output directory (default: source_dir/dist).
    pub fn output_dir<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.output_dir = Some(path.into());
        self
    }

    /// Set the cache directory (default: source_dir/.rpp).
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

        let output_dir = self
            .output_dir
            .unwrap_or_else(|| source_dir.join("dist"));

        let cache_dir = self
            .cache_dir
            .unwrap_or_else(|| source_dir.join(".rpp"));

        let cache_path = cache_dir.join("build.cache");

        let num_workers = self
            .num_workers
            .unwrap_or_else(|| {
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
```

## Build Pipeline Flow

```
┌──────────────────────────────────────────────────────────────────┐
│                         BuildEngine                               │
│                                                                   │
│  BuildEngine::build()                                             │
│       │                                                           │
│       ▼                                                           │
│  ┌─────────────────┐                                              │
│  │ Phase 1:        │  DiscoveryPhase::run()                       │
│  │ DISCOVERY       │  → FileIndex { entries, cached }             │
│  │                 │                                              │
│  │ • Walk source   │                                              │
│  │ • Check cache   │                                              │
│  │ • Load content  │                                              │
│  └────────┬────────┘                                              │
│           │                                                       │
│           ▼                                                       │
│  ┌─────────────────┐                                              │
│  │ Phase 2:        │  ProcessPhase::run()                         │
│  │ PROCESS         │  → ProcessPhaseResult { processed, cached }  │
│  │                 │                                              │
│  │ • Worker pool   │                                              │
│  │ • Chain procs   │                                              │
│  │ • Handle C/S/C  │                                              │
│  └────────┬────────┘                                              │
│           │                                                       │
│           ▼                                                       │
│  ┌─────────────────┐                                              │
│  │ Phase 3:        │  FinalizePhase::run()                        │
│  │ FINALIZE        │  → BuildResult                               │
│  │                 │                                              │
│  │ • Run generators│                                              │
│  │ • Write output  │                                              │
│  │ • Copy cached   │                                              │
│  └────────┬────────┘                                              │
│           │                                                       │
│           ▼                                                       │
│  ┌─────────────────┐                                              │
│  │ Save Cache      │  cache.save()                                │
│  └─────────────────┘                                              │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

## Usage Example

```rust
use rpp::build::BuildEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = BuildEngine::builder()
        .source_dir("./my_pack")
        .output_dir("./dist")
        .num_workers(4)
        .build()?;

    // Register plugins
    engine.register_processor(JsonMinifyProcessor::new());
    engine.register_generator(AtlasGenerator::new());

    // Run build
    let result = engine.build()?;

    println!(
        "Built {} files ({} cached) in {:?}",
        result.files_processed,
        result.files_cached,
        result.duration
    );

    Ok(())
}
```

## Update `build/mod.rs`

```rust
mod engine;
pub use engine::{BuildEngine, BuildEngineBuilder};
```

## Update `lib.rs`

```rust
pub mod build;
pub mod plugin;
pub mod sandbox;
pub mod worker;

// Re-export commonly used types
pub use build::{BuildEngine, BuildResult, BuildError};
pub use plugin::{Plugin, ProcessorPlugin, GeneratorPlugin, ProcessResult};
```

## Verification

```bash
cargo check -p rpp
cargo test -p rpp engine
```
