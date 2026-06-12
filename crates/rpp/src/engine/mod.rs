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

mod discovery;
mod generator;
mod keys;
mod result;
mod sync;
mod worker;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::cache::{
    FileEntry, GeneratorEntry, GeneratorMutation, Manifest, ObjectStore, OutputRef,
};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::{BuildStats, PluginFactory};

use self::discovery::SourceFile;
use self::generator::{read_set_matches, OutputSet, RecordedMutation, RecordingHost};
use self::keys::{chain_for, chain_key, compile_processors, ChainStep};
use self::worker::{Job, JobOutcome, WorkerPool};

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
        // Validate that processor globs compile up-front for clear errors.
        compile_processors(&self.factories).map_err(Error::Build)?;
        validate_build_dirs(&self.config, &self.project_root)?;

        let mut plugin_ids = BTreeSet::new();
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

        let compiled = compile_processors(&self.factories).map_err(Error::Build)?;

        // ---- Discovery + per-file processing ----
        let sources = discovery::discover(&self.source)?;

        // Run lifecycle on_build_start on a dedicated main-thread instance set.
        let mut main_instances = self.instantiate_all()?;
        for inst in main_instances.iter_mut() {
            inst.on_build_start()?;
        }

        let mut new_manifest = Manifest::empty(global_key);
        let mut output = OutputSet {
            files: BTreeMap::new(),
        };
        let mut source_outputs = BTreeMap::<String, String>::new();

        let mut processed = 0usize;
        let mut cached = 0usize;
        let mut dropped = 0usize;

        // Partition into clean (cache hit) and dirty (needs processing).
        let mut dirty: Vec<(SourceFile, Arc<Vec<ChainStep>>, u64)> = Vec::new();

        for src in sources {
            let chain = chain_for(&compiled, &src.rel);
            let ck = chain_key(&chain);

            let prev_entry = prev.as_ref().and_then(|m| m.files.get(&src.rel));
            let clean = match prev_entry {
                Some(entry) => {
                    entry.chain_key == ck
                        && (src.matches_fast(&entry.fingerprint) || {
                            // Fast path missed; verify by content hash.
                            match src.fingerprint() {
                                Ok((fp, _)) => fp.xxh3 == entry.fingerprint.xxh3,
                                Err(_) => false,
                            }
                        })
                }
                None => false,
            };

            if clean {
                let entry = prev_entry.expect("clean implies prev entry").clone();
                // Materialize outputs from CAS into the in-memory output set.
                let mut ok = true;
                let mut materialized = Vec::with_capacity(entry.outputs.len());
                for out in &entry.outputs {
                    match store.get(out.object) {
                        Some(bytes) => {
                            materialized.push((out.path.clone(), bytes));
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    for (path, bytes) in materialized {
                        claim_source_output(&mut source_outputs, &path, &src.rel)?;
                        output.files.insert(path, bytes);
                    }
                    if entry.outputs.is_empty() {
                        dropped += 1;
                    }
                    cached += 1;
                    new_manifest.files.insert(src.rel.clone(), entry);
                    continue;
                }
                // CAS miss: fall through to reprocess.
            }

            dirty.push((src, Arc::new(chain), ck));
        }

        // Process dirty files through the worker pool.
        if !dirty.is_empty() {
            let worker_count = self.worker_count();
            let pool = WorkerPool::new(worker_count, Arc::clone(&self.factories));

            // Submit jobs (reading contents now, recording fingerprints).
            let mut pending: BTreeMap<String, (u64, crate::cache::Fingerprint)> = BTreeMap::new();
            let mut submitted = 0usize;
            for (src, chain, ck) in &dirty {
                let (fp, contents) = src.fingerprint()?;
                pending.insert(src.rel.clone(), (*ck, fp));
                pool.submit(Job {
                    rel: src.rel.clone(),
                    file: crate::model::PackFile::new(src.rel.clone(), contents),
                    chain: Arc::clone(chain),
                })?;
                submitted += 1;
            }

            let mut outcomes = Vec::with_capacity(submitted);
            for _ in 0..submitted {
                let outcome = match pool.recv() {
                    Some(r) => r?,
                    None => return Err(Error::Build("worker pool closed early".into())),
                };
                outcomes.push(outcome);
            }
            outcomes.sort_by(|a, b| outcome_rel(a).cmp(outcome_rel(b)));

            for outcome in outcomes {
                match outcome {
                    JobOutcome::Produced { rel, file } => {
                        let (ck, fp) = pending
                            .get(&rel)
                            .cloned()
                            .ok_or_else(|| Error::Build(format!("unknown result for {rel}")))?;
                        let object = store.put(&file.contents)?;
                        // The processor may have renamed the file via file.path.
                        claim_source_output(&mut source_outputs, &file.path, &rel)?;
                        output.files.insert(file.path.clone(), file.contents);
                        new_manifest.files.insert(
                            rel,
                            FileEntry {
                                fingerprint: fp,
                                chain_key: ck,
                                outputs: vec![OutputRef {
                                    path: file.path,
                                    object,
                                }],
                            },
                        );
                        processed += 1;
                    }
                    JobOutcome::Dropped { rel } => {
                        let (ck, fp) = pending
                            .get(&rel)
                            .cloned()
                            .ok_or_else(|| Error::Build(format!("unknown result for {rel}")))?;
                        new_manifest.files.insert(
                            rel,
                            FileEntry {
                                fingerprint: fp,
                                chain_key: ck,
                                outputs: Vec::new(),
                            },
                        );
                        processed += 1;
                        dropped += 1;
                    }
                }
            }

            pool.shutdown();
        }

        // ---- Generator phase (sequential, main thread) ----
        let mut generated = 0usize;
        let generators = self.run_generators(
            &mut main_instances,
            &mut output,
            &mut new_manifest,
            &store,
            prev.as_ref(),
        )?;
        generated += generators;

        // ---- Output sync ----
        let changes = self.sync_output(&output)?;

        // ---- GC + persist manifest ----
        let live = collect_live_objects(&new_manifest);
        store.gc(&live)?;
        new_manifest.save(&manifest_path)?;

        // ---- Lifecycle on_build_finish ----
        let stats = BuildStats {
            processed,
            cached,
            generated,
            dropped,
        };
        for inst in main_instances.iter_mut() {
            inst.on_build_finish(&stats)?;
        }

        Ok(BuildResult {
            processed,
            cached,
            generated,
            dropped,
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

    fn worker_count(&self) -> usize {
        match self.config.build.workers {
            0 => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            n => n,
        }
    }

    /// Run all generators, reusing cached output when their read-set is unchanged.
    fn run_generators(
        &self,
        instances: &mut [Box<dyn crate::model::PluginInstance>],
        output: &mut OutputSet,
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

            // Try the cache: replay the previous read-set.
            if let Some(prev_entry) = prev.and_then(|m| m.generators.get(&plugin_id)) {
                if prev_entry.plugin_key == plugin_key
                    && read_set_matches(&prev_entry.read_set, output, &self.source)
                {
                    // Reuse cached outputs.
                    let mut ok = true;
                    let mut materialized = Vec::new();
                    for mutation in &prev_entry.mutations {
                        match mutation {
                            GeneratorMutation::Emit(out) => match store.get(out.object) {
                                Some(bytes) => materialized.push(RecordedMutation::Emit {
                                    path: out.path.clone(),
                                    contents: bytes,
                                }),
                                None => {
                                    ok = false;
                                    break;
                                }
                            },
                            GeneratorMutation::Remove(path) => {
                                materialized.push(RecordedMutation::Remove(path.clone()));
                            }
                        }
                    }
                    if ok {
                        for mutation in materialized {
                            apply_generator_mutation(output, mutation);
                        }
                        generated += prev_entry.mutations.len();
                        new_manifest
                            .generators
                            .insert(plugin_id, prev_entry.clone());
                        continue;
                    }
                }
            }

            // Run the generator fresh.
            let mut host = RecordingHost::new(output, self.source.clone());
            instances[index].generate(&mut host)?;
            let recorded = std::mem::take(&mut host.mutations);
            let reads = host.into_reads();

            // Record ordered mutations, storing emitted bytes in the CAS.
            let mut mutations = Vec::with_capacity(recorded.len());
            for mutation in recorded {
                match mutation {
                    RecordedMutation::Emit { path, contents } => {
                        let object = store.put(&contents)?;
                        mutations.push(GeneratorMutation::Emit(OutputRef { path, object }));
                    }
                    RecordedMutation::Remove(path) => {
                        mutations.push(GeneratorMutation::Remove(path));
                    }
                }
            }
            generated += mutations.len();

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

    /// Write the in-memory output set to disk, removing stale files.
    fn sync_output(&self, output: &OutputSet) -> Result<ChangeReport> {
        std::fs::create_dir_all(&self.output).map_err(|e| Error::io(&self.output, e))?;

        let existing = sync::list_existing(&self.output)?;
        let desired: BTreeSet<String> = output.files.keys().cloned().collect();
        let release_archive = (self.config.build.squash.enabled && self.config.build.squash.zip)
            .then(|| format!("{}.zip", self.config.pack.name));

        let mut report = ChangeReport::default();

        if std::fs::symlink_metadata(&self.output)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(Error::Build(format!(
                "output directory `{}` must not be a symlink",
                self.output.display()
            )));
        }

        // Remove stale files before writing so a stale symlink cannot be used
        // as an ancestor of a desired output path.
        for rel in existing.difference(&desired) {
            if release_archive.as_deref() == Some(rel.as_str()) {
                continue;
            }
            sync::remove_file(&self.output, rel)?;
            report.removed.push(rel.clone());
        }

        // Write/overwrite desired files (only when content differs).
        for (rel, contents) in &output.files {
            crate::util::path::validate_relative(rel).map_err(Error::Build)?;
            let path = self.output.join(rel);
            if std::fs::symlink_metadata(&path)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
            }
            let needs_write = match std::fs::read(&path) {
                Ok(existing) => &existing != contents,
                Err(_) => true,
            };
            if needs_write {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                }
                std::fs::write(&path, contents).map_err(|e| Error::io(&path, e))?;
                report.written.push(rel.clone());
            }
        }

        report.written.sort();
        report.removed.sort();
        Ok(report)
    }
}

/// Compute the set of CAS objects referenced by a manifest.
fn collect_live_objects(manifest: &Manifest) -> HashSet<u64> {
    let mut live = HashSet::new();
    for entry in manifest.files.values() {
        for out in &entry.outputs {
            live.insert(out.object);
        }
    }
    for entry in manifest.generators.values() {
        for mutation in &entry.mutations {
            if let GeneratorMutation::Emit(out) = mutation {
                live.insert(out.object);
            }
        }
    }
    live
}

fn outcome_rel(outcome: &JobOutcome) -> &str {
    match outcome {
        JobOutcome::Produced { rel, .. } | JobOutcome::Dropped { rel } => rel,
    }
}

fn claim_source_output(
    owners: &mut BTreeMap<String, String>,
    output: &str,
    source: &str,
) -> Result<()> {
    crate::util::path::validate_relative(output).map_err(Error::Build)?;
    if let Some(previous) = owners.insert(output.to_string(), source.to_string()) {
        return Err(Error::Build(format!(
            "source files `{previous}` and `{source}` both produce `{output}`"
        )));
    }
    Ok(())
}

fn apply_generator_mutation(output: &mut OutputSet, mutation: RecordedMutation) {
    match mutation {
        RecordedMutation::Emit { path, contents } => {
            output.files.insert(path, contents);
        }
        RecordedMutation::Remove(path) => {
            output.files.remove(&path);
        }
    }
}
