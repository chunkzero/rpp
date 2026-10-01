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
pub(crate) mod discovery;
mod external;
mod file_phase;
mod finalize;
mod generator;
mod keys;
mod result;
mod worker;

use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::cache::{GeneratorEntry, GeneratorMutation, Manifest, ObjectStore};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::{BuildStats, PluginFactory};
use crate::util::glob::GlobSet;

use self::cache_replay::materialize_generator_mutations;
use self::generator::{read_set_matches, OutputSet, RecordedMutation, RecordingHost};
use self::keys::{compile_processors, CompiledProcessor};

pub use self::result::{BuildResult, ChangeReport, ExternalChangeReport};

/// Builder for an [`Engine`].
pub struct EngineBuilder {
    config: Config,
    project_root: PathBuf,
    factories: Vec<Arc<dyn PluginFactory>>,
}

impl EngineBuilder {
    /// Set the project root (directory containing `rpp.toml`). Defaults to `.`.
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
    pub fn build_engine(mut self) -> Result<Engine> {
        self.project_root = std::fs::canonicalize(&self.project_root)
            .map_err(|e| Error::io(&self.project_root, e))?;
        let compiled = compile_processors(&self.factories).map_err(Error::Build)?;
        validate_build_dirs(&self.config, &self.project_root)?;

        let mut plugin_ids = std::collections::BTreeSet::new();
        for factory in &self.factories {
            if !plugin_ids.insert(factory.id()) {
                return Err(Error::Build(format!(
                    "plugin id `{}` is configured more than once",
                    factory.id()
                )));
            }
        }

        let overrides = self
            .factories
            .iter()
            .map(|factory| {
                GlobSet::new(factory.overrides()).map_err(|message| Error::Generator {
                    plugin: factory.id().to_string(),
                    message,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let source = self.project_root.join(&self.config.build.source);
        let output = self.project_root.join(&self.config.build.output);
        let cache_dir = self.project_root.join(".rpp/cache");

        for factory in &self.factories {
            for root in factory.output_roots().values() {
                boundary::external_root(&self.config, &self.project_root, root)?;
            }
        }

        Ok(Engine {
            project_root: self.project_root,
            config: self.config,
            source,
            output,
            cache_dir,
            factories: Arc::new(self.factories),
            compiled,
            overrides,
            pool: parking_lot::Mutex::new(None),
        })
    }
}

fn validate_build_dirs(config: &Config, project_root: &Path) -> Result<()> {
    let source = &config.build.source;
    let output = &config.build.output;
    for (label, path) in [("source", source), ("output", output)] {
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(Error::Config {
                path: project_root.join("rpp.toml"),
                message: format!("`build.{label}` must be a normalized project-relative path"),
            });
        }
        if path.starts_with(".rpp") {
            return Err(Error::Config {
                path: project_root.join("rpp.toml"),
                message: format!("`build.{label}` must not be inside `.rpp`"),
            });
        }
    }
    if source == output || source.starts_with(output) || output.starts_with(source) {
        return Err(Error::Config {
            path: project_root.join("rpp.toml"),
            message: "`build.source` and `build.output` must be separate directories".into(),
        });
    }
    boundary::validate_project(config, project_root)
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
    pub(crate) pool: parking_lot::Mutex<Option<worker::WorkerPool>>,
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

    /// Remove the output directory and the entire `.rpp` cache directory.
    pub fn clean(&self) -> Result<()> {
        clean_project_artifacts(&self.config, &self.project_root)
    }

    /// Run a full (incremental) build.
    pub fn build(&self) -> Result<BuildResult> {
        self.config.validate_source(&self.project_root)?;
        let start = Instant::now();
        validate_build_dirs(&self.config, &self.project_root)?;
        for factory in self.factories.iter() {
            for root in factory.output_roots().values() {
                boundary::external_root(&self.config, &self.project_root, root)?;
            }
        }
        external::validate_previous(&self.config, &self.project_root)?;

        let manifest_path = self.cache_dir.join("manifest.bin");
        let store = ObjectStore::open(self.cache_dir.join("objects"))?;

        let global_key = keys::global_key(&self.config);
        let prev = Manifest::load(&manifest_path);
        let global_match = prev
            .as_ref()
            .map(|m| m.global_key == global_key)
            .unwrap_or(false);
        let prev = prev.filter(|_| global_match);

        let mut sources = discovery::discover(&self.source)?;
        sources.retain(|source| {
            !self
                .factories
                .iter()
                .any(|factory| factory.is_authoring_source(&source.rel))
        });
        let source_files = sources
            .iter()
            .map(|source| source.rel.clone())
            .collect::<Vec<_>>();

        let mut main_instances = self.instantiate_all()?;
        for inst in main_instances.iter_mut() {
            inst.on_build_start()?;
        }

        let mut new_manifest = Manifest::empty(global_key);
        let mut output = OutputSet::default();

        let file_stats = file_phase::process_files(file_phase::FilePhaseCtx {
            engine: self,
            compiled: &self.compiled,
            sources,
            store: &store,
            prev: prev.as_ref(),
            new_manifest: &mut new_manifest,
            output: &mut output,
            factories: &self.factories,
        })?;

        let generated = self.run_generators(GeneratorPhaseCtx {
            instances: &mut main_instances,
            output: &mut output,
            new_manifest: &mut new_manifest,
            store: &store,
            prev: prev.as_ref(),
            source_files: &source_files,
        })?;

        let stats = BuildStats {
            processed: file_stats.processed,
            cached: file_stats.cached,
            generated,
            dropped: file_stats.dropped,
        };
        finalize::finish_build(&mut main_instances, stats)?;

        validate_build_dirs(&self.config, &self.project_root)?;
        let external = external::PublicationPlan::prepare(
            &self.config,
            &self.project_root,
            &new_manifest,
            &store,
        )?;
        external.record_recovery(&self.project_root)?;
        let mut changes = finalize::sync_output(&self.config, &self.output, &output, &store)?;
        changes.external = external.publish(&self.project_root)?;

        new_manifest.save(&manifest_path)?;
        let live = finalize::collect_live_objects(&new_manifest);
        store.gc(&live)?;

        Ok(BuildResult {
            processed: file_stats.processed,
            cached: file_stats.cached,
            generated,
            dropped: file_stats.dropped,
            duration: start.elapsed(),
            changes,
        })
    }

    fn instantiate_all(&self) -> Result<Vec<Box<dyn crate::model::PluginInstance>>> {
        let mut out = Vec::with_capacity(self.factories.len());
        for factory in self.factories.iter() {
            out.push(factory.instantiate()?);
        }
        Ok(out)
    }

    pub(crate) fn worker_count(&self) -> usize {
        match self.config.build.workers {
            0 => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            n => n,
        }
    }

    fn run_generators(&self, ctx: GeneratorPhaseCtx<'_>) -> Result<usize> {
        let GeneratorPhaseCtx {
            instances,
            output,
            new_manifest,
            store,
            prev,
            source_files,
        } = ctx;
        let mut generated = 0usize;

        for (index, factory) in self.factories.iter().enumerate() {
            if !factory.has_generator() {
                continue;
            }
            let plugin_id = factory.id().to_string();
            let overrides = &self.overrides[index];
            let plugin_key = factory.cache_key();

            let replayable = prev
                .and_then(|m| m.generators.get(&plugin_id))
                .filter(|entry| {
                    factory.cacheable_generator()
                        && entry.plugin_key == plugin_key
                        && read_set_matches(
                            &entry.read_set,
                            output,
                            &self.source,
                            source_files,
                            store,
                        )
                });
            if let Some(prev_entry) = replayable {
                let materialized = materialize_generator_mutations(
                    store,
                    &self.output,
                    &prev_entry.mutations,
                    output,
                    &plugin_id,
                    overrides,
                )?;
                if materialized {
                    new_manifest
                        .generators
                        .insert(plugin_id, prev_entry.clone());
                    continue;
                }
            }

            let mut host = RecordingHost::new(
                output,
                &plugin_id,
                overrides,
                self.source.clone(),
                source_files.to_vec(),
                store,
                factory.output_roots(),
            );
            instances[index].generate(&mut host)?;
            if !host.errors.is_empty() {
                return Err(Error::Generator {
                    plugin: factory.id().to_string(),
                    message: host.errors.join("; "),
                });
            }

            let recorded = std::mem::take(&mut host.mutations);
            let reads = host.into_reads();

            let mut mutations = Vec::with_capacity(recorded.len());
            for mutation in recorded {
                match mutation {
                    RecordedMutation::Emit { path, object } => {
                        mutations.push(GeneratorMutation::Emit(crate::cache::OutputRef {
                            path,
                            object,
                        }));
                    }
                    RecordedMutation::EmitExternal {
                        root,
                        path,
                        contents,
                    } => {
                        let object = store.put(&contents)?;
                        mutations.push(GeneratorMutation::EmitExternal { root, path, object });
                    }
                    RecordedMutation::Remove(path) => {
                        mutations.push(GeneratorMutation::Remove(path));
                    }
                }
            }
            generated += 1;

            new_manifest.generators.insert(
                plugin_id,
                GeneratorEntry {
                    plugin_key,
                    read_set: reads,
                    mutations,
                },
            );
        }

        Ok(generated)
    }
}

struct GeneratorPhaseCtx<'a> {
    instances: &'a mut [Box<dyn crate::model::PluginInstance>],
    output: &'a mut OutputSet,
    new_manifest: &'a mut Manifest,
    store: &'a ObjectStore,
    prev: Option<&'a Manifest>,
    source_files: &'a [String],
}

/// Remove pack output, cache state, and externally generated files owned by RPP.
///
/// This does not load or resolve plugins, so it is suitable for `rpp clean`
/// even when a plugin package is temporarily unavailable.
pub fn clean_project_artifacts(config: &Config, project_root: &Path) -> Result<()> {
    let project_root =
        std::fs::canonicalize(project_root).map_err(|e| Error::io(project_root, e))?;
    validate_build_dirs(config, &project_root)?;
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
