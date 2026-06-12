//! Plugin manifest: `plugin.toml` parsing and validation (spec §2).

use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::path::validate_relative;

/// Plugin id grammar: `^[a-z0-9][a-z0-9_-]*$`.
static ID_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9_-]*$").expect("static id regex is valid"));

/// The runtime that backs a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    /// A Lua 5.4 plugin.
    Lua,
    /// A WASM (WASIp2 component) plugin.
    Wasm,
}

/// A validated `plugin.toml` manifest.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Unique plugin id (`^[a-z0-9][a-z0-9_-]*$`).
    pub id: String,
    /// Semantic version.
    pub version: Version,
    /// Optional description.
    pub description: Option<String>,
    /// Authors.
    pub authors: Vec<String>,
    /// The runtime backing this plugin.
    pub runtime: Runtime,
    /// Lua entry script (relative to plugin root); defaults to `init.lua`.
    pub entry: String,
    /// WASM component module (relative to plugin root); required for wasm.
    pub module: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    plugin: RawPlugin,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlugin {
    id: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    runtime: Runtime,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    module: Option<String>,
}

impl PluginManifest {
    /// Parse and validate a manifest from TOML text, attributing errors to `path`.
    pub fn parse(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let raw: RawManifest = toml::from_str(text).map_err(|e| Error::Manifest {
            path: path.clone(),
            message: format!("{e}"),
        })?;
        let raw = raw.plugin;

        if !ID_REGEX.is_match(&raw.id) {
            return Err(Error::Manifest {
                path,
                message: format!("`id` `{}` must match ^[a-z0-9][a-z0-9_-]*$", raw.id),
            });
        }

        let version = Version::parse(&raw.version).map_err(|e| Error::Manifest {
            path: path.clone(),
            message: format!("`version` `{}` is not valid semver: {e}", raw.version),
        })?;

        let entry = raw.entry.unwrap_or_else(|| "init.lua".to_string());
        validate_relative(&entry).map_err(|message| Error::Manifest {
            path: path.clone(),
            message: format!("invalid `entry`: {message}"),
        })?;

        if raw.runtime == Runtime::Wasm && raw.module.is_none() {
            return Err(Error::Manifest {
                path,
                message: "`module` is required for wasm runtime".into(),
            });
        }
        if let Some(module) = &raw.module {
            validate_relative(module).map_err(|message| Error::Manifest {
                path: path.clone(),
                message: format!("invalid `module`: {message}"),
            })?;
        }

        Ok(PluginManifest {
            id: raw.id,
            version,
            description: raw.description,
            authors: raw.authors,
            runtime: raw.runtime,
            entry,
            module: raw.module,
        })
    }

    /// Load and parse a `plugin.toml` from a plugin package directory.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        Self::parse(&text, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lua_manifest() {
        let m = PluginManifest::parse(
            r#"
[plugin]
id = "json-minify"
version = "1.2.0"
description = "Minifies JSON files"
authors = ["someone"]
runtime = "lua"
"#,
            "plugin.toml",
        )
        .unwrap();
        assert_eq!(m.id, "json-minify");
        assert_eq!(m.version, Version::new(1, 2, 0));
        assert_eq!(m.runtime, Runtime::Lua);
        assert_eq!(m.entry, "init.lua");
    }

    #[test]
    fn wasm_requires_module() {
        let err = PluginManifest::parse(
            "[plugin]\nid=\"x\"\nversion=\"1.0.0\"\nruntime=\"wasm\"\n",
            "plugin.toml",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Manifest { .. }));
    }

    #[test]
    fn rejects_bad_id() {
        let err = PluginManifest::parse(
            "[plugin]\nid=\"Bad_ID\"\nversion=\"1.0.0\"\nruntime=\"lua\"\n",
            "plugin.toml",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Manifest { .. }));
    }

    #[test]
    fn rejects_bad_version() {
        let err = PluginManifest::parse(
            "[plugin]\nid=\"x\"\nversion=\"notsemver\"\nruntime=\"lua\"\n",
            "plugin.toml",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Manifest { .. }));
    }

    #[test]
    fn custom_entry() {
        let m = PluginManifest::parse(
            "[plugin]\nid=\"x\"\nversion=\"1.0.0\"\nruntime=\"lua\"\nentry=\"main.lua\"\n",
            "plugin.toml",
        )
        .unwrap();
        assert_eq!(m.entry, "main.lua");
    }
}
