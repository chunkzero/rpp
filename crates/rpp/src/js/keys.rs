//! Cache keys of a loaded plugin.

use std::path::Path;

use rpp_js::{Bundle, Limits};
use serde_json::Value;

use super::access::RuntimeAccess;
use crate::config::WasmConfig;
use crate::error::{Error, Result};
use crate::manifest::PluginManifest;
use crate::util::hash::HashWriter;

/// The key of cached processor results: the rpp version, every bundled file outside `source`,
/// the manifest, component binaries (by the digest they were compiled from when loaded),
/// `options` (already [`canonical_options`]) and host access.
pub(super) fn processor_key(
    root: &Path,
    manifest: &PluginManifest,
    manifest_source: &str,
    bundle: &Bundle,
    source: Option<&Path>,
    options: &Value,
    access: &RuntimeAccess,
) -> Result<u64> {
    let mut writer = HashWriter::new();

    writer.write_str("rpp.js.processor.v1");
    writer.write_str(env!("CARGO_PKG_VERSION"));
    writer.write_str("inputs");
    for (input, hash) in bundle
        .input_hashes
        .iter()
        .filter(|(input, _)| source.is_none_or(|source| !input.starts_with(source)))
    {
        writer.write_str(&input.to_string_lossy());
        writer.write_u64(*hash);
    }
    writer.write_str("manifest");
    writer.write(manifest_source.as_bytes());
    for (name, component) in &manifest.components {
        writer.write_str("component");
        writer.write_str(name);
        writer.write_str(&component.module);
        #[cfg(feature = "wasm")]
        if let Some(compiled) = access.components.get(name) {
            writer.write_str("sha256");
            writer.write(&compiled.digest());
            continue;
        }
        let path = root.join(&component.module);
        let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
        writer.write_str("bytes");
        writer.write(&bytes);
    }
    writer.write_str("options");
    writer.write(options.to_string().as_bytes());
    writer.write_str("host-access");
    let access_key =
        serde_json::to_vec(&(access.security, &access.permissions, &access.outputs))
            .map_err(|error| Error::Build(format!("failed to hash plugin host access: {error}")))?;
    writer.write(&access_key);

    Ok(writer.finish())
}

/// The key of cached generator results: the processor key plus the bundled code.
pub(super) fn cache_key(processor_key: u64, bundle: &Bundle) -> u64 {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.js.plugin.v2");
    writer.write_u64(processor_key);
    writer.write(bundle.code.as_bytes());
    writer.finish()
}

/// The key of a cached `describe` result: the generator key plus the runtime and component
/// limits it ran under.
pub(super) fn describe_key(cache_key: u64, limits: Limits, wasm: &WasmConfig) -> u64 {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.js.describe.v1");
    writer.write_u64(cache_key);
    writer.write_u64(limits.heap_bytes as u64);
    writer.write_u64(u64::try_from(limits.time.as_nanos()).unwrap_or(u64::MAX));
    writer.write_u64(u64::from(wasm.memory_limit_mb));
    writer.write_u64(wasm.execution_deadline_seconds);
    writer.finish()
}

/// `value` with object keys sorted recursively, so hashing and plugins see the same order.
pub(super) fn canonical_options(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, item)| (key.clone(), canonical_options(item)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_options).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_options_sorts_keys() {
        let mut first = serde_json::Map::new();
        first.insert("b".into(), json!([{ "y": 1, "x": 2 }]));
        first.insert("a".into(), json!("s"));
        assert_eq!(
            canonical_options(&Value::Object(first)).to_string(),
            r#"{"a":"s","b":[{"x":2,"y":1}]}"#
        );
    }
}
