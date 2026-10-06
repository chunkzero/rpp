//! The incremental build engine (spec §7).
//!
//! ```no_run
//! use std::sync::Arc;
//! use rpp::config::Config;
//! use rpp::engine::Engine;
//!
//! # fn demo(config: Config) -> rpp::Result<()> {
//! let engine = Engine::builder(config)
//!     .project_root(".")
//!     .build_engine()?;
//! let result = engine.build()?;
//! println!("processed {} files in {:?}", result.processed, result.duration);
//! # Ok(())
//! # }
//! ```

mod boundary;
mod cache_replay;
mod external;
mod file_phase;
mod generator;
mod keys;
mod output;
mod output_sync;
mod result;
mod session;
mod worker;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::cache::{Manifest, ObjectStore};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::{PluginFactory, PluginInstance};
use crate::source::{self, SourceFile};
use crate::util::glob::GlobSet;

use self::keys::{compile_processors, CompiledProcessor};
use self::session::BuildSession;

pub use self::result::{BuildResult, ChangeReport, ExternalChangeReport};

/// Builder for an [`Engine`].
pub struct EngineBuilder {
    config: Config,
    project_root: PathBuf,
    factories: Vec<Arc<dyn PluginFactory>>,
}

impl EngineBuilder {
    /// Set the project root (directory containing `rpp.config.ts`). Defaults to `.`.
    pub fn project_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.project_root = root.into();
        self
    }

    /// Register a plugin factory. Order is significant (tie-break for priority).
    pub fn plugin(mut self, factory: Arc<dyn PluginFactory>) -> Self {
        self.factories.push(factory);
        self
    }

    /// Register multiple plugin factories in order.
    pub fn plugins(mut self, factories: impl IntoIterator<Item = Arc<dyn PluginFactory>>) -> Self {
        self.factories.extend(factories);
        self
    }

    /// Finalize the engine.
    pub fn build_engine(self) -> Result<Engine> {
        let Self {
            config,
            project_root,
            factories,
        } = self;
        let project_root =
            std::fs::canonicalize(&project_root).map_err(|e| Error::io(&project_root, e))?;
        let compiled = compile_processors(&factories).map_err(Error::Build)?;
        boundary::validate_layout(&config, &project_root)?;
        boundary::validate_destinations(&config, &project_root, &factories)?;

        let mut plugin_ids = BTreeSet::new();
        if let Some(factory) = factories
            .iter()
            .find(|factory| !plugin_ids.insert(factory.id()))
        {
            return Err(Error::Build(format!(
                "plugin id `{}` is configured more than once",
                factory.id()
            )));
        }
        let overrides = factories
            .iter()
            .map(|factory| {
                GlobSet::new(factory.overrides()).map_err(|message| Error::PluginLoad {
                    plugin: factory.id().to_string(),
                    message,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Engine {
            source: project_root.join(&config.build.source),
            output: project_root.join(&config.build.output),
            cache_dir: project_root.join(".rpp/cache"),
            project_root,
            config,
            factories: Arc::new(factories),
            compiled,
            overrides,
            pool: parking_lot::Mutex::new(None),
        })
    }
}

/// The build engine.
pub struct Engine {
    project_root: PathBuf,
    config: Config,
    source: PathBuf,
    output: PathBuf,
    cache_dir: PathBuf,
    factories: Arc<Vec<Arc<dyn PluginFactory>>>,
    compiled: Vec<CompiledProcessor>,
    /// Compiled `overrides` globs, parallel to `factories`.
    overrides: Vec<GlobSet>,
    /// Worker threads reused across builds (see `file_phase`).
    pool: parking_lot::Mutex<Option<worker::WorkerPool>>,
}

impl Engine {
    /// Start building an engine from a parsed [`Config`].
    pub fn builder(config: Config) -> EngineBuilder {
        EngineBuilder {
            config,
            project_root: PathBuf::from("."),
            factories: Vec::new(),
        }
    }

    /// Number of effective plugins after project/global override resolution.
    pub fn plugin_count(&self) -> usize {
        self.factories.len()
    }

    /// Whether `rel` (a forward-slash path under the source directory) is an authoring input
    /// of any plugin, and so excluded from the pack.
    pub fn is_authoring_source(&self, rel: &str) -> bool {
        self.factories
            .iter()
            .any(|factory| factory.is_authoring_source(rel))
    }

    /// Run a full (incremental) build.
    pub fn build(&self) -> Result<BuildResult> {
        self.config.validate_source(&self.project_root)?;
        let start = Instant::now();
        boundary::validate_destinations(&self.config, &self.project_root, &self.factories)?;
        external::validate_previous(&self.config, &self.project_root)?;

        let store = ObjectStore::open(self.cache_dir.join("objects"))?;
        let global_key = keys::global_key(&self.config);
        let prev =
            Manifest::load(&self.manifest_path()).filter(|prev| prev.global_key == global_key);
        let sources = self.discover_sources()?;
        let mut instances = self.instantiate_all()?;
        for instance in &mut instances {
            instance.on_build_start()?;
        }

        let manifest = Manifest::empty(global_key);
        let mut session = BuildSession::new(self, store, prev.as_ref(), manifest, &sources);
        session.insert_pack_metadata()?;
        session.process_files(sources)?;
        session.run_generators(&mut instances)?;
        let stats = session.stats.clone();
        for instance in &mut instances {
            instance.on_build_finish(&stats)?;
        }
        let output_digest = session.output.digest();
        let changes = session.finish()?;

        Ok(BuildResult {
            processed: stats.processed,
            cached: stats.cached,
            generated: stats.generated,
            dropped: stats.dropped,
            duration: start.elapsed(),
            changes,
            output_digest,
        })
    }

    /// Pack sources under the source directory, excluding plugin authoring inputs.
    fn discover_sources(&self) -> Result<Vec<SourceFile>> {
        let mut sources = source::discover(&self.source)?;
        sources.retain(|source| !self.is_authoring_source(&source.rel));
        Ok(sources)
    }

    fn instantiate_all(&self) -> Result<Vec<Box<dyn PluginInstance>>> {
        self.factories
            .iter()
            .map(|factory| factory.instantiate())
            .collect()
    }

    fn manifest_path(&self) -> PathBuf {
        self.cache_dir.join("manifest.bin")
    }

    fn worker_count(&self) -> usize {
        match self.config.build.workers {
            0 => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            n => n,
        }
    }
}

/// Remove pack output, cache state, and externally generated files owned by RPP.
///
/// This does not load or resolve plugins, so it is suitable for `rpp clean`
/// even when a plugin package is temporarily unavailable.
pub fn clean_project_artifacts(config: &Config, project_root: &Path) -> Result<()> {
    let project_root =
        std::fs::canonicalize(project_root).map_err(|e| Error::io(project_root, e))?;
    boundary::validate_layout(config, &project_root)?;
    boundary::validate_destinations(config, &project_root, &[])?;
    external::clean(config, &project_root)?;
    for dir in [
        project_root.join(&config.build.output),
        project_root.join(".rpp"),
    ] {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
    }
    Ok(())
}
