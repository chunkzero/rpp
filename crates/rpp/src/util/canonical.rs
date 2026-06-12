//! Deterministic serialization of plugin options for cache keys.

/// Produce a deterministic string form of options independent of TOML formatting.
pub(crate) fn canonical_options_json(options: &toml::Value) -> String {
    let json: serde_json::Value = serde_json::to_value(options).unwrap_or(serde_json::Value::Null);
    canonical_json(&json)
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(&map[*k])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}
