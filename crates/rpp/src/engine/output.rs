//! The in-memory pack output and its per-path ownership.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cache::ObjectStore;
use crate::error::{Error, Result};
use crate::util::glob::GlobSet;
use crate::util::hash::{xxh3, HashWriter};
use crate::util::path::validate_relative;

/// Stored output contents: immutable CAS references or verified output paths.
#[derive(Clone, Debug)]
pub(super) enum OutputContent {
    /// A CAS object that must be materialized into the output directory.
    Object(u64),
    /// A CAS object whose bytes already sit at the output path on disk.
    Linked { key: u64, path: PathBuf },
}

impl OutputContent {
    pub(super) fn load_bytes(&self, store: &ObjectStore) -> Option<Vec<u8>> {
        match self {
            Self::Object(key) => store.get(*key),
            Self::Linked { key, path } => std::fs::read(path)
                .ok()
                .filter(|bytes| xxh3(bytes) == *key)
                .or_else(|| store.get(*key)),
        }
    }

    /// The CAS key of the contents.
    pub(super) fn key(&self) -> u64 {
        match self {
            Self::Object(key) | Self::Linked { key, .. } => *key,
        }
    }

    /// xxh3 of the contents; CAS keys are content hashes, so object entries
    /// need no read.
    pub(super) fn content_hash(&self, store: &ObjectStore) -> Option<u64> {
        match self {
            Self::Object(key) => store.contains(*key).then_some(*key),
            Self::Linked { key, .. } => Some(*key),
        }
    }
}

/// Who produced an output path.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Owner {
    /// The processor chain of this source file (relative to the source directory).
    Source(String),
    /// A generator plugin.
    Plugin(String),
    /// Generated from `rpp.config.ts`.
    Config,
}

impl Owner {
    fn describe(&self) -> String {
        match self {
            Self::Source(rel) => format!("source `{rel}`"),
            Self::Plugin(id) => format!("plugin `{id}`"),
            Self::Config => "rpp.config.ts".into(),
        }
    }
}

/// The accumulated pack output: relative output path -> contents, plus the owner of every path.
#[derive(Default)]
pub(super) struct OutputSet {
    files: BTreeMap<String, OutputContent>,
    owners: BTreeMap<String, Owner>,
}

impl OutputSet {
    pub(super) fn files(&self) -> &BTreeMap<String, OutputContent> {
        &self.files
    }

    /// xxh3 over every path and its content key, in path order.
    pub(super) fn digest(&self) -> u64 {
        let mut writer = HashWriter::new();
        for (path, content) in &self.files {
            writer.write_str(path);
            writer.write_u64(content.key());
        }
        writer.finish()
    }

    /// Add `path` as generated from the config, before any source or generator output.
    pub(super) fn insert_config(&mut self, path: &str, content: OutputContent) {
        self.owners.insert(path.to_string(), Owner::Config);
        self.files.insert(path.to_string(), content);
    }

    /// Add `path` as an output of the processor chain of `source`.
    ///
    /// Each path may be produced by only one source file.
    pub(super) fn insert_source(
        &mut self,
        source: &str,
        path: &str,
        content: OutputContent,
    ) -> Result<()> {
        validate_relative(path).map_err(Error::Build)?;
        let owner = Owner::Source(source.to_string());
        if let Some(previous) = self.owners.insert(path.to_string(), owner) {
            return Err(Error::Build(format!(
                "source `{source}` produces `{path}`, which {} already produces",
                previous.describe()
            )));
        }
        self.files.insert(path.to_string(), content);
        Ok(())
    }

    /// Apply one generator emit (`Some`) or removal (`None`) of `path` for `plugin`.
    ///
    /// A path owned by another source or plugin may only be changed when `overrides` matches it;
    /// ownership then moves to `plugin` on emit and is cleared on removal.
    pub(super) fn apply_generator(
        &mut self,
        plugin: &str,
        overrides: &GlobSet,
        path: &str,
        content: Option<OutputContent>,
    ) -> std::result::Result<(), String> {
        if let Some(owner) = self.owners.get(path) {
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
                self.files.insert(path.to_string(), content);
                self.owners
                    .insert(path.to_string(), Owner::Plugin(plugin.to_string()));
            }
            None => {
                self.files.remove(path);
                self.owners.remove(path);
            }
        }
        Ok(())
    }
}
