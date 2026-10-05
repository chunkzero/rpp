//! Plugin manifest: `rpp.json` parsing and validation (spec §2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::util::glob;
use crate::util::path::validate_relative;

/// The plugin id grammar, also used for package, component and discover names.
pub(crate) const ID_GRAMMAR: &str = "^[a-z0-9][a-z0-9_-]*$";

/// Whether `name` matches [`ID_GRAMMAR`].
pub(crate) fn is_valid_id(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// A validated `rpp.json` manifest.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Unique plugin id (`^[a-z0-9][a-z0-9_-]*$`).
    pub id: String,
    /// Semantic version.
    pub version: Version,
    /// Optional description.
    pub description: Option<String>,
    /// Entry module (relative to plugin root); defaults to `src/plugin.ts`.
    pub entry: String,
    /// Named WASM components callable by the plugin.
    pub components: BTreeMap<String, ComponentManifest>,
    /// Config module (relative to the plugin root) whose default export is the
    /// plugin's config factory.
    pub config: Option<String>,
    /// Required rpp version range.
    pub rpp: Option<VersionReq>,
    /// Entry discovery patterns by name, relative to the pack source directory.
    pub discover: BTreeMap<String, String>,
    /// Pack-path globs of files this plugin's generator may emit over or remove even when
    /// another source or plugin owns them.
    pub overrides: Vec<String>,
}

/// One named component library shipped in a plugin package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentManifest {
    /// Component binary path relative to the plugin package.
    pub module: String,
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
    discover: BTreeMap<String, String>,
    #[serde(default)]
    overrides: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    dependencies: Option<serde::de::IgnoredAny>,
}

type Check<T> = std::result::Result<T, String>;

fn validate_id(field: &str, name: &str) -> Check<()> {
    if is_valid_id(name) {
        Ok(())
    } else {
        Err(format!("{field} `{name}` must match {ID_GRAMMAR}"))
    }
}

fn parse_version(version: &str) -> Check<Version> {
    Version::parse(version).map_err(|e| format!("`version` `{version}` is not valid semver: {e}"))
}

fn parse_rpp(req: Option<&str>) -> Check<Option<VersionReq>> {
    req.map(|req| {
        VersionReq::parse(req)
            .map_err(|e| format!("`rpp` `{req}` is not a valid version range: {e}"))
    })
    .transpose()
}

/// Whether `entry` names a module the JavaScript runtime can load.
fn is_script_entry(entry: &str) -> bool {
    [".ts", ".mts", ".js", ".mjs"]
        .iter()
        .any(|ext| entry.ends_with(ext))
}

fn validate_entry(entry: &str) -> Check<()> {
    validate_relative(entry).map_err(|m| format!("invalid `entry`: {m}"))?;
    if entry.ends_with(".lua") {
        return Err(format!(
            "`entry` `{entry}` is a Lua script; Lua plugins are no longer supported, \
             write the entry as a TypeScript module; see {}",
            crate::MIGRATION_GUIDE
        ));
    }
    if !is_script_entry(entry) {
        return Err(format!(
            "`entry` `{entry}` must be a .ts, .mts, .js or .mjs file"
        ));
    }
    Ok(())
}

fn parse_components(raw: BTreeMap<String, String>) -> Check<BTreeMap<String, ComponentManifest>> {
    raw.into_iter()
        .map(|(name, module)| {
            validate_id("component name", &name)?;
            validate_relative(&module)
                .map_err(|m| format!("invalid module for component `{name}`: {m}"))?;
            Ok((name, ComponentManifest { module }))
        })
        .collect()
}

fn validate_discover(discover: &BTreeMap<String, String>) -> Check<()> {
    for (name, pattern) in discover {
        validate_id("discover name", name)?;
        validate_relative(pattern)
            .and_then(|()| glob::compile(pattern).map(drop))
            .map_err(|m| format!("invalid pattern for discover `{name}`: {m}"))?;
    }
    Ok(())
}

fn validate_overrides(overrides: &[String]) -> Check<()> {
    for pattern in overrides {
        validate_relative(pattern)
            .and_then(|()| glob::compile(pattern).map(drop))
            .map_err(|m| format!("invalid `overrides` pattern: {m}"))?;
    }
    Ok(())
}

impl PluginManifest {
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
    ///   "components": { "compiler": "window.wasm" },
    ///   "discover": { "windows": "*/window/**/window.ts" }
    /// }
    /// ```
    ///
    /// `discover` maps names (same grammar) to one glob each, relative to the pack source
    /// directory. `name` becomes [`PluginManifest::id`] (same grammar). `entry` defaults to
    /// `src/plugin.ts` and must be a JavaScript entry; `entry`, `config` and component
    /// paths must be relative. `rpp` is a version requirement checked by
    /// [`PluginManifest::load`]. Unknown keys are rejected, except `dependencies`, which is
    /// ignored.
    pub fn parse_json(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        Self::from_raw(text).map_err(|message| Error::Manifest { path, message })
    }

    fn from_raw(text: &str) -> Check<Self> {
        let raw: RawJsonManifest = serde_json::from_str(text).map_err(|e| e.to_string())?;
        validate_id("`name`", &raw.name)?;
        let version = parse_version(&raw.version)?;
        let rpp = parse_rpp(raw.rpp.as_deref())?;
        let entry = raw.entry.unwrap_or_else(|| "src/plugin.ts".to_string());
        validate_entry(&entry)?;
        if let Some(config) = &raw.config {
            validate_relative(config).map_err(|m| format!("invalid `config`: {m}"))?;
        }
        let components = parse_components(raw.components)?;
        validate_discover(&raw.discover)?;
        validate_overrides(&raw.overrides)?;
        Ok(PluginManifest {
            id: raw.name,
            version,
            description: raw.description,
            entry,
            components,
            config: raw.config,
            rpp,
            discover: raw.discover,
            overrides: raw.overrides,
        })
    }

