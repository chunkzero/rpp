//! Single-file optimization.

use crate::error::Result;
use crate::options::{PngLevel, SquashOptions};

/// Optimize a single file's bytes in isolation.
///
/// Dispatches by the file extension of `path`:
/// - `.json` / `.mcmeta`: parse with `serde_json` and re-serialize compactly.
///   Invalid JSON is **not** an error — it is passed through and a warning is
///   surfaced (see below); the function returns `Ok(None)`.
/// - `.png`: optimize with oxipng ([`PngLevel::Fast`] = preset 2,
///   [`PngLevel::Max`] = preset 6), always stripping safe metadata chunks.
///
/// Returns `Ok(Some(bytes))` only when the result is **strictly smaller** than
/// the input; otherwise `Ok(None)` (file unchanged, not applicable, or it would
/// have grown). `path` is used only for extension dispatch; `contents` are the
/// bytes to optimize.
///
/// Warnings (e.g. invalid JSON) are emitted via `tracing::warn!` when the
/// `tracing` feature is enabled. Callers needing to *collect* warnings should
/// use [`crate::squash_dir`], which records them into the report.
pub fn squash_file(path: &str, contents: Vec<u8>, opts: &SquashOptions) -> Result<Option<Vec<u8>>> {
    let mut warnings = Vec::new();
    let out = squash_file_collecting(path, contents, opts, &mut warnings)?;
    for warning in warnings {
        emit_warning(&warning);
    }
    Ok(out)
}

/// Like [`squash_file`] but pushes warnings into `warnings` instead of emitting
/// them, so directory walks can aggregate them into the report.
pub(crate) fn squash_file_collecting(
    path: &str,
    contents: Vec<u8>,
    opts: &SquashOptions,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<u8>>> {
    match Kind::of(path) {
        Some(Kind::Json) if opts.json => Ok(squash_json(path, &contents, warnings)),
        Some(Kind::Png) if opts.png.is_enabled() => {
            Ok(squash_png(path, &contents, opts.png, warnings))
        }
        _ => Ok(None),
    }
}

/// File categories squash knows how to optimize.
enum Kind {
    Json,
    Png,
}

impl Kind {
    fn of(path: &str) -> Option<Kind> {
        let ext = path.rsplit('.').next().filter(|_| path.contains('.'))?;
        match ext.to_ascii_lowercase().as_str() {
            "json" | "mcmeta" => Some(Kind::Json),
            "png" => Some(Kind::Png),
            _ => None,
        }
    }
}

/// Minify JSON; returns `Some` only if strictly smaller. Invalid JSON records a
/// warning and returns `None`.
fn squash_json(path: &str, contents: &[u8], warnings: &mut Vec<String>) -> Option<Vec<u8>> {
    let value: serde_json::Value = match serde_json::from_slice(contents) {
        Ok(v) => v,
        Err(err) => {
            warnings.push(format!(
                "{path}: invalid JSON, passed through unchanged ({err})"
            ));
            return None;
        }
    };
    // Compact serialization. serde_json never fails serializing a Value it
    // parsed, but handle the Result without unwrapping regardless.
    let minified = serde_json::to_vec(&value).ok()?;
    if minified.len() < contents.len() {
        Some(minified)
    } else {
        None
    }
}

/// Optimize a PNG; returns `Some` only if strictly smaller. Any oxipng failure
/// yields `None` (the original is kept).
fn squash_png(
    path: &str,
    contents: &[u8],
    level: PngLevel,
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    let preset = level.preset()?;
    let mut options = oxipng::Options::from_preset(preset);
    // Strip metadata that does not affect image display (safe, lossless).
    options.strip = oxipng::StripChunks::Safe;

    match oxipng::optimize_from_memory(contents, &options) {
        Ok(optimized) if optimized.len() < contents.len() => Some(optimized),
        Ok(_) => None,
        Err(err) => {
            warnings.push(format!(
                "{path}: png optimization failed, kept original ({err})"
            ));
            None
        }
    }
}

/// Emit a warning via tracing when enabled; no-op otherwise.
fn emit_warning(message: &str) {
    #[cfg(feature = "tracing")]
    tracing::warn!("{message}");
    #[cfg(not(feature = "tracing"))]
    let _ = message;
}
