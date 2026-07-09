//! CAS materialization helpers for cache hits.

use std::collections::BTreeMap;

use crate::cache::{FileEntry, GeneratorMutation, ObjectStore};
use crate::error::Result;

use super::generator::{apply_generator_mutation, OutputContent, OutputSet, RecordedMutation};

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
    for out in &entry.outputs {
        if !store.contains(out.object) {
            return Ok(false);
        }
    }

    for out in &entry.outputs {
        claim_source_output(source_owners, &out.path, source_rel)?;
        output
            .files
            .insert(out.path.clone(), OutputContent::from_object(out.object));
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
    for mutation in mutations {
        match mutation {
            GeneratorMutation::Emit(out) => {
                if !store.contains(out.object) {
                    return Ok(false);
                }
            }
            GeneratorMutation::EmitExternal { object, .. } => {
                if !store.contains(*object) {
                    return Ok(false);
                }
            }
            GeneratorMutation::Remove(_) => {}
        }
    }

    for mutation in mutations {
        match mutation {
            GeneratorMutation::Emit(out) => {
                claim_generator_output(output_owners, &out.path, plugin_id)?;
                apply_generator_mutation(
                    output,
                    RecordedMutation::EmitObject {
                        path: out.path.clone(),
                        object: out.object,
                    },
                );
            }
            GeneratorMutation::Remove(path) => {
                output_owners.remove(path);
                apply_generator_mutation(output, RecordedMutation::Remove(path.clone()));
            }
            GeneratorMutation::EmitExternal { .. } => {}
        }
    }
    Ok(true)
}

pub(crate) fn claim_source_output(
    owners: &mut BTreeMap<String, String>,
    output: &str,
    source: &str,
) -> Result<()> {
    crate::util::path::validate_relative(output).map_err(crate::error::Error::Build)?;
    if let Some(previous) = owners.insert(output.to_string(), source.to_string()) {
        return Err(crate::error::Error::Build(format!(
            "source files `{previous}` and `{source}` both produce `{output}`"
        )));
    }
    Ok(())
}

fn claim_generator_output(
    owners: &mut BTreeMap<String, String>,
    output: &str,
    plugin_id: &str,
) -> Result<()> {
    crate::util::path::validate_relative(output).map_err(crate::error::Error::Build)?;
    if let Some(previous) = owners.insert(output.to_string(), plugin_id.to_string()) {
        return Err(crate::error::Error::Build(format!(
            "output `{output}` already claimed by `{previous}`"
        )));
    }
    Ok(())
}
