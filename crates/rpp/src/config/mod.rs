//! Project configuration: the `rpp.toml` schema (spec §1).
//!
//! TOML is the only supported project config format. The [`Config`] type mirrors
//! the documented schema. The `squash` and `dev` sections are parsed into plain
//! structs here and consumed by other crates (`rpp-squash`, the dev server).

mod source;

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
    /// Source descriptor (`path:...` or `github:owner/repo`).
    pub source: String,
    /// Optional git ref (tag/branch/sha) for GitHub sources.
    #[serde(default)]
    pub r#ref: Option<String>,
    /// Optional subdirectory within a GitHub repo.
    #[serde(default)]
    pub subdir: Option<String>,
    /// Arbitrary options passed to the plugin.
    #[serde(default = "empty_table")]
    pub options: toml::Value,
}

/// An empty TOML table; the default for plugin options.
pub(crate) fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

impl PluginConfig {
    /// Parse [`Self::source`] into a structured [`PluginSourceSpec`].
    pub fn parse_source(&self) -> std::result::Result<PluginSourceSpec, SourceParseError> {
        PluginSourceSpec::parse(&self.source, self.r#ref.as_deref(), self.subdir.as_deref())
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
            if let Err(e) = plugin.parse_source() {
                return Err(Error::Config {
                    path: path.to_path_buf(),
                    message: format!("plugin source `{}`: {e}", plugin.source),
                });
            }
        }
        Ok(())
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
            cfg.plugins[0].parse_source().unwrap(),
            PluginSourceSpec::Path { .. }
        ));
        assert!(matches!(
            cfg.plugins[1].parse_source().unwrap(),
            PluginSourceSpec::GitHub { .. }
        ));
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
}
