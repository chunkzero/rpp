//! The config types, deserialized from the camelCase JSON `rpp.config.ts` exports.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The project configuration exported by `rpp.config.ts`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
}

/// `pack` section. RPP generates the pack's `pack.mcmeta` from it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackConfig {
    /// Pack name; used for the zip filename. Required.
    pub name: String,
    /// Text component shown in the pack list: a string, array, or object.
    #[serde(default = "empty_description")]
    pub description: serde_json::Value,
    /// Supported resource pack formats. Required.
    pub format: FormatRange,
    /// Overlay directories applied for a subset of formats, in order.
    #[serde(default)]
    pub overlays: Vec<OverlayConfig>,
    /// Files from lower packs hidden by this pack.
    #[serde(default)]
    pub filter: Vec<FilterPattern>,
    /// Languages added by this pack, keyed by language code.
    #[serde(default)]
    pub language: BTreeMap<String, LanguageConfig>,
}

fn empty_description() -> serde_json::Value {
    serde_json::Value::String(String::new())
}

/// An inclusive range of resource pack formats: `34` or `{ min: 34, max: 69 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FormatRange {
    /// Lowest supported format.
    pub min: u32,
    /// Highest supported format.
    pub max: u32,
}

impl<'de> Deserialize<'de> for FormatRange {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let format = |value: &serde_json::Value| {
            value.as_u64().and_then(|format| u32::try_from(format).ok())
        };
        let value = serde_json::Value::deserialize(deserializer)?;
        let range = match &value {
            serde_json::Value::Object(map) if map.len() == 2 => map
                .get("min")
                .and_then(format)
                .zip(map.get("max").and_then(format))
                .map(|(min, max)| FormatRange { min, max }),
            value => format(value).map(|format| FormatRange {
                min: format,
                max: format,
            }),
        };
        range.ok_or_else(|| {
            D::Error::custom(format!(
                "expected a format number or `{{ min, max }}` format numbers, got `{value}`"
            ))
        })
    }
}

/// One `pack.overlays` entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayConfig {
    /// Overlay directory at the pack root.
    pub directory: String,
    /// Formats the overlay applies to.
    pub format: FormatRange,
}

/// One `pack.filter` pattern. Both fields are regular expressions; an omitted field
/// matches everything.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilterPattern {
    /// Namespace pattern.
    pub namespace: Option<String>,
    /// Path pattern.
    pub path: Option<String>,
}

/// One `pack.language` entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LanguageConfig {
    /// Full language name.
    pub name: String,
    /// Country or region name.
    pub region: String,
    /// Whether the language reads right to left.
    #[serde(default)]
    pub bidirectional: bool,
}

/// Plugin runtime limits (`build.limits`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LimitsConfig {
    /// Per-plugin-runtime memory limit in megabytes.
    pub memory_limit_mb: u32,
    /// Maximum wall-clock execution time per plugin call, in seconds.
    pub execution_deadline_seconds: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            memory_limit_mb: 256,
            execution_deadline_seconds: 60,
        }
    }
}

/// WASM component limits (`build.wasm`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct WasmConfig {
    /// Per-instance linear-memory limit in megabytes.
    pub memory_limit_mb: u32,
    /// Maximum wall-clock execution time per component call, in seconds.
    pub execution_deadline_seconds: u64,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            memory_limit_mb: 512,
            execution_deadline_seconds: 60,
        }
    }
}

/// `build` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct BuildConfig {
    /// Pack source directory (contains `assets/`).
    pub source: PathBuf,
    /// Output directory; the zip is written to `<output>/<name>.zip`.
    pub output: PathBuf,
    /// Worker thread count; `0` means available parallelism.
    pub workers: usize,
    /// Plugin runtime limits.
    pub limits: LimitsConfig,
    /// WASM component limits.
    pub wasm: WasmConfig,
    /// Squash settings.
    pub squash: SquashConfig,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            source: PathBuf::from("src"),
            output: PathBuf::from("dist"),
            workers: 0,
            limits: LimitsConfig::default(),
            wasm: WasmConfig::default(),
            squash: SquashConfig::default(),
        }
    }
}

/// `build.squash` section. Consumed by `rpp-squash`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct SquashConfig {
    /// Whether squashing runs at all.
    pub enabled: bool,
    /// Optimization engine.
    pub engine: SquashEngine,
    /// Minify `.json`/`.mcmeta` in the output.
    pub json: bool,
    /// PNG optimization level: `false`, `"fast"`, or `"max"`.
    pub png: PngSetting,
    /// Produce `<output>/<name>.zip`.
    pub zip: bool,
    /// Glob patterns of files to strip from the output before zipping.
    pub strip: Vec<String>,
    /// PackSquash binary name/path (used when `engine` is `"packsquash"`).
    pub packsquash_binary: String,
    /// Optional passthrough options file for PackSquash.
    pub packsquash_options: Option<PathBuf>,
}

impl Default for SquashConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: SquashEngine::default(),
            json: true,
            png: PngSetting::default(),
            zip: true,
            strip: ["**/.DS_Store", "**/Thumbs.db", "**/*.psd", "**/*.xcf"]
                .map(String::from)
                .to_vec(),
            packsquash_binary: "packsquash".into(),
            packsquash_options: None,
        }
    }
}

/// Release optimization engine: `"builtin"` | `"packsquash"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SquashEngine {
    /// The built-in optimizer from `rpp-squash`.
    #[default]
    Builtin,
    /// The external PackSquash binary.
    Packsquash,
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
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct DevConfig {
    /// Host to bind the dev server to.
    pub host: String,
    /// Port to bind the dev server to.
    pub port: u16,
    /// Whether to open a browser on startup.
    pub open: bool,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8080,
            open: false,
        }
    }
}

/// One entry of `plugins`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginConfig {
    /// Dependency name from the project's `rpp.json` (the `plugin` key).
    #[serde(rename = "plugin")]
    pub package: String,
    /// Arbitrary JSON options passed to the plugin; an empty object by default.
    #[serde(default = "empty_object")]
    pub options: serde_json::Value,
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

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
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
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PluginPermissions {
    /// Executable names or absolute paths accepted by `process.run`.
    pub process: Vec<String>,
    /// Environment variable names visible to process calls and WASI components.
    pub environment: Vec<String>,
    /// Project-relative directories made readable to WASI components.
    pub read: Vec<PathBuf>,
    /// Project-relative directories made writable to WASI components.
    pub write: Vec<PathBuf>,
    /// Permit WASI sockets.
    pub network: bool,
    /// Permit WASI clocks and real time in plugin code.
    pub clocks: bool,
    /// Permit host-backed WASI randomness and real randomness in plugin code.
    pub random: bool,
    /// Permit inherited WASI stdout/stderr.
    pub stdio: bool,
}

impl PluginPermissions {
    /// Whether no capability is granted.
    pub(crate) fn is_empty(&self) -> bool {
        self.process.is_empty()
            && self.environment.is_empty()
            && self.read.is_empty()
            && self.write.is_empty()
            && !self.network
            && !self.clocks
            && !self.random
            && !self.stdio
    }
}
