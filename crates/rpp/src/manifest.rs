//! Plugin manifest: `plugin.toml` and `rpp.json` parsing and validation (spec §2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::util::path::validate_relative;

/// Plugin id grammar: `^[a-z0-9][a-z0-9_-]*$`.
static ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9_-]*$").expect("static id regex is valid"));

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
    /// Lua entry script (relative to plugin root); defaults to `init.lua`.
    pub entry: String,
    /// Named WASM components callable by the Lua entry script.
    pub components: BTreeMap<String, ComponentManifest>,
    /// Config module (relative to the plugin root) whose default export is the
    /// plugin's config factory; `rpp.json` packages only.
    pub config: Option<String>,
}

/// One named component library shipped in a plugin package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentManifest {
    /// Component binary path relative to the plugin package.
    pub module: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    plugin: RawPlugin,
    #[serde(default, rename = "component")]
    components: BTreeMap<String, RawComponent>,
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
    #[serde(default)]
    entry: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawComponent {
    module: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJsonManifest {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    rpp: Option<String>,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    config: Option<String>,
    #[serde(default)]
    components: BTreeMap<String, String>,
    #[serde(default)]
    #[allow(dead_code)]
    dependencies: Option<serde::de::IgnoredAny>,
}

fn is_script_entry(entry: &str) -> bool {
    [".ts", ".mts", ".js", ".mjs"]
        .iter()
        .any(|ext| entry.ends_with(ext))
}

impl PluginManifest {
    /// Parse and validate a manifest from TOML text, attributing errors to `path`.
    pub fn parse(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let raw: RawManifest = toml::from_str(text).map_err(|e| Error::Manifest {
            path: path.clone(),
            message: format!("{e}"),
        })?;
        let RawManifest {
            plugin: raw,
            components,
        } = raw;

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

        let mut validated_components = BTreeMap::new();
        for (name, component) in components {
            if !ID_REGEX.is_match(&name) {
                return Err(Error::Manifest {
                    path,
                    message: format!("component name `{name}` must match ^[a-z0-9][a-z0-9_-]*$"),
                });
            }
            validate_relative(&component.module).map_err(|message| Error::Manifest {
                path: path.clone(),
                message: format!("invalid module for component `{name}`: {message}"),
            })?;
            validated_components.insert(
                name,
                ComponentManifest {
                    module: component.module,
                },
            );
        }

        Ok(PluginManifest {
            id: raw.id,
            version,
            description: raw.description,
            authors: raw.authors,
            entry,
            components: validated_components,
            config: None,
        })
    }

    /// Parse and validate an `rpp.json` package manifest, attributing errors to `path`.
    ///
    /// ```json
    /// {
    ///   "name": "window",
    ///   "version": "0.1.0",
    ///   "description": "…",
    ///   "rpp": ">=0.2",
    ///   "entry": "src/plugin.ts",
    ///   "config": "src/config.ts",
    ///   "components": { "compiler": "window.wasm" }
    /// }
    /// ```
    ///
    /// `name` becomes [`PluginManifest::id`] (same grammar). `entry` defaults to
    /// `src/plugin.ts` and must be a JavaScript entry; `entry`, `config` and component
    /// paths must be relative. `rpp` is checked during dependency resolution and only
    /// validated as a version requirement here. Unknown keys are rejected, except
    /// `dependencies`, which is ignored.
    pub fn parse_json(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let fail = |message: String| Error::Manifest {
            path: path.clone(),
            message,
        };
        let raw: RawJsonManifest = serde_json::from_str(text).map_err(|e| fail(e.to_string()))?;

        if !ID_REGEX.is_match(&raw.name) {
            return Err(fail(format!(
                "`name` `{}` must match ^[a-z0-9][a-z0-9_-]*$",
                raw.name
            )));
        }
        let version = Version::parse(&raw.version).map_err(|e| {
            fail(format!(
                "`version` `{}` is not valid semver: {e}",
                raw.version
            ))
        })?;
        if let Some(req) = &raw.rpp {
            VersionReq::parse(req)
                .map_err(|e| fail(format!("`rpp` `{req}` is not a valid version range: {e}")))?;
        }

        let entry = raw.entry.unwrap_or_else(|| "src/plugin.ts".to_string());
        validate_relative(&entry).map_err(|m| fail(format!("invalid `entry`: {m}")))?;
        if !is_script_entry(&entry) {
            return Err(fail(format!(
                "`entry` `{entry}` must be a .ts, .mts, .js or .mjs file"
            )));
        }
        if let Some(config) = &raw.config {
            validate_relative(config).map_err(|m| fail(format!("invalid `config`: {m}")))?;
        }

        let mut components = BTreeMap::new();
        for (name, module) in raw.components {
            if !ID_REGEX.is_match(&name) {
                return Err(fail(format!(
                    "component name `{name}` must match ^[a-z0-9][a-z0-9_-]*$"
                )));
            }
            validate_relative(&module)
                .map_err(|m| fail(format!("invalid module for component `{name}`: {m}")))?;
            components.insert(name, ComponentManifest { module });
        }

        Ok(PluginManifest {
            id: raw.name,
            version,
            description: raw.description,
            authors: Vec::new(),
            entry,
            components,
            config: raw.config,
        })
    }

    /// Load `rpp.json` from `dir` when present, otherwise `plugin.toml`.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_source(dir.as_ref()).map(|(manifest, _)| manifest)
    }

    /// Like [`PluginManifest::load`], also returning the manifest text.
    pub(crate) fn load_with_source(dir: &Path) -> Result<(Self, String)> {
        let json_path = dir.join("rpp.json");
        let (path, is_json) = if json_path.is_file() {
            (json_path, true)
        } else {
            (dir.join("plugin.toml"), false)
        };
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let manifest = if is_json {
            Self::parse_json(&text, path)?
        } else {
            Self::parse(&text, path)?
        };
        Ok((manifest, text))
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
"#,
            "plugin.toml",
        )
        .unwrap();
        assert_eq!(m.id, "json-minify");
        assert_eq!(m.version, Version::new(1, 2, 0));
        assert_eq!(m.entry, "init.lua");
        assert!(m.components.is_empty());
    }

    #[test]
    fn parses_named_component() {
        let manifest = PluginManifest::parse(
            "[plugin]\nid=\"x\"\nversion=\"1.0.0\"\n\
             [component.compiler]\nmodule=\"compiler.wasm\"\n",
            "plugin.toml",
        )
        .unwrap();
        assert_eq!(manifest.components["compiler"].module, "compiler.wasm");
    }

    #[test]
    fn rejects_bad_id() {
        let err = PluginManifest::parse(
            "[plugin]\nid=\"Bad_ID\"\nversion=\"1.0.0\"\n",
            "plugin.toml",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Manifest { .. }));
    }

    #[test]
    fn rejects_bad_version() {
        let err =
            PluginManifest::parse("[plugin]\nid=\"x\"\nversion=\"notsemver\"\n", "plugin.toml")
                .unwrap_err();
        assert!(matches!(err, Error::Manifest { .. }));
    }

    #[test]
    fn custom_entry() {
        let m = PluginManifest::parse(
            "[plugin]\nid=\"x\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\n",
            "plugin.toml",
        )
        .unwrap();
        assert_eq!(m.entry, "main.lua");
    }

    #[test]
    fn parses_json_manifest() {
        let m = PluginManifest::parse_json(
            r#"{"name":"window","version":"0.1.0","rpp":">=0.2","config":"src/config.ts",
                "components":{"compiler":"window.wasm"},"dependencies":{"x":"^1"}}"#,
            "rpp.json",
        )
        .unwrap();
        assert_eq!(m.id, "window");
        assert_eq!(m.entry, "src/plugin.ts");
        assert_eq!(m.config.as_deref(), Some("src/config.ts"));
        assert_eq!(m.components["compiler"].module, "window.wasm");
    }

    #[test]
    fn rejects_invalid_json_manifests() {
        for text in [
            r#"{"name":"x","version":"1.0.0","bogus":1}"#,
            r#"{"name":"X","version":"1.0.0"}"#,
            r#"{"name":"x","version":"1.0.0","rpp":"nope"}"#,
            r#"{"name":"x","version":"1.0.0","entry":"init.lua"}"#,
            r#"{"name":"x","version":"1.0.0","entry":"../a.ts"}"#,
            r#"{"name":"x","version":"1.0.0","components":{"c":"/abs.wasm"}}"#,
        ] {
            let err = PluginManifest::parse_json(text, "rpp.json").unwrap_err();
            assert!(matches!(err, Error::Manifest { .. }), "{text}");
        }
    }
}
