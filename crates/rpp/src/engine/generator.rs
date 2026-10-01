//! The [`GeneratorHost`] implementation with read-set recording (spec §7).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cache::{ObjectStore, ReadRecord};
use crate::model::{GeneratorHost, ReadKind};
use crate::util::glob::{self, GlobSet};
use crate::util::hash::xxh3;
use crate::util::path::validate_relative;

/// Stored output contents: immutable CAS references or verified output paths.
#[derive(Clone, Debug)]
pub(crate) enum OutputContent {
    /// A CAS object that must be materialized into the output directory.
    Object(u64),
    /// A CAS object whose bytes already sit at the output path on disk.
    Linked { key: u64, path: PathBuf },
}

impl OutputContent {
    pub(crate) fn load_bytes(&self, store: &ObjectStore) -> Option<Vec<u8>> {
        match self {
            Self::Object(key) => store.get(*key),
            Self::Linked { key, path } => std::fs::read(path)
                .ok()
                .filter(|bytes| xxh3(bytes) == *key)
                .or_else(|| store.get(*key)),
        }
    }

    /// xxh3 of the contents; CAS keys are content hashes, so object entries
    /// need no read.
    fn content_hash(&self, store: &ObjectStore) -> Option<u64> {
        match self {
            Self::Object(key) => store.contains(*key).then_some(*key),
            Self::Linked { key, .. } => Some(*key),
        }
    }
}

/// The accumulated output of the processing phase, fed to generators.
///
/// Maps relative output path -> contents. Generators read from and write to it.
/// Every path in `files` has an entry in `owners`.
#[derive(Clone, Default)]
pub(crate) struct OutputSet {
    pub(crate) files: BTreeMap<String, OutputContent>,
    pub(crate) owners: BTreeMap<String, Owner>,
}

/// Who produced an output path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The processor chain of this source file (relative to the source directory).
    Source(String),
    /// A generator plugin.
    Plugin(String),
}

impl Owner {
    fn describe(&self) -> String {
        match self {
            Self::Source(rel) => format!("source `{rel}`"),
            Self::Plugin(id) => format!("plugin `{id}`"),
        }
    }
}

/// One generator-side output mutation.
pub(crate) enum RecordedMutation {
    Emit {
        path: String,
        object: u64,
    },
    EmitExternal {
        root: String,
        path: String,
        contents: Vec<u8>,
    },
    Remove(String),
}

/// Why an output path could not be claimed.
pub(crate) enum ClaimError {
    InvalidPath(String),
    Taken { previous: Owner },
}

/// Record `owner` as the producer of `path`.
pub(crate) fn claim_output(
    owners: &mut BTreeMap<String, Owner>,
    path: &str,
    owner: Owner,
) -> Result<(), ClaimError> {
    validate_relative(path).map_err(ClaimError::InvalidPath)?;
    match owners.insert(path.to_string(), owner) {
        Some(previous) => Err(ClaimError::Taken { previous }),
        None => Ok(()),
    }
}

/// Apply one generator emit (`Some`) or removal (`None`) of `path` for `plugin`.
///
/// A path owned by another source or plugin may only be changed when `overrides` matches it;
/// ownership then moves to `plugin` on emit and is cleared on removal.
pub(crate) fn apply_mutation(
    output: &mut OutputSet,
    plugin: &str,
    overrides: &GlobSet,
    path: &str,
    content: Option<OutputContent>,
) -> Result<(), String> {
    if let Some(owner) = output.owners.get(path) {
        let own = matches!(owner, Owner::Plugin(id) if id == plugin);
        if !own && !overrides.is_match(path) {
            let verb = if content.is_some() { "emit" } else { "remove" };
            return Err(format!(
                "cannot {verb} `{path}`: owned by {}; add a matching glob to `overrides` in the \
                 plugin manifest",
                owner.describe()
            ));
        }
    }
    match content {
        Some(content) => {
            output.files.insert(path.to_string(), content);
            output
                .owners
                .insert(path.to_string(), Owner::Plugin(plugin.to_string()));
        }
        None => {
            output.files.remove(path);
            output.owners.remove(path);
        }
    }
    Ok(())
}

/// A generator host bound to one generator run.
///
/// Records every read so the generator can be replayed for incremental
/// invalidation, and applies emits/removes to the shared output set.
pub(crate) struct RecordingHost<'a> {
    output: &'a mut OutputSet,
    plugin: &'a str,
    overrides: &'a GlobSet,
    read_view: OutputSet,
    source_root: PathBuf,
    source_files: Vec<String>,
    store: &'a ObjectStore,
    external_roots: BTreeMap<String, PathBuf>,
    reads: Vec<ReadRecord>,
    pub(crate) mutations: Vec<RecordedMutation>,
    pub(crate) errors: Vec<String>,
}

