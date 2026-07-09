//! Project configuration: the `rpp.toml` schema (spec §1).
//!
//! TOML is the only supported project config format. The [`Config`] type mirrors
//! the documented schema. The `squash` and `dev` sections are parsed into plain
//! structs here and consumed by other crates (`rpp-squash`, the dev server).

mod source;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub use source::{PluginSourceSpec, SourceParseError};

/// The fully parsed `rpp.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Pack metadata.
    pub pack: PackConfig,
    /// Build settings.
    #[serde(default)]
    pub build: BuildConfig,
    /// Dev-server settings.
    #[serde(default)]
    pub dev: DevConfig,
    /// Ordered plugin list. Order is the tie-break order for processor priority.
    #[serde(default, rename = "plugin")]
    pub plugins: Vec<PluginConfig>,
}

/// `[pack]` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackConfig {
    /// Pack name; used for the zip filename. Required.
    pub name: String,
    /// Human-readable description; written into `pack.mcmeta` when generated.
    #[serde(default)]
    pub description: Option<String>,
    /// Pack format; validated against `src/pack.mcmeta` when present.
    #[serde(default)]
    pub pack_format: Option<u32>,
}

/// Lua sandbox limits (`[build.lua]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LuaConfig {
    /// Per-Lua-state memory limit in megabytes.
    #[serde(default = "default_lua_memory_limit_mb")]
    pub memory_limit_mb: u32,
    /// Maximum wall-clock execution time per Lua call, in seconds.
    #[serde(default = "default_lua_execution_deadline_seconds")]
    pub execution_deadline_seconds: u64,
}

impl Default for LuaConfig {
    fn default() -> Self {
        Self {
            memory_limit_mb: default_lua_memory_limit_mb(),
            execution_deadline_seconds: default_lua_execution_deadline_seconds(),
        }
    }
}

/// `[build]` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    /// Pack source directory (contains `pack.mcmeta`, `assets/`).
    #[serde(default = "default_source")]
    pub source: PathBuf,
    /// Output directory; the zip is written to `<output>/<name>.zip`.
    #[serde(default = "default_output")]
    pub output: PathBuf,
    /// Worker thread count; `0` means available parallelism.
    #[serde(default)]
    pub workers: usize,
    /// Lua sandbox limits.
    #[serde(default)]
    pub lua: LuaConfig,
    /// Squash settings.
    #[serde(default)]
    pub squash: SquashConfig,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            source: default_source(),
            output: default_output(),
            workers: 0,
            lua: LuaConfig::default(),
            squash: SquashConfig::default(),
        }
    }
}

/// `[build.squash]` section. Consumed by `rpp-squash`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SquashConfig {
    /// Whether squashing runs at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optimization engine: `"builtin"` or `"packsquash"`.
    #[serde(default = "default_engine")]
    pub engine: String,
    /// Minify `.json`/`.mcmeta` in the output.
    #[serde(default = "default_true")]
    pub json: bool,
    /// PNG optimization level: `false`, `"fast"`, or `"max"`.
    #[serde(default)]
    pub png: PngSetting,
    /// Produce `<output>/<name>.zip`.
    #[serde(default = "default_true")]
    pub zip: bool,
    /// Glob patterns of files to strip from the output before zipping.
    #[serde(default = "default_strip")]
    pub strip: Vec<String>,
    /// PackSquash binary name/path (used when `engine = "packsquash"`).
    #[serde(default = "default_packsquash_binary")]
    pub packsquash_binary: String,
    /// Optional passthrough options file for PackSquash.
    #[serde(default)]
    pub packsquash_options: Option<PathBuf>,
}

impl Default for SquashConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: default_engine(),
            json: true,
            png: PngSetting::default(),
            zip: true,
            strip: default_strip(),
            packsquash_binary: default_packsquash_binary(),
            packsquash_options: None,
        }
    }
}

/// PNG optimization setting: `false` | `"fast"` | `"max"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PngSetting {
    /// PNG optimization disabled.
    #[default]
    Off,
    /// Fast optimization (oxipng preset 2).
    Fast,
    /// Maximum optimization (oxipng preset 6).
    Max,
}