    /// Load and validate `dir/rpp.json`. A directory with only a legacy `plugin.toml` is rejected
    /// with a pointer to [`crate::MIGRATION_GUIDE`].
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_source(dir.as_ref()).map(|(manifest, _)| manifest)
    }

    /// Like [`PluginManifest::load`], also returning the manifest text.
    pub(crate) fn load_with_source(dir: &Path) -> Result<(Self, String)> {
        let path = dir.join("rpp.json");
        if !path.is_file() {
            let legacy = dir.join("plugin.toml");
            if legacy.is_file() {
                return Err(Error::Manifest {
                    path: legacy,
                    message: format!(
                        "`plugin.toml` plugins (Lua) are no longer supported; add `rpp.json` \
                         with a TypeScript entry; see {}",
                        crate::MIGRATION_GUIDE
                    ),
                });
            }
        }
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let manifest = Self::parse_json(&text, path.clone())?;
        if let Some(req) = &manifest.rpp {
            let mut running = Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version");
            running.pre = semver::Prerelease::EMPTY;
            if !req.matches(&running) {
                return Err(Error::Manifest {
                    path,
                    message: format!(
                        "requires rpp `{req}`, but the running rpp is {}",
                        env!("CARGO_PKG_VERSION")
                    ),
                });
            }
        }
        Ok((manifest, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_json_discover() {
        let m = PluginManifest::parse_json(
            r#"{"name":"x","version":"1.0.0","discover":{"windows":"*/window/**/window.ts"}}"#,
            "rpp.json",
        )
        .unwrap();
        assert_eq!(m.discover["windows"], "*/window/**/window.ts");
        let m = PluginManifest::parse_json(r#"{"name":"x","version":"1.0.0"}"#, "rpp.json");
        assert!(m.unwrap().discover.is_empty());
    }

    #[test]
    fn rejects_invalid_discover() {
        for discover in [
            r#"{"Bad":"a/*.ts"}"#,
            r#"{"w":""}"#,
            r#"{"w":"../a/*.ts"}"#,
            r#"{"w":"/abs/*.ts"}"#,
            r#"{"w":"a/[.ts"}"#,
            r#"{"w":["a/*.ts"]}"#,
        ] {
            let text = format!(r#"{{"name":"x","version":"1.0.0","discover":{discover}}}"#);
            let err = PluginManifest::parse_json(&text, "rpp.json").unwrap_err();
            assert!(matches!(err, Error::Manifest { .. }), "{discover}");
        }
    }

    #[test]
    fn parses_json_overrides() {
        let m = PluginManifest::parse_json(
            r#"{"name":"x","version":"1.0.0","overrides":["assets/*/textures/**"]}"#,
            "rpp.json",
        )
        .unwrap();
        assert_eq!(m.overrides, ["assets/*/textures/**"]);
    }

    #[test]
    fn rejects_invalid_overrides() {
        for overrides in [r#"["../a"]"#, r#"["/abs/**"]"#, r#"["a/["]"#, r#"[""]"#] {
            let text = format!(r#"{{"name":"x","version":"1.0.0","overrides":{overrides}}}"#);
            let err = PluginManifest::parse_json(&text, "rpp.json").unwrap_err();
            assert!(matches!(err, Error::Manifest { .. }), "{overrides}");
        }
    }

    #[test]
    fn load_rejects_unmatched_rpp_requirement() {
        let dir = tempfile::tempdir().unwrap();
        let write = |req: &str| {
            std::fs::write(
                dir.path().join("rpp.json"),
                format!(r#"{{"name":"x","version":"1.0.0","rpp":"{req}"}}"#),
            )
            .unwrap();
        };
        write(">=0.1");
        PluginManifest::load(dir.path()).unwrap();
        write(">=99");
        let err = PluginManifest::load(dir.path()).unwrap_err().to_string();
        assert!(
            err.contains(">=99") && err.contains(env!("CARGO_PKG_VERSION")),
            "{err}"
        );
    }

    #[test]
    fn rejects_invalid_json_manifests() {
        for text in [
            r#"{"name":"x","version":"1.0.0","bogus":1}"#,
            r#"{"name":"X","version":"1.0.0"}"#,
            r#"{"name":"x","version":"1.0.0","rpp":"nope"}"#,
            r#"{"name":"x","version":"1.0.0","entry":"../a.ts"}"#,
            r#"{"name":"x","version":"1.0.0","components":{"c":"/abs.wasm"}}"#,
        ] {
            let err = PluginManifest::parse_json(text, "rpp.json").unwrap_err();
            assert!(matches!(err, Error::Manifest { .. }), "{text}");
        }
    }

    #[test]
    fn plugin_toml_only_dir_is_rejected_with_guide() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("plugin.toml"),
            "[plugin]\nid = \"x\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let err = PluginManifest::load(dir.path()).unwrap_err().to_string();
        assert!(err.contains("plugin.toml"), "{err}");
        assert!(err.ends_with(crate::MIGRATION_GUIDE), "{err}");
    }

    #[test]
    fn lua_entry_in_rpp_json_is_rejected_with_guide() {
        let err = PluginManifest::parse_json(
            r#"{"name":"x","version":"1.0.0","entry":"init.lua"}"#,
            "rpp.json",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Lua"), "{err}");
        assert!(err.ends_with(crate::MIGRATION_GUIDE), "{err}");
    }
}
