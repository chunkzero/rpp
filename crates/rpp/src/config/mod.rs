//! Project configuration: the schema `rpp.config.ts` default-exports (spec §1).
//!
//! The [`Config`] type is built from the evaluated config by [`Config::from_ts_json`]. The
//! `squash` and `dev` sections are plain structs consumed by other crates (`rpp-squash`,
//! the dev server).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

mod ts;

/// The project configuration exported by `rpp.config.ts`.
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

/// `pack` section.
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

/// Plugin runtime limits (`build.limits`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    /// Per-plugin-runtime memory limit in megabytes.
    #[serde(default = "default_limits_memory_limit_mb")]
    pub memory_limit_mb: u32,
    /// Maximum wall-clock execution time per plugin call, in seconds.
    #[serde(default = "default_limits_execution_deadline_seconds")]
    pub execution_deadline_seconds: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            memory_limit_mb: default_limits_memory_limit_mb(),
            execution_deadline_seconds: default_limits_execution_deadline_seconds(),
        }
    }
}

/// WASM component limits (`build.wasm`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WasmConfig {
    /// Per-instance linear-memory limit in megabytes.
    #[serde(default = "default_wasm_memory_limit_mb")]
    pub memory_limit_mb: u32,
    /// Maximum wall-clock execution time per component call, in seconds.
    #[serde(default = "default_wasm_execution_deadline_seconds")]
    pub execution_deadline_seconds: u64,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            memory_limit_mb: default_wasm_memory_limit_mb(),
            execution_deadline_seconds: default_wasm_execution_deadline_seconds(),
        }
    }
}

/// `build` section.
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
    /// Plugin runtime limits.
    #[serde(default)]
    pub limits: LimitsConfig,
    /// WASM component limits.
    #[serde(default)]
    pub wasm: WasmConfig,
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
            limits: LimitsConfig::default(),
            wasm: WasmConfig::default(),
            squash: SquashConfig::default(),
        }
    }
}

/// `build.squash` section. Consumed by `rpp-squash`.
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
    /// PackSquash binary name/path (used when `engine` is `"packsquash"`).
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

/// `dev` section. Consumed by the dev server.
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

/// One entry of `plugins`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// Dependency name from the project's `rpp.json`.
    pub package: String,
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
}

/// Capabilities granted by a plugin entry.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPermissions {
    /// Executable names or absolute paths accepted by `process.run`.
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
    /// Permit WASI clocks and real time in plugin code.
    #[serde(default)]
    pub clocks: bool,
    /// Permit host-backed WASI randomness and real randomness in plugin code.
    #[serde(default)]
    pub random: bool,
    /// Permit inherited WASI stdout/stderr.
    #[serde(default)]
    pub stdio: bool,
}

/// An empty TOML table; the default for plugin options.
pub(crate) fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

impl PluginConfig {
    /// Validate capability policy and output roots, attributing errors to `path`.
    pub fn validate(&self, path: &Path) -> Result<()> {
        if !valid_dependency_name(&self.package) {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: format!(
                    "plugin package `{}` must match ^[a-z0-9][a-z0-9_-]*$",
                    self.package
                ),
            });
        }
        validate_plugin_security(self, path)
    }

    /// Human-readable identity for diagnostics.
    pub fn label(&self) -> &str {
        &self.package
    }
}

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
            plugin.validate(path)?;
        }
        if self.build.limits.memory_limit_mb == 0 {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`build.limits.memory_limit_mb` must be greater than 0".into(),
            });
        }
        if self.build.limits.execution_deadline_seconds == 0 {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`build.limits.execution_deadline_seconds` must be greater than 0".into(),
            });
        }
        if self.build.wasm.memory_limit_mb == 0 {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`build.wasm.memory_limit_mb` must be greater than 0".into(),
            });
        }
        if self.build.wasm.execution_deadline_seconds == 0 {
            return Err(Error::Config {
                path: path.to_path_buf(),
                message: "`build.wasm.execution_deadline_seconds` must be greater than 0".into(),
            });
        }

        Ok(())
    }

    pub(crate) fn validate_source(&self, project_root: &Path) -> Result<()> {
        let path = project_root.join("rpp.config.ts");
        if let Some(expected) = self.pack.pack_format {
            let mcmeta_path = project_root.join(&self.build.source).join("pack.mcmeta");
            if mcmeta_path.is_file() {
                validate_pack_format_mcmeta(&mcmeta_path, expected, &path)?;
            }
        }

        Ok(())
    }
}

fn valid_dependency_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
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
        || permissions.stdio;
    if plugin.security == SecurityMode::Sandboxed && has_permissions {
        return Err(Error::Config {
            path: path.to_path_buf(),
            message: format!(
                "plugin `{}` grants permissions but uses `security: \"sandboxed\"`",
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
        return Err("path must not contain `.` or prefix components");
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
fn default_wasm_memory_limit_mb() -> u32 {
    512
}

fn default_wasm_execution_deadline_seconds() -> u64 {
    60
}

fn default_limits_memory_limit_mb() -> u32 {
    256
}
fn default_limits_execution_deadline_seconds() -> u64 {
    60
}

#[cfg(test)]
mod tests {
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
        let mut config = Config::new("x");
        config.pack.pack_format = Some(34);
        let msg = config.validate_source(root).unwrap_err().to_string();
        assert!(msg.contains("does not match"), "{msg}");

        std::fs::write(
            root.join("src/pack.mcmeta"),
            r#"{"pack":{"pack_format":34}}"#,
        )
        .unwrap();
        config.validate_source(root).unwrap();
    }
}