impl<'de> Deserialize<'de> for PngSetting {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Str(String),
        }

        match Raw::deserialize(deserializer)? {
            Raw::Bool(false) => Ok(PngSetting::Off),
            Raw::Bool(true) => Ok(PngSetting::Fast),
            Raw::Str(s) => match s.as_str() {
                "off" | "false" => Ok(PngSetting::Off),
                "fast" => Ok(PngSetting::Fast),
                "max" => Ok(PngSetting::Max),
                other => Err(D::Error::custom(format!(
                    "expected false, \"fast\", or \"max\", got `{other}`"
                ))),
            },
        }
    }
}

impl Serialize for PngSetting {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            PngSetting::Off => serializer.serialize_bool(false),
            PngSetting::Fast => serializer.serialize_str("fast"),
            PngSetting::Max => serializer.serialize_str("max"),
        }
    }
}

/// `[dev]` section. Consumed by the dev server.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevConfig {
    /// Host to bind the dev server to.
    #[serde(default = "default_host")]
    pub host: String,
    /// Port to bind the dev server to.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Whether to open a browser on startup.
    #[serde(default)]
    pub open: bool,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            open: false,
        }
    }
}

/// One `[[plugin]]` entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// Global plugin id to use for this project.
    #[serde(default)]
    pub id: Option<String>,
    /// Source descriptor (`path:...` or `github:owner/repo`).
    #[serde(default)]
    pub source: Option<String>,
    /// Optional git ref (tag/branch/sha) for GitHub sources.
    #[serde(default)]
    pub r#ref: Option<String>,
    /// Optional subdirectory within a GitHub repo.
    #[serde(default)]
    pub subdir: Option<String>,
    /// Arbitrary options passed to the plugin.
    #[serde(default = "empty_table")]
    pub options: toml::Value,
    /// Security boundary used for this plugin.
    #[serde(default)]
    pub security: SecurityMode,
    /// Explicit host capabilities granted to this plugin.
    #[serde(default)]
    pub permissions: PluginPermissions,
    /// Named project-relative output roots for generated non-pack artifacts.
    #[serde(default)]
    pub outputs: BTreeMap<String, PathBuf>,
}

/// The strength of the plugin sandbox.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityMode {
    /// Pure processors and generator access only through tracked RPP APIs.
    #[default]
    Sandboxed,
    /// Explicit host capabilities are enabled while resource limits remain active.
    Trusted,
    /// Full Lua standard library and arbitrary host access; no sandbox guarantee.
    Native,
}

/// Capabilities granted by a `[[plugin]]` configuration entry.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPermissions {
    /// Executable names or absolute paths accepted by `rpp.process.run`.
    #[serde(default)]
    pub process: Vec<String>,
    /// Environment variable names visible to process calls and WASI components.
    #[serde(default)]
    pub environment: Vec<String>,
    /// Project-relative directories made readable to WASI components.
    #[serde(default)]
    pub read: Vec<PathBuf>,
    /// Project-relative directories made writable to WASI components.
    #[serde(default)]
    pub write: Vec<PathBuf>,
    /// Permit WASI sockets.
    #[serde(default)]
    pub network: bool,
    /// Permit WASI clocks and Lua's restricted `os.clock`/`time`/`date` table.
    #[serde(default)]
    pub clocks: bool,
    /// Permit host-backed WASI randomness and Lua's `math.random` functions.
    #[serde(default)]
    pub random: bool,
    /// Permit inherited WASI stdout/stderr.
    #[serde(default)]
    pub stdio: bool,
    /// Additional Lua standard-library capabilities in trusted mode.
    #[serde(default)]
    pub lua: Vec<LuaCapability>,
}

/// Additional Lua facilities available only to trusted/native plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LuaCapability {
    /// The `io` standard library.
    Io,
    /// Full `os` standard library, including `os.execute`.
    Os,
    /// Dynamic Lua loading (`load`, `loadfile`, and `dofile`).
    Load,
    /// The debug standard library.
    Debug,
    /// Lua package search paths, excluding native modules.
    Package,
}

