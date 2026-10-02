//! The generator phase and its read-set recording [`GeneratorHost`] (spec §7).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cache::{
    GeneratorEntry, GeneratorMutation, ObjectStore, OutputRef, ReadKind, ReadRecord,
};
use crate::error::{Error, Result};
use crate::model::{GeneratorHost, PluginFactory, PluginInstance};
use crate::util::glob::{self, GlobSet};
use crate::util::hash::xxh3;
use crate::util::path::validate_relative;

use super::output::{OutputContent, OutputSet};
use super::session::BuildSession;

impl BuildSession<'_> {
    /// Run or replay every generator in plugin order.
    pub(super) fn run_generators(
        &mut self,
        instances: &mut [Box<dyn PluginInstance>],
    ) -> Result<()> {
        let engine = self.engine;
        for (index, factory) in engine.factories.iter().enumerate() {
            if !factory.has_generator() {
                continue;
            }
            let overrides = &engine.overrides[index];
            let plugin_key = factory.cache_key();
            if self.try_replay_generator(factory.as_ref(), overrides, plugin_key)? {
                continue;
            }
            let entry = self.record_generator(
                factory.as_ref(),
                overrides,
                plugin_key,
                instances[index].as_mut(),
            )?;
            self.stats.generated += 1;
            self.manifest
                .generators
                .insert(factory.id().to_string(), entry);
        }
        Ok(())
    }

    /// Replay the previous run of `factory`'s generator when its key and read-set still match.
    fn try_replay_generator(
        &mut self,
        factory: &dyn PluginFactory,
        overrides: &GlobSet,
        plugin_key: u64,
    ) -> Result<bool> {
        let replayable = self
            .prev
            .and_then(|manifest| manifest.generators.get(factory.id()))
            .filter(|entry| {
                factory.cacheable_generator()
                    && entry.plugin_key == plugin_key
                    && self.read_set_matches(&entry.read_set)
            });
        match replayable {
            Some(entry) => self.replay_generator(factory.id(), overrides, entry),
            None => Ok(false),
        }
    }

    fn record_generator(
        &mut self,
        factory: &dyn PluginFactory,
        overrides: &GlobSet,
        plugin_key: u64,
        instance: &mut dyn PluginInstance,
    ) -> Result<GeneratorEntry> {
        let mut host = RecordingHost {
            read_view: self.output.files().clone(),
            output: &mut self.output,
            plugin: factory.id(),
            overrides,
            source_root: &self.engine.source,
            source_files: &self.source_files,
            store: &self.store,
            external_roots: factory.output_roots(),
            reads: Vec::new(),
            mutations: Vec::new(),
            errors: Vec::new(),
        };
        instance.generate(&mut host)?;
        host.finish(plugin_key)
    }

    /// Whether every recorded read still hashes to the same value against the current
    /// output and sources, so the generator's previous mutations are still valid.
    fn read_set_matches(&self, reads: &[ReadRecord]) -> bool {
        reads.iter().all(|record| {
            let current = match record.kind {
                ReadKind::List => matching_paths(self.output.files().keys(), Some(&record.key))
                    .map(|matched| hash_paths(&matched)),
                ReadKind::SourceList => matching_paths(self.source_files.iter(), Some(&record.key))
                    .map(|matched| hash_paths(&matched)),
                ReadKind::File => Ok(self
                    .output
                    .files()
                    .get(&record.key)
                    .and_then(|content| content.content_hash(&self.store))
                    .unwrap_or(0)),
                ReadKind::Source => Ok(hash_contents(
                    confined_read(&self.engine.source, &record.key).as_deref(),
                )),
            };
            current == Ok(record.hash)
        })
    }
}

/// A generator host bound to one generator run.
///
/// Records every read so the generator can be replayed for incremental
/// invalidation, and applies emits/removes to the shared output set.
struct RecordingHost<'a> {
    output: &'a mut OutputSet,
    /// The output as it was before this generator ran; reads never see its own emits.
    read_view: BTreeMap<String, OutputContent>,
    plugin: &'a str,
    overrides: &'a GlobSet,
    source_root: &'a Path,
    source_files: &'a [String],
    store: &'a ObjectStore,
    external_roots: BTreeMap<String, PathBuf>,
    reads: Vec<ReadRecord>,
    mutations: Vec<GeneratorMutation>,
    errors: Vec<String>,
}