impl<'a> RecordingHost<'a> {
    /// Create a host over `output`, reading raw sources from `source_root`.
    pub(crate) fn new(
        output: &'a mut OutputSet,
        plugin: &'a str,
        overrides: &'a GlobSet,
        source_root: PathBuf,
        source_files: Vec<String>,
        store: &'a ObjectStore,
        external_roots: BTreeMap<String, PathBuf>,
    ) -> Self {
        Self {
            read_view: output.clone(),
            output,
            plugin,
            overrides,
            source_root,
            source_files,
            store,
            external_roots,
            reads: Vec::new(),
            mutations: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Consume the host and return its recorded read-set.
    pub(crate) fn into_reads(self) -> Vec<ReadRecord> {
        self.reads
    }

    fn record(&mut self, kind: ReadKind, key: String, hash: u64) {
        self.reads.push(ReadRecord { kind, key, hash });
    }
}

impl GeneratorHost for RecordingHost<'_> {
    fn list_files(&mut self, glob_pat: Option<&str>) -> Vec<String> {
        let matched = match matching_paths(self.read_view.files.keys(), glob_pat) {
            Ok(matched) => matched,
            Err(message) => {
                self.errors.push(message);
                Vec::new()
            }
        };
        let hash = hash_paths(&matched);
        self.record(ReadKind::List, glob_pat.unwrap_or("**").to_string(), hash);

        matched
    }

    fn list_source_files(&mut self, glob_pat: Option<&str>) -> Vec<String> {
        let matched = match matching_paths(self.source_files.iter(), glob_pat) {
            Ok(matched) => matched,
            Err(message) => {
                self.errors.push(message);
                Vec::new()
            }
        };
        let hash = hash_paths(&matched);
        self.record(
            ReadKind::SourceList,
            glob_pat.unwrap_or("**").to_string(),
            hash,
        );
        matched
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        if validate_relative(path).is_err() {
            return None;
        }
        let contents = self
            .read_view
            .files
            .get(path)
            .and_then(|content| content.load_bytes(self.store));
        let hash = contents.as_ref().map(|c| xxh3(c)).unwrap_or(0);
        self.record(ReadKind::File, path.to_string(), hash);
        contents
    }

    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        let contents = confined_read(&self.source_root, path);
        let hash = contents.as_ref().map(|c| xxh3(c)).unwrap_or(0);
        self.record(ReadKind::Source, path.to_string(), hash);
        contents
    }

    fn emit(&mut self, path: &str, contents: Vec<u8>) {
        if let Err(message) = validate_relative(path) {
            self.errors
                .push(format!("invalid emit path `{path}`: {message}"));
            return;
        }
        let object = match self.store.put(&contents) {
            Ok(object) => object,
            Err(error) => {
                self.errors
                    .push(format!("cannot store emit `{path}`: {error}"));
                return;
            }
        };
        let content = OutputContent::Object(object);
        if let Err(message) = apply_mutation(
            self.output,
            self.plugin,
            self.overrides,
            path,
            Some(content),
        ) {
            self.errors.push(message);
            return;
        }
        self.mutations.push(RecordedMutation::Emit {
            path: path.to_string(),
            object,
        });
    }

    fn remove(&mut self, path: &str) {
        if let Err(message) = validate_relative(path).map_err(|e| e.to_string()) {
            self.errors
                .push(format!("invalid remove path `{path}`: {message}"));
            return;
        }
        if let Err(message) = apply_mutation(self.output, self.plugin, self.overrides, path, None) {
            self.errors.push(message);
            return;
        }
        self.mutations
            .push(RecordedMutation::Remove(path.to_string()));
    }

    fn emit_output(&mut self, root: &str, path: &str, contents: Vec<u8>) {
        let Some(root_path) = self.external_roots.get(root) else {
            self.errors
                .push(format!("unknown output root `{root}` for `{path}`"));
            return;
        };
        if let Err(message) = validate_relative(path).map_err(|e| e.to_string()) {
            self.errors
                .push(format!("invalid external output path `{path}`: {message}"));
            return;
        }
        self.mutations.push(RecordedMutation::EmitExternal {
            root: root_path.to_string_lossy().replace('\\', "/"),
            path: path.to_string(),
            contents,
        });
    }
}

/// Replay a recorded read-set against the current output/source state.
///
/// Returns `true` if every read still hashes to the same value (so the
/// generator's output is still valid).
pub(crate) fn read_set_matches(
    reads: &[ReadRecord],
    output: &OutputSet,
    source_root: &std::path::Path,
    source_files: &[String],
    store: &ObjectStore,
) -> bool {
    for record in reads {
        let current = match record.kind {
            ReadKind::List => match matching_paths(output.files.keys(), Some(&record.key)) {
                Ok(matched) => hash_paths(&matched),
                Err(_) => return false,
            },
            ReadKind::SourceList => match matching_paths(source_files.iter(), Some(&record.key)) {
                Ok(matched) => hash_paths(&matched),
                Err(_) => return false,
            },
            ReadKind::File => output
                .files
                .get(&record.key)
                .and_then(|content| content.content_hash(store))
                .unwrap_or(0),
            ReadKind::Source => confined_read(source_root, &record.key)
                .map(|c| xxh3(&c))
                .unwrap_or(0),
        };
        if current != record.hash {
            return false;
        }
    }
    true
}

/// Sorted paths matching `glob_pat`; `None` and `"**"` match everything.
/// Used both when recording a list read and when replaying it.
fn matching_paths<'a>(
    paths: impl Iterator<Item = &'a String>,
    glob_pat: Option<&str>,
) -> Result<Vec<String>, String> {
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

fn hash_paths<P: AsRef<str>>(paths: &[P]) -> u64 {
    let mut joined = String::new();
    for path in paths {
        joined.push_str(path.as_ref());
        joined.push('\n');
    }
    xxh3(joined.as_bytes())
}

fn confined_read(root: &std::path::Path, path: &str) -> Option<Vec<u8>> {
    validate_relative(path).ok()?;
    let canonical_root = root.canonicalize().ok()?;
    let canonical = canonical_root.join(path).canonicalize().ok()?;
    if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
        return None;
    }
    std::fs::read(canonical).ok()
}