/// An empty TOML table; the default for plugin options.
pub(crate) fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

impl PluginConfig {
    /// Parse [`Self::source`] into a structured [`PluginSourceSpec`].
    pub fn parse_source(&self) -> Option<std::result::Result<PluginSourceSpec, SourceParseError>> {
        self.source.as_ref().map(|source| {
            PluginSourceSpec::parse(source, self.r#ref.as_deref(), self.subdir.as_deref())
        })
    }

    /// Human-readable identity for diagnostics.
    pub fn label(&self) -> &str {
        self.source
            .as_deref()
            .or(self.id.as_deref())
            .unwrap_or("<unnamed>")
    }
}

impl Config {
    /// Parse a [`Config`] from TOML text, attributing errors to `path`.
    pub fn parse(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let config: Config = toml::from_str(text).map_err(|e| Error::Config {
            path: path.clone(),
            message: format!("{e}"),
        })?;
        config.validate(&path)?;
        Ok(config)
    }

    /// Load and parse a `rpp.toml` from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        Self::parse(&text, path)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if self.pack.name.trim().is_empty() {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`pack.name` must not be empty".into(),
            });
        }
        if self.pack.name == "."
            || self.pack.name == ".."
            || self.pack.name.contains(['/', '\\', '\0'])
        {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`pack.name` must be a file-name-safe value without path separators"
                    .into(),
            });
        }
        match self.build.squash.engine.as_str() {
            "builtin" | "packsquash" => {}
            other => {
                return Err(Error::Config {
                    path: path.to_path_buf(),
                    message: format!(
                        "`build.squash.engine` must be \"builtin\" or \"packsquash\", got `{other}`"
                    ),
                });
            }
        }
        for plugin in &self.plugins {
            validate_plugin_identity(plugin, path)?;
            validate_plugin_security(plugin, path)?;
        }

        if self.build.lua.memory_limit_mb == 0 {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`build.lua.memory_limit_mb` must be greater than 0".into(),
            });
        }
        if self.build.lua.execution_deadline_seconds == 0 {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`build.lua.execution_deadline_seconds` must be greater than 0".into(),
            });
        }

        if let Some(expected) = self.pack.pack_format {
            let project_root = path.parent().unwrap_or_else(|| Path::new("."));
            let mcmeta_path = project_root.join(&self.build.source).join("pack.mcmeta");
            if mcmeta_path.is_file() {
                validate_pack_format_mcmeta(&mcmeta_path, expected, path)?;
            }
        }

        Ok(())
    }
}

fn validate_plugin_identity(plugin: &PluginConfig, path: &Path) -> Result<()> {
    match (plugin.id.as_deref(), plugin.source.as_deref()) {
        (Some(id), None) if valid_plugin_id_ref(id) => Ok(()),
        (Some(id), None) => Err(Error::Config {
            path: path.to_path_buf(),
            message: format!("plugin id `{id}` is invalid"),
        }),
        (None, Some(source)) => {
            if let Err(e) =
                PluginSourceSpec::parse(source, plugin.r#ref.as_deref(), plugin.subdir.as_deref())
            {
                return Err(Error::Config {
                    path: path.to_path_buf(),
                    message: format!("plugin source `{source}`: {e}"),
                });
            }
            Ok(())
        }
        (Some(_), Some(_)) => Err(Error::Config {
            path: path.to_path_buf(),
            message: "plugin entries must set either `id` or `source`, not both".into(),
        }),
        (None, None) => Err(Error::Config {
            path: path.to_path_buf(),
            message: "plugin entries must set either `id` or `source`".into(),
        }),
    }
}

fn valid_plugin_id_ref(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn validate_plugin_security(plugin: &PluginConfig, path: &Path) -> Result<()> {
    let permissions = &plugin.permissions;
    let has_permissions = !permissions.process.is_empty()
        || !permissions.environment.is_empty()
        || !permissions.read.is_empty()
        || !permissions.write.is_empty()
        || permissions.network
        || permissions.clocks
        || permissions.random
        || permissions.stdio
        || !permissions.lua.is_empty();
    if plugin.security == SecurityMode::Sandboxed && has_permissions {
        return Err(Error::Config {
            path: path.to_path_buf(),
            message: format!(
                "plugin `{}` grants permissions but uses `security = \"sandboxed\"`",
                plugin.label()
            ),
        });
    }

    for (label, values) in [
        ("permissions.read", &permissions.read),
        ("permissions.write", &permissions.write),
    ] {
        for value in values {
            validate_project_relative(value).map_err(|message| Error::Config {
                path: path.to_path_buf(),
                message: format!("plugin `{}` {label}: {message}", plugin.label()),
            })?;
        }
    }
    for (name, output) in &plugin.outputs {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: format!(
                    "plugin `{}` has invalid output name `{name}`",
                    plugin.label()
                ),
            });
        }
        validate_project_relative(output).map_err(|message| Error::Config {
            path: path.to_path_buf(),
            message: format!("plugin `{}` output `{name}`: {message}", plugin.label()),
        })?;
    }
    Ok(())
}

