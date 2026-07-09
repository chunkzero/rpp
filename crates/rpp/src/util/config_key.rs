//! Canonical serialization of build-relevant config for cache keys.

use serde::Serialize;

use crate::config::{BuildConfig, Config, PackConfig};
use crate::util::hash::xxh3;

/// Build-relevant `[build]` fields for the global cache key (excludes squash).
#[derive(Serialize)]
struct BuildKeySection<'a> {
    source: String,
    output: String,
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
        output: path_key(&build.output),
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
