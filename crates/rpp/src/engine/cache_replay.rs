//! CAS materialization helpers for cache hits.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cache::{FileEntry, GeneratorMutation, ObjectStore};
use crate::error::{Error, Result};
use crate::util::glob::GlobSet;
use crate::util::hash::xxh3;
use crate::util::path::validate_relative;

use super::generator::{apply_mutation, claim_output, ClaimError, OutputContent, OutputSet, Owner};

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
        claim_source_output(&mut output.owners, &out.path, source_rel)?;
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
    plugin: &str,
    overrides: &GlobSet,
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
        apply_mutation(output, plugin, overrides, path, content).map_err(|message| {
            Error::Generator {
                plugin: plugin.to_string(),
                message,
            }
        })?;
    }
    Ok(true)
}

/// Claim `output` for the processor chain of `source`.
pub(crate) fn claim_source_output(
    owners: &mut BTreeMap<String, Owner>,
    output: &str,
    source: &str,
) -> Result<()> {
    claim_output(owners, output, Owner::Source(source.to_string())).map_err(|error| match error {
        ClaimError::InvalidPath(message) => Error::Build(message),
        ClaimError::Taken { previous } => {
            let Owner::Source(previous) = previous else {
                unreachable!("only sources claim outputs during the file phase")
            };
            Error::Build(format!(
                "source files `{previous}` and `{source}` both produce `{output}`"
            ))
        }
    })
}