fn validate_project_relative(path: &Path) -> std::result::Result<(), &'static str> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("path must be project-relative");
    }
    if path.components().any(|component| {
        !matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::ParentDir
        )
    }) {
        return Err("path must be normalized");
    }
    Ok(())
}

fn validate_pack_format_mcmeta(
    mcmeta_path: &Path,
    expected: u32,
    config_path: &Path,
) -> Result<()> {
    let text = std::fs::read_to_string(mcmeta_path).map_err(|e| Error::io(mcmeta_path, e))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| Error::Config {
        path: config_path.to_path_buf(),
        message: format!("`{}` is not valid JSON: {e}", mcmeta_path.display()),
    })?;
    let actual = value
        .get("pack")
        .and_then(|pack| pack.get("pack_format"))
        .and_then(|format| format.as_u64());
    match actual {
        Some(actual) if actual == u64::from(expected) => Ok(()),
        Some(actual) => Err(Error::Config {
            path: config_path.to_path_buf(),
            message: format!(
                "`pack.pack_format` ({expected}) does not match `{}` pack_format ({actual})",
                mcmeta_path.display()
            ),
        }),
        None => Err(Error::Config {
            path: config_path.to_path_buf(),
            message: format!(
                "`{}` is missing `pack.pack_format` but `pack.pack_format` is set in config",
                mcmeta_path.display()
            ),
        }),
    }
}

