//! Replay of cached file and generator results into the build output.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cache::{FileEntry, GeneratorEntry, GeneratorMutation, ObjectStore};
use crate::error::{Error, Result};
use crate::util::glob::GlobSet;
use crate::util::hash::xxh3;
use crate::util::path::validate_relative;

use super::output::OutputContent;
use super::session::BuildSession;

impl BuildSession<'_> {
    /// Link a clean file's cached outputs and record its entry in the new manifest.
    ///
    /// Returns `Ok(false)`, changing nothing, when any referenced object is missing.
    pub(super) fn replay_file(&mut self, rel: &str, entry: &FileEntry) -> Result<bool> {
        let output_dir = &self.engine.output;
        let mut contents = Vec::with_capacity(entry.outputs.len());
        for out in &entry.outputs {
            match cached_content(&self.store, output_dir, &out.path, out.object) {
                Some(content) => contents.push(content),
                None => return Ok(false),
            }
        }
        for (out, content) in entry.outputs.iter().zip(contents) {
            self.output.insert_source(rel, &out.path, content)?;
        }
        self.stats.cached += 1;
        self.stats.dropped += usize::from(entry.outputs.is_empty());
        self.manifest.files.insert(rel.to_string(), entry.clone());
        Ok(true)
    }

    /// Apply a generator's cached mutations in order and record its entry in the new manifest.
    ///
    /// Returns `Ok(false)`, changing nothing, when any referenced object is missing.
    pub(super) fn replay_generator(
        &mut self,
        plugin: &str,
        overrides: &GlobSet,
        entry: &GeneratorEntry,
    ) -> Result<bool> {
        let Some(emits) = self.cached_emits(&entry.mutations) else {
            return Ok(false);
        };
        for mutation in &entry.mutations {
            let (path, content) = match mutation {
                GeneratorMutation::Emit(out) => {
                    let content = emits
                        .get(&out.path)
                        .cloned()
                        .unwrap_or(OutputContent::Object(out.object));
                    (&out.path, Some(content))
                }
                GeneratorMutation::Remove(path) => (path, None),
                GeneratorMutation::EmitExternal { .. } => continue,
            };
            validate_relative(path).map_err(Error::Build)?;
            self.output
                .apply_generator(plugin, overrides, path, content)
                .map_err(|message| Error::Generator {
                    plugin: plugin.to_string(),
                    message,
                })?;
        }
        self.manifest
            .generators
            .insert(plugin.to_string(), entry.clone());
        Ok(true)
    }

    /// Resolve the contents of every pack emit, or `None` when any object is missing.
    fn cached_emits(
        &self,
        mutations: &[GeneratorMutation],
    ) -> Option<BTreeMap<String, OutputContent>> {
        let mut emits = BTreeMap::new();
        for mutation in mutations {
            match mutation {
                GeneratorMutation::Emit(out) => {
                    let content =
                        cached_content(&self.store, &self.engine.output, &out.path, out.object)?;
                    emits.insert(out.path.clone(), content);
                }
                GeneratorMutation::EmitExternal { object, .. } if !self.store.contains(*object) => {
                    return None
                }
                _ => {}
            }
        }
        Some(emits)
    }
}

/// Resolve a cached object for `rel`: `Linked` when the output directory
/// already holds its bytes (one read of the destination, none of the CAS),
/// `Object` when the CAS holds a valid copy, `None` when neither does.
fn cached_content(
    store: &ObjectStore,
    output_dir: &Path,
    rel: &str,
    key: u64,
) -> Option<OutputContent> {
    let path = output_dir.join(rel);
    let on_disk = std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file())
        && std::fs::read(&path)
            .map(|existing| xxh3(&existing) == key)
            .unwrap_or(false);
    if on_disk {
        Some(OutputContent::Linked { key, path })
    } else if store.contains(key) {
        Some(OutputContent::Object(key))
    } else {
        None
    }
}