impl RecordingHost<'_> {
    /// The recorded cache entry, or the generator error if any host call failed.
    fn finish(self, plugin_key: u64) -> Result<GeneratorEntry> {
        if !self.errors.is_empty() {
            return Err(Error::Generator {
                plugin: self.plugin.to_string(),
                message: self.errors.join("; "),
            });
        }
        Ok(GeneratorEntry {
            plugin_key,
            read_set: self.reads,
            mutations: self.mutations,
        })
    }

    fn record(&mut self, kind: ReadKind, key: String, hash: u64) {
        self.reads.push(ReadRecord { kind, key, hash });
    }

    /// Record a list read; an invalid glob is a generator error and lists nothing.
    fn record_list(
        &mut self,
        kind: ReadKind,
        matched: std::result::Result<Vec<String>, String>,
        glob_pat: Option<&str>,
    ) -> Vec<String> {
        let matched = matched.unwrap_or_else(|message| {
            self.errors.push(message);
            Vec::new()
        });
        self.record(
            kind,
            glob_pat.unwrap_or("**").to_string(),
            hash_paths(&matched),
        );
        matched
    }

    fn store_bytes(&mut self, what: &str, path: &str, contents: &[u8]) -> Option<u64> {
        self.store
            .put(contents)
            .map_err(|error| {
                self.errors
                    .push(format!("cannot store {what} `{path}`: {error}"))
            })
            .ok()
    }
}

impl GeneratorHost for RecordingHost<'_> {
    fn list_files(&mut self, glob_pat: Option<&str>) -> Vec<String> {
        let matched = matching_paths(self.read_view.keys(), glob_pat);
        self.record_list(ReadKind::List, matched, glob_pat)
    }

    fn list_source_files(&mut self, glob_pat: Option<&str>) -> Vec<String> {
        let matched = matching_paths(self.source_files.iter(), glob_pat);
        self.record_list(ReadKind::SourceList, matched, glob_pat)
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        if validate_relative(path).is_err() {
            return None;
        }
        let contents = self
            .read_view
            .get(path)
            .and_then(|content| content.load_bytes(self.store));
        self.record(
            ReadKind::File,
            path.to_string(),
            hash_contents(contents.as_deref()),
        );
        contents
    }

    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        let contents = confined_read(self.source_root, path);
        self.record(
            ReadKind::Source,
            path.to_string(),
            hash_contents(contents.as_deref()),
        );
        contents
    }

    fn emit(&mut self, path: &str, contents: Vec<u8>) {
        if let Err(message) = validate_relative(path) {
            self.errors
                .push(format!("invalid emit path `{path}`: {message}"));
            return;
        }
        let Some(object) = self.store_bytes("emit", path, &contents) else {
            return;
        };
        let content = Some(OutputContent::Object(object));
        if let Err(message) =
            self.output
                .apply_generator(self.plugin, self.overrides, path, content)
        {
            self.errors.push(message);
            return;
        }
        self.mutations.push(GeneratorMutation::Emit(OutputRef {
            path: path.to_string(),
            object,
        }));
    }

    fn remove(&mut self, path: &str) {
        if let Err(message) = validate_relative(path) {
            self.errors
                .push(format!("invalid remove path `{path}`: {message}"));
            return;
        }
        if let Err(message) = self
            .output
            .apply_generator(self.plugin, self.overrides, path, None)
        {
            self.errors.push(message);
            return;
        }
        self.mutations
            .push(GeneratorMutation::Remove(path.to_string()));
    }

    fn emit_output(&mut self, root: &str, path: &str, contents: Vec<u8>) {
        let Some(root_path) = self.external_roots.get(root) else {
            self.errors
                .push(format!("unknown output root `{root}` for `{path}`"));
            return;
        };
        let root = root_path.to_string_lossy().replace('\\', "/");
        if let Err(message) = validate_relative(path) {
            self.errors
                .push(format!("invalid external output path `{path}`: {message}"));
            return;
        }
        let Some(object) = self.store_bytes("external output", path, &contents) else {
            return;
        };
        self.mutations.push(GeneratorMutation::EmitExternal {
            root,
            path: path.to_string(),
            object,
        });
    }
}

/// Sorted paths matching `glob_pat`; `None` and `"**"` match everything.
/// Used both when recording a list read and when replaying it.
fn matching_paths<'a>(
    paths: impl Iterator<Item = &'a String>,
    glob_pat: Option<&str>,
) -> std::result::Result<Vec<String>, String> {
    let pattern = match glob_pat {
        None | Some("**") => None,
        Some(glob_pat) => Some(glob::compile(glob_pat)?),
    };
    let mut matched = paths
        .filter(|path| pattern.as_ref().is_none_or(|pattern| pattern.matches(path)))
        .cloned()
        .collect::<Vec<_>>();
    matched.sort();
    Ok(matched)
}

fn hash_paths(paths: &[String]) -> u64 {
    let mut joined = String::new();
    for path in paths {
        joined.push_str(path);
        joined.push('\n');
    }
    xxh3(joined.as_bytes())
}

/// Hash of read contents; a missing file hashes to `0`.
fn hash_contents(contents: Option<&[u8]>) -> u64 {
    contents.map(xxh3).unwrap_or(0)
}

fn confined_read(root: &Path, path: &str) -> Option<Vec<u8>> {
    validate_relative(path).ok()?;
    let canonical_root = root.canonicalize().ok()?;
    let canonical = canonical_root.join(path).canonicalize().ok()?;
    if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
        return None;
    }
    std::fs::read(canonical).ok()
}
