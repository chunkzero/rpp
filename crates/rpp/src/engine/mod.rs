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

mod cache_replay;
mod discovery;
mod file_phase;
mod finalize;
mod generator;
mod keys;
mod result;
mod sync;
mod worker;

use std::collections::BTreeMap;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::cache::{GeneratorEntry, GeneratorMutation, Manifest, ObjectStore};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::{BuildStats, PluginFactory};

use self::cache_replay::materialize_generator_mutations;
use self::generator::{read_set_matches, OutputSet, RecordedMutation, RecordingHost};
use self::keys::{compile_processors, CompiledProcessor};

pub use self::result::{BuildResult, ChangeReport};

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
    pub fn build_engine(self) -> Result<Engine> {
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

        let source = self.project_root.join(&self.config.build.source);
        let output = self.project_root.join(&self.config.build.output);
        let rpp_dir = self.project_root.join(".rpp");
        let cache_dir = rpp_dir.join("cache");

        Ok(Engine {
            config: self.config,
            source,
            output,
            rpp_dir,
            cache_dir,
            factories: Arc::new(self.factories),
            compiled,
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
    Ok(())
}

/// The build engine.
pub struct Engine {
    config: Config,
    source: PathBuf,
    output: PathBuf,
    rpp_dir: PathBuf,
    cache_dir: PathBuf,
    factories: Arc<Vec<Arc<dyn PluginFactory>>>,
    compiled: Vec<CompiledProcessor>,
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

    /// The resolved source directory.
    pub fn source_dir(&self) -> &Path {
        &self.source
    }

    /// The resolved output directory.
    pub fn output_dir(&self) -> &Path {
        &self.output
    }

    /// Remove the output directory and the entire `.rpp` cache directory.
    pub fn clean(&self) -> Result<()> {
        for dir in [&self.output, &self.rpp_dir] {
            if dir.exists() {
                std::fs::remove_dir_all(dir).map_err(|e| Error::io(dir, e))?;
            }
        }
        Ok(())
    }

    /// Run a full (incremental) build.
    pub fn build(&self) -> Result<BuildResult> {
        let start = Instant::now();

        let manifest_path = self.cache_dir.join("manifest.bin");
        let store = ObjectStore::open(self.cache_dir.join("objects"))?;

        let global_key = keys::global_key(&self.config, &self.factories);
        let prev = Manifest::load(&manifest_path);
        let global_match = prev
            .as_ref()
            .map(|m| m.global_key == global_key)
            .unwrap_or(false);
        let prev = prev.filter(|_| global_match);

        let sources = discovery::discover(&self.source)?;

        let mut main_instances = self.instantiate_all()?;
        for inst in main_instances.iter_mut() {
            inst.on_build_start()?;
        }

        let mut new_manifest = Manifest::empty(global_key);
        let mut output = OutputSet {
            files: BTreeMap::new(),
        };
        let mut source_owners = BTreeMap::<String, String>::new();
        let mut output_owners = BTreeMap::<String, String>::new();

        let file_stats = file_phase::process_files(file_phase::FilePhaseCtx {
            engine: self,
            compiled: &self.compiled,
            sources,
            store: &store,
            prev: prev.as_ref(),
            new_manifest: &mut new_manifest,
            output: &mut output,
            source_owners: &mut source_owners,
            factories: &self.factories,
        })?;

        for (path, owner) in &source_owners {
            output_owners
                .entry(path.clone())
                .or_insert_with(|| owner.clone());
        }

        let generated = self.run_generators(
            &mut main_instances,
            &mut output,
            &mut output_owners,
            &mut new_manifest,
            &store,
            prev.as_ref(),
        )?;

        let changes = finalize::sync_output(&self.config, &self.output, &output, &store)?;

        let live = finalize::collect_live_objects(&new_manifest);
        store.gc(&live)?;
        new_manifest.save(&manifest_path)?;

        let stats = BuildStats {
            processed: file_stats.processed,
            cached: file_stats.cached,
            generated,
            dropped: file_stats.dropped,
        };
        finalize::finish_build(&mut main_instances, stats)?;

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

    fn run_generators(
        &self,
        instances: &mut [Box<dyn crate::model::PluginInstance>],
        output: &mut OutputSet,
        output_owners: &mut BTreeMap<String, String>,
        new_manifest: &mut Manifest,
        store: &ObjectStore,
        prev: Option<&Manifest>,
    ) -> Result<usize> {
        let mut generated = 0usize;

        for (index, factory) in self.factories.iter().enumerate() {
            if !factory.has_generator() {
                continue;
            }
            let plugin_id = factory.id().to_string();
            let plugin_key = factory.cache_key();

            let replayed = prev
                .and_then(|m| m.generators.get(&plugin_id))
                .filter(|prev_entry| prev_entry.plugin_key == plugin_key)
                .filter(|prev_entry| {
                    read_set_matches(&prev_entry.read_set, output, &self.source, store)
                })
                .map(|prev_entry| {
                    materialize_generator_mutations(
                        store,
                        &prev_entry.mutations,
                        output,
                        output_owners,
                        &plugin_id,
                    )
                    .map(|ok| (ok, prev_entry.clone()))
                })
                .transpose()?
                .and_then(|(ok, entry)| ok.then_some(entry));
            if let Some(prev_entry) = replayed {
                new_manifest.generators.insert(plugin_id, prev_entry);
                continue;
            }

            let mut host = RecordingHost::new(
                output,
                self.source.clone(),
                store,
                &plugin_id,
                output_owners,
            );
            instances[index].generate(&mut host)?;
            if !host.errors.is_empty() {
                return Err(Error::Generator {
                    plugin: plugin_id,
                    message: host.errors.join("; "),
                });
            }

            let recorded = std::mem::take(&mut host.mutations);
            let reads = host.into_reads();

            let mut mutations = Vec::with_capacity(recorded.len());
            for mutation in recorded {
                match mutation {
                    RecordedMutation::Emit { path, contents } => {
                        let object = store.put(&contents)?;
                        mutations.push(GeneratorMutation::Emit(crate::cache::OutputRef {
                            path,
                            object,
                        }));
                    }
                    RecordedMutation::EmitObject { .. } => {
                        return Err(Error::Build(
                            "internal error: fresh generator run produced a cached object ref"
                                .into(),
                        ));
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
