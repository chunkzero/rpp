//! Canonical serialization of the config for cache keys.

use serde::Serialize;

use crate::config::{BuildConfig, Config, PackConfig};
use crate::util::hash::xxh3;

/// Build-relevant `[build]` fields for the global cache key. Outputs are
/// content-addressed, so the output path and squash settings are excluded.
#[derive(Serialize)]
struct BuildKeySection<'a> {
    source: String,
    limits: &'a crate::config::LimitsConfig,
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
        limits: &build.limits,
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
