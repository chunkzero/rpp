//! Configuration types for squash operations.
//!
//! These map directly onto the `[build.squash]` table in `rpp.toml` (see
//! `docs/SPEC.md` section 1). They derive [`serde`] traits so consumers can
//! deserialize user config straight into them.

use serde::{Deserialize, Serialize};

/// PNG optimization aggressiveness.
///
/// Maps onto the `png` config key: `false` -> [`PngLevel::Off`],
/// `"fast"` -> [`PngLevel::Fast`], `"max"` -> [`PngLevel::Max`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PngLevel {
    /// PNG optimization disabled.
    #[default]
    Off,
    /// oxipng preset 2 — quick, modest savings.
    Fast,
    /// oxipng preset 6 — slower, maximum lossless savings (Zopfli kept off).
    Max,
}

impl PngLevel {
    /// Returns the oxipng preset level for this setting, or `None` when disabled.
    pub fn preset(self) -> Option<u8> {
        match self {
            PngLevel::Off => None,
            PngLevel::Fast => Some(2),
            PngLevel::Max => Some(6),
        }
    }

    /// Whether PNG optimization is enabled.
    pub fn is_enabled(self) -> bool {
        !matches!(self, PngLevel::Off)
    }
}

/// Options controlling single-file and directory squash operations.
///
/// Construct with [`SquashOptions::builder`] or via [`Default`] and field
/// assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SquashOptions {
    /// Minify `.json` and `.mcmeta` files (parse -> compact re-serialize).
    pub json: bool,
    /// PNG optimization level.
    pub png: PngLevel,
    /// Glob patterns of files to delete from the directory before/while squashing
    /// (e.g. `"**/.DS_Store"`, `"**/*.psd"`). Matched against forward-slash
    /// relative paths.
    pub strip: Vec<String>,
}

impl Default for SquashOptions {
    fn default() -> Self {
        Self {
            json: true,
            png: PngLevel::Off,
            strip: Vec::new(),
        }
    }
}

impl SquashOptions {
    /// Start building a [`SquashOptions`].
    pub fn builder() -> SquashOptionsBuilder {
        SquashOptionsBuilder::default()
    }
}

/// Builder for [`SquashOptions`].
#[derive(Debug, Clone, Default)]
pub struct SquashOptionsBuilder {
    json: Option<bool>,
    png: Option<PngLevel>,
    strip: Vec<String>,
}

impl SquashOptionsBuilder {
    /// Enable or disable JSON minification.
    pub fn json(mut self, json: bool) -> Self {
        self.json = Some(json);
        self
    }

    /// Set the PNG optimization level.
    pub fn png(mut self, png: PngLevel) -> Self {
        self.png = Some(png);
        self
    }

    /// Add a single strip glob pattern.
    pub fn strip_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.strip.push(pattern.into());
        self
    }

    /// Replace the full list of strip glob patterns.
    pub fn strip(mut self, patterns: impl IntoIterator<Item = String>) -> Self {
        self.strip = patterns.into_iter().collect();
        self
    }

    /// Finish building.
    pub fn build(self) -> SquashOptions {
        let defaults = SquashOptions::default();
        SquashOptions {
            json: self.json.unwrap_or(defaults.json),
            png: self.png.unwrap_or(defaults.png),
            strip: self.strip,
        }
    }
}

/// Options controlling deterministic zip creation.
///
/// Defaults produce a byte-reproducible deflate archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZipOptions {
    /// Deflate compression level (0-9). `None` uses the `zip` crate default.
    pub compression_level: Option<i64>,
    /// Optional archive comment written into the zip end-of-central-directory.
    /// Kept empty by default to preserve byte-reproducibility across machines.
    pub comment: String,
}

impl Default for ZipOptions {
    fn default() -> Self {
        Self {
            compression_level: Some(9),
            comment: String::new(),
        }
    }
}

impl ZipOptions {
    /// Start building a [`ZipOptions`].
    pub fn builder() -> ZipOptionsBuilder {
        ZipOptionsBuilder::default()
    }
}

/// Builder for [`ZipOptions`].
#[derive(Debug, Clone, Default)]
pub struct ZipOptionsBuilder {
    compression_level: Option<i64>,
    comment: Option<String>,
}

impl ZipOptionsBuilder {
    /// Set the deflate compression level (0-9).
    pub fn compression_level(mut self, level: i64) -> Self {
        self.compression_level = Some(level);
        self
    }

    /// Set the archive comment. Leave empty for reproducible output.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Finish building.
    pub fn build(self) -> ZipOptions {
        let defaults = ZipOptions::default();
        ZipOptions {
            compression_level: self.compression_level.or(defaults.compression_level),
            comment: self.comment.unwrap_or(defaults.comment),
        }
    }
}
