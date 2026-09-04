//! CAS materialization helpers for cache hits.

use std::collections::BTreeMap;

use crate::cache::{FileEntry, GeneratorMutation, ObjectStore};
use crate::error::{Error, Result};

use super::generator::{claim_output, ClaimError, OutputContent, OutputSet};

/// Materialize file-processor cache outputs into the in-memory output set.
///
/// Returns `Ok(true)` when every referenced object exists and was linked into
/// `output`; `Ok(false)` when any object is missing.
pub(crate) fn materialize_file_entry(
    store: &ObjectStore,
    entry: &FileEntry,
    output: &mut OutputSet,
    source_owners: &mut BTreeMap<String, String>,
    source_rel: &str,
) -> Result<bool> {
    if entry.outputs.iter().any(|out| !store.contains(out.object)) {
        return Ok(false);
    }
    for out in &entry.outputs {
        claim_source_output(source_owners, &out.path, source_rel)?;
        output
            .files
            .insert(out.path.clone(), OutputContent::Object(out.object));
    }
    Ok(true)
}

/// Materialize generator cache mutations into the in-memory output set.
///
/// Returns `Ok(true)` when every referenced object exists; `Ok(false)` when any
/// object is missing.
pub(crate) fn materialize_generator_mutations(
    store: &ObjectStore,
    mutations: &[GeneratorMutation],
    output: &mut OutputSet,
    output_owners: &mut BTreeMap<String, String>,
    plugin_id: &str,
) -> Result<bool> {
    let missing = mutations.iter().any(|mutation| match mutation {
        GeneratorMutation::Emit(out) => !store.contains(out.object),
        GeneratorMutation::EmitExternal { object, .. } => !store.contains(*object),
        GeneratorMutation::Remove(_) => false,
    });
    if missing {
        return Ok(false);
    }

    for mutation in mutations {
        match mutation {
            GeneratorMutation::Emit(out) => {
                claim_output(output_owners, &out.path, plugin_id).map_err(|error| match error {
                    ClaimError::InvalidPath(message) => Error::Build(message),
                    ClaimError::Taken { previous } => Error::Build(format!(
                        "output `{}` already claimed by `{previous}`",
                        out.path
                    )),
                })?;
                output
                    .files
                    .insert(out.path.clone(), OutputContent::Object(out.object));
            }
            GeneratorMutation::Remove(path) => {
                output_owners.remove(path);
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
