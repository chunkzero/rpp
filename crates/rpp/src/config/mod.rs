//! Project configuration: the schema `rpp.config.ts` default-exports (spec §1).
//!
//! The [`Config`] type is built from the evaluated config by [`Config::from_ts_json`]. The
//! `squash` and `dev` sections are plain structs consumed by other crates (`rpp-squash`,
//! the dev server).

use std::path::PathBuf;

use crate::error::Result;

mod schema;
mod ts;
mod validate;

pub use schema::{
    BuildConfig, Config, DevConfig, LimitsConfig, PackConfig, PluginConfig, PluginPermissions,
    PngSetting, SecurityMode, SquashConfig, SquashEngine, WasmConfig,
};

impl Config {
    /// A config with default settings and the given pack name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            pack: PackConfig {
                name: name.into(),
                description: None,
                pack_format: None,
            },
            build: BuildConfig::default(),
            dev: DevConfig::default(),
            plugins: Vec::new(),
        }
    }

    /// Build a [`Config`] from the JSON value `rpp.config.ts` default-exports,
    /// attributing errors to `path`.
    ///
    /// Keys are camelCase (`pack.packFormat`, `build.squash.packsquashBinary`). `plugins` is
    /// an array of `{ plugin, options?, security?, permissions?, outputs? }` where `plugin`
    /// names an `rpp.json` dependency (stored in [`PluginConfig::package`]), and
    /// `build.limits` holds the plugin runtime limits. Keys from the removed TOML schema
    /// (`build.lua`, `permissions.lua`, `security: "native"`, `id`, `source`, `ref`,
    /// `subdir`) are rejected with a pointer to [`crate::MIGRATION_GUIDE`]. Keys inside
    /// `options` and `outputs` are kept verbatim; `null` values are invalid.
    pub fn from_ts_json(value: &serde_json::Value, path: impl Into<PathBuf>) -> Result<Self> {
        ts::from_json(value, path.into())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn new_has_documented_defaults() {
        let cfg = Config::new("demo");
        assert_eq!(cfg.pack.name, "demo");
        assert_eq!(cfg.build.source, PathBuf::from("src"));
        assert_eq!(cfg.build.output, PathBuf::from("dist"));
        assert_eq!(cfg.build.workers, 0);
        assert_eq!(cfg.build.limits.memory_limit_mb, 256);
        assert_eq!(cfg.build.limits.execution_deadline_seconds, 60);
        assert_eq!(cfg.build.wasm.memory_limit_mb, 512);
        assert!(cfg.build.squash.enabled);
        assert_eq!(cfg.dev.port, 8080);
        assert!(cfg.plugins.is_empty());
        cfg.validate(Path::new("rpp.config.ts")).unwrap();
    }
}
