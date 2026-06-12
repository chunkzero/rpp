//! The [`GeneratorHost`] implementation with read-set recording (spec §7).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cache::{ObjectStore, ReadKindRepr, ReadRecord};
use crate::model::{GeneratorHost, ReadKind};
use crate::util::glob;
use crate::util::hash::xxh3;
use crate::util::path::validate_relative;

/// Stored output contents: in-memory bytes or a CAS object reference.
#[derive(Clone, Debug)]
pub(crate) enum OutputContent {
    Bytes(Vec<u8>),
    Object(u64),
}

impl OutputContent {
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::Bytes(bytes)
    }

    pub(crate) fn from_object(object: u64) -> Self {
        Self::Object(object)
    }

    pub(crate) fn load_bytes(&self, store: &ObjectStore) -> Option<Vec<u8>> {
        match self {
            Self::Bytes(bytes) => Some(bytes.clone()),
            Self::Object(key) => store.get(*key),
        }
    }
}

/// The accumulated output of the processing phase, fed to generators.
///
/// Maps relative output path -> contents. Generators read from and write to it.
#[derive(Clone)]
pub(crate) struct OutputSet {
    pub(crate) files: BTreeMap<String, OutputContent>,
}

/// One generator-side output mutation.
pub(crate) enum RecordedMutation {
    Emit { path: String, contents: Vec<u8> },
    EmitObject { path: String, object: u64 },
    Remove(String),
}

/// A generator host bound to one generator run.
///
/// Records every read so the generator can be replayed for incremental
/// invalidation, and applies emits/removes to the shared output set.
pub(crate) struct RecordingHost<'a> {
    output: &'a mut OutputSet,
    read_view: OutputSet,
    source_root: PathBuf,
    store: &'a ObjectStore,
    plugin_id: String,
    output_owners: &'a mut BTreeMap<String, String>,
    reads: Vec<ReadRecord>,
    pub(crate) mutations: Vec<RecordedMutation>,
    pub(crate) errors: Vec<String>,
}

impl<'a> RecordingHost<'a> {
    /// Create a host over `output`, reading raw sources from `source_root`.
    pub(crate) fn new(
        output: &'a mut OutputSet,
        source_root: PathBuf,
        store: &'a ObjectStore,
        plugin_id: impl Into<String>,
        output_owners: &'a mut BTreeMap<String, String>,
    ) -> Self {
        Self {
            read_view: output.clone(),
            output,
            source_root,
            store,
            plugin_id: plugin_id.into(),
            output_owners,
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
        self.reads.push(ReadRecord {
            kind: ReadKindRepr::from(kind),
            key,
            hash,
        });
    }

    fn claim_output(&mut self, path: &str) -> Result<(), String> {
        validate_relative(path).map_err(|e| e.to_string())?;
        if let Some(previous) = self
            .output_owners
            .insert(path.to_string(), self.plugin_id.clone())
        {
            return Err(format!("output `{path}` already claimed by `{previous}`"));
        }
        Ok(())
    }
}

impl GeneratorHost for RecordingHost<'_> {
    fn list_files(&mut self, glob_pat: Option<&str>) -> Vec<String> {
        let mut matched: Vec<String> = self
            .read_view
            .files
            .keys()
            .filter(|p| glob_pat.map(|g| glob::matches(g, p)).unwrap_or(true))
            .cloned()
            .collect();
        matched.sort();

        let mut joined = String::new();
        for p in &matched {
            joined.push_str(p);
            joined.push('\n');
        }
        let hash = xxh3(joined.as_bytes());
        self.record(ReadKind::List, glob_pat.unwrap_or("**").to_string(), hash);

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
        if let Err(message) = validate_relative(path).map_err(|e| e.to_string()) {
            self.errors
                .push(format!("invalid emit path `{path}`: {message}"));
            return;
        }
        if let Err(message) = self.claim_output(path) {
            self.errors.push(message);
            return;
        }
        self.output.files.insert(
            path.to_string(),
            OutputContent::from_bytes(contents.clone()),
        );
        self.mutations.push(RecordedMutation::Emit {
            path: path.to_string(),
            contents,
        });
    }

    fn remove(&mut self, path: &str) {
        if let Err(message) = validate_relative(path).map_err(|e| e.to_string()) {
            self.errors
                .push(format!("invalid remove path `{path}`: {message}"));
            return;
        }
        self.output_owners.remove(path);
        self.output.files.remove(path);
        self.mutations
            .push(RecordedMutation::Remove(path.to_string()));
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
    store: &ObjectStore,
) -> bool {
    for record in reads {
        let current = match record.kind {
            ReadKindRepr::List => {
                let mut matched: Vec<&String> = output
                    .files
                    .keys()
                    .filter(|p| {
                        if record.key == "**" {
                            true
                        } else {
                            glob::matches(&record.key, p)
                        }
                    })
                    .collect();
                matched.sort();
                let mut joined = String::new();
                for p in matched {
                    joined.push_str(p);
                    joined.push('\n');
                }
                xxh3(joined.as_bytes())
            }
            ReadKindRepr::File => output
                .files
                .get(&record.key)
                .and_then(|content| content.load_bytes(store))
                .map(|c| xxh3(&c))
                .unwrap_or(0),
            ReadKindRepr::Source => confined_read(source_root, &record.key)
                .map(|c| xxh3(&c))
                .unwrap_or(0),
        };
        if current != record.hash {
            return false;
        }
    }
    true
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

pub(crate) fn apply_generator_mutation(output: &mut OutputSet, mutation: RecordedMutation) {
    match mutation {
        RecordedMutation::Emit { path, contents } => {
            output
                .files
                .insert(path, OutputContent::from_bytes(contents));
        }
        RecordedMutation::EmitObject { path, object } => {
            output
                .files
                .insert(path, OutputContent::from_object(object));
        }
        RecordedMutation::Remove(path) => {
            output.files.remove(&path);
        }
    }
}
