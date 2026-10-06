//! Configuration types for squash operations.
//!
//! These mirror the `build.squash` object in `rpp.config.ts` (see `docs/SPEC.md`
//! section 1); the CLI maps its parsed config onto them.

/// PNG optimization aggressiveness.
///
/// Maps onto the `png` config key: `false` -> [`PngLevel::Off`],
/// `"fast"` -> [`PngLevel::Fast`], `"max"` -> [`PngLevel::Max`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// The oxipng preset level for this setting, or `None` when disabled.
    pub(crate) fn preset(self) -> Option<u8> {
        match self {
            PngLevel::Off => None,
            PngLevel::Fast => Some(2),
            PngLevel::Max => Some(6),
        }
    }
}

/// Options controlling release archive optimization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquashOptions {
    /// Minify `.json` and `.mcmeta` files (parse -> compact re-serialize).
    pub json: bool,
    /// PNG optimization level.
    pub png: PngLevel,
    /// Glob patterns of files to leave out of the archive (e.g.
    /// `"**/.DS_Store"`, `"**/*.psd"`). Matched against forward-slash relative
    /// paths.
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
