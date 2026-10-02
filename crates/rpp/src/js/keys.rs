//! Cache keys of a loaded plugin.

use std::path::Path;

use rpp_js::Bundle;
use serde_json::Value;

use super::access::RuntimeAccess;
use crate::error::{Error, Result};
use crate::manifest::PluginManifest;
use crate::util::hash::HashWriter;

/// The key of cached processor results: the rpp version, every bundled file outside `source`,
/// the manifest, component binaries, canonical options and host access.
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
        let path = root.join(&component.module);
        let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
        writer.write_str("component");
        writer.write_str(name);
        writer.write_str(&component.module);
        writer.write(&bytes);
    }
    writer.write_str("options");
    writer.write(canonical_json(options).as_bytes());
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

/// `value` as JSON with object keys sorted, independent of map ordering.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            let parts: Vec<String> = entries
                .into_iter()
                .map(|(key, item)| {
                    format!("{}:{}", Value::from(key.as_str()), canonical_json(item))
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_sorts_keys() {
        let mut first = serde_json::Map::new();
        first.insert("b".into(), json!([{ "y": 1, "x": 2 }]));
        first.insert("a".into(), json!("s"));
        assert_eq!(
            canonical_json(&Value::Object(first)),
            r#"{"a":"s","b":[{"x":2,"y":1}]}"#
        );
    }
}
