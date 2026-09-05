//! Canonical serialization of config and plugin options for cache keys.

use serde::Serialize;

use crate::config::{BuildConfig, Config, PackConfig};
use crate::util::hash::xxh3;

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

/// Build-relevant `[build]` fields for the global cache key. Outputs are
/// content-addressed, so the output path and squash settings are excluded.
#[derive(Serialize)]
struct BuildKeySection<'a> {
    source: String,
    lua: &'a crate::config::LuaConfig,
}

/// `[pack]` plus build-relevant fields for the global cache key.
#[derive(Serialize)]
struct GlobalKeyConfig<'a> {
    pack: &'a PackConfig,
    build: BuildKeySection<'a>,
}

fn path_key(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn build_key_section(build: &BuildConfig) -> BuildKeySection<'_> {
    BuildKeySection {
        source: path_key(&build.source),
        lua: &build.lua,
    }
}

/// Hash the build-relevant config sections using canonical JSON serialization.
pub(crate) fn config_digest(config: &Config) -> u64 {
    let payload = GlobalKeyConfig {
        pack: &config.pack,
        build: build_key_section(&config.build),
    };
    let bytes = serde_json::to_vec(&payload).expect("global key config serializes");
    xxh3(&bytes)
}