fn default_source() -> PathBuf {
    PathBuf::from("src")
}
fn default_output() -> PathBuf {
    PathBuf::from("dist")
}
fn default_true() -> bool {
    true
}
fn default_engine() -> String {
    "builtin".into()
}
fn default_packsquash_binary() -> String {
    "packsquash".into()
}
fn default_strip() -> Vec<String> {
    vec![
        "**/.DS_Store".into(),
        "**/Thumbs.db".into(),
        "**/*.psd".into(),
        "**/*.xcf".into(),
    ]
}
fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    8080
}
fn default_lua_memory_limit_mb() -> u32 {
    256
}
fn default_lua_execution_deadline_seconds() -> u64 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let cfg = Config::parse("[pack]\nname = \"demo\"\n", "rpp.toml").unwrap();
        assert_eq!(cfg.pack.name, "demo");
        assert_eq!(cfg.build.source, PathBuf::from("src"));
        assert_eq!(cfg.build.output, PathBuf::from("dist"));
        assert!(cfg.build.squash.enabled);
        assert_eq!(cfg.dev.port, 8080);
        assert!(cfg.plugins.is_empty());
    }

    #[test]
    fn parses_full_config() {
        let text = r#"
[pack]
name = "my-pack"
description = "An example pack"
pack_format = 34

[build]
source = "source"
output = "out"
workers = 4

[build.squash]
enabled = true
engine = "packsquash"
json = true
png = "max"
zip = false
strip = ["**/*.bak"]
packsquash_binary = "ps"

[dev]
host = "0.0.0.0"
port = 9000
open = true

[[plugin]]
source = "path:plugins/json-minify"
[plugin.options]
pretty = false

[[plugin]]
source = "github:example/rpp-plugins"
ref = "v1.2.0"
subdir = "plugins/atlas"
"#;
        let cfg = Config::parse(text, "rpp.toml").unwrap();
        assert_eq!(cfg.pack.pack_format, Some(34));
        assert_eq!(cfg.build.workers, 4);
        assert_eq!(cfg.build.squash.engine, "packsquash");
        assert_eq!(cfg.build.squash.png, PngSetting::Max);
        assert!(!cfg.build.squash.zip);
        assert_eq!(cfg.dev.host, "0.0.0.0");
        assert_eq!(cfg.plugins.len(), 2);
        assert!(matches!(
            cfg.plugins[0].parse_source().unwrap().unwrap(),
            PluginSourceSpec::Path { .. }
        ));
        assert!(matches!(
            cfg.plugins[1].parse_source().unwrap().unwrap(),
            PluginSourceSpec::GitHub { .. }
        ));
    }

    #[test]
    fn parses_global_plugin_reference() {
        let cfg = Config::parse(
            "[pack]\nname = \"demo\"\n[[plugin]]\nid = \"window\"\nsecurity = \"trusted\"\n",
            "rpp.toml",
        )
        .unwrap();
        assert_eq!(cfg.plugins[0].id.as_deref(), Some("window"));
        assert!(cfg.plugins[0].source.is_none());
        assert!(cfg.plugins[0].parse_source().is_none());
    }

    #[test]
    fn png_bool_false_is_off() {
        let cfg = Config::parse(
            "[pack]\nname=\"x\"\n[build.squash]\npng = false\n",
            "rpp.toml",
        )
        .unwrap();
        assert_eq!(cfg.build.squash.png, PngSetting::Off);
    }

    #[test]
    fn empty_name_rejected() {
        let err = Config::parse("[pack]\nname = \"\"\n", "rpp.toml").unwrap_err();
        assert!(matches!(err, Error::Config { .. }));
    }

    #[test]
    fn unsafe_pack_name_rejected() {
        let err = Config::parse("[pack]\nname = \"../escape\"\n", "rpp.toml").unwrap_err();
        assert!(matches!(err, Error::Config { .. }));
    }

    #[test]
    fn unknown_field_rejected() {
        let err = Config::parse("[pack]\nname=\"x\"\nbogus = 1\n", "rpp.toml").unwrap_err();
        assert!(matches!(err, Error::Config { .. }));
    }

    #[test]
    fn bad_engine_rejected() {
        let err = Config::parse(
            "[pack]\nname=\"x\"\n[build.squash]\nengine=\"nope\"\n",
            "rpp.toml",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Config { .. }));
    }

    #[test]
    fn lua_defaults() {
        let cfg = Config::parse("[pack]\nname = \"demo\"\n", "rpp.toml").unwrap();
        assert_eq!(cfg.build.lua.memory_limit_mb, 256);
        assert_eq!(cfg.build.lua.execution_deadline_seconds, 60);
    }

    #[test]
    fn parses_lua_limits() {
        let cfg = Config::parse(
            "[pack]\nname=\"x\"\n[build.lua]\nmemory_limit_mb = 512\nexecution_deadline_seconds = 120\n",
            "rpp.toml",
        )
        .unwrap();
        assert_eq!(cfg.build.lua.memory_limit_mb, 512);
        assert_eq!(cfg.build.lua.execution_deadline_seconds, 120);
    }

    #[test]
    fn pack_format_mismatch_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/pack.mcmeta"),
            r#"{"pack":{"pack_format":9}}"#,
        )
        .unwrap();
        let err = Config::parse(
            "[pack]\nname=\"x\"\npack_format = 34\n",
            root.join("rpp.toml"),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn pack_format_agreement_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/pack.mcmeta"),
            r#"{"pack":{"pack_format":34}}"#,
        )
        .unwrap();
        Config::parse(
            "[pack]\nname=\"x\"\npack_format = 34\n",
            root.join("rpp.toml"),
        )
        .unwrap();
    }
}
