//! CAS materialization helpers for cache hits.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cache::{FileEntry, GeneratorMutation, ObjectStore};
use crate::error::{Error, Result};
use crate::util::hash::xxh3;

use super::generator::{claim_output, ClaimError, OutputContent, OutputSet};

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

/// Materialize file-processor cache outputs into the in-memory output set.
///
/// Returns `Ok(true)` when every referenced object exists and was linked into
/// `output`; `Ok(false)` when any object is missing.
pub(crate) fn materialize_file_entry(
    store: &ObjectStore,
    output_dir: &Path,
    entry: &FileEntry,
    output: &mut OutputSet,
    source_owners: &mut BTreeMap<String, String>,
    source_rel: &str,
) -> Result<bool> {
    let mut contents = Vec::with_capacity(entry.outputs.len());
    for out in &entry.outputs {
        match cached_content(store, output_dir, &out.path, out.object) {
            Some(content) => contents.push(content),
            None => return Ok(false),
        }
    }
    for (out, content) in entry.outputs.iter().zip(contents) {
        claim_source_output(source_owners, &out.path, source_rel)?;
        output.files.insert(out.path.clone(), content);
    }
    Ok(true)
}

/// Materialize generator cache mutations into the in-memory output set.
///
/// Returns `Ok(true)` when every referenced object exists; `Ok(false)` when any
/// object is missing.
pub(crate) fn materialize_generator_mutations(
    store: &ObjectStore,
    output_dir: &Path,
    mutations: &[GeneratorMutation],
    output: &mut OutputSet,
) -> Result<bool> {
    let mut emits = BTreeMap::new();
    for mutation in mutations {
        match mutation {
            GeneratorMutation::Emit(out) => {
                match cached_content(store, output_dir, &out.path, out.object) {
                    Some(content) => {
                        emits.insert(out.path.clone(), content);
                    }
                    None => return Ok(false),
                }
            }
            GeneratorMutation::EmitExternal { object, .. } if !store.contains(*object) => {
                return Ok(false)
            }
            _ => {}
        }
    }

    for mutation in mutations {
        match mutation {
            GeneratorMutation::Emit(out) => {
                crate::util::path::validate_relative(&out.path).map_err(Error::Build)?;
                let content = emits
                    .get(&out.path)
                    .cloned()
                    .unwrap_or(OutputContent::Object(out.object));
                output.files.insert(out.path.clone(), content);
            }
            GeneratorMutation::Remove(path) => {
                output.files.remove(path);
            }
            GeneratorMutation::EmitExternal { .. } => {}
        }
    }
    Ok(true)
}

/// Claim `output` for the processor chain of `source`.
pub(crate) fn claim_source_output(
    owners: &mut BTreeMap<String, String>,
    output: &str,
    source: &str,
) -> Result<()> {
    claim_output(owners, output, source).map_err(|error| match error {
        ClaimError::InvalidPath(message) => Error::Build(message),
        ClaimError::Taken { previous } => Error::Build(format!(
            "source files `{previous}` and `{source}` both produce `{output}`"
        )),
    })
}
