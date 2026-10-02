//! Single-file optimization.

use crate::options::{PngLevel, SquashOptions};

/// Optimize a single file's bytes, dispatching by the extension of `path`:
/// - `.json` / `.mcmeta`: parse with `serde_json` and re-serialize compactly.
/// - `.png`: optimize with oxipng ([`PngLevel::Fast`] = preset 2,
///   [`PngLevel::Max`] = preset 6), always stripping safe metadata chunks.
///
/// Returns `Some(bytes)` only when the result is strictly smaller than the
/// input; otherwise `None` (file unchanged, not applicable, or it would have
/// grown). Recoverable problems (invalid JSON, PNG failures) push a message
/// into `warnings` and also yield `None`.
pub(crate) fn squash_file(
    path: &str,
    contents: &[u8],
    opts: &SquashOptions,
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    match Kind::of(path)? {
        Kind::Json if opts.json => squash_json(path, contents, warnings),
        Kind::Png if opts.png != PngLevel::Off => squash_png(path, contents, opts.png, warnings),
        _ => None,
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

fn squash_json(path: &str, contents: &[u8], warnings: &mut Vec<String>) -> Option<Vec<u8>> {
    let value: serde_json::Value = match serde_json::from_slice(contents) {
        Ok(value) => value,
        Err(err) => {
            warnings.push(format!(
                "{path}: invalid JSON, passed through unchanged ({err})"
            ));
            return None;
        }
    };
    let minified = serde_json::to_vec(&value).ok()?;
    (minified.len() < contents.len()).then_some(minified)
}

fn squash_png(
    path: &str,
    contents: &[u8],
    level: PngLevel,
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    let mut options = oxipng::Options::from_preset(level.preset()?);
    options.strip = oxipng::StripChunks::Safe;

    match oxipng::optimize_from_memory(contents, &options) {
        Ok(optimized) => (optimized.len() < contents.len()).then_some(optimized),
        Err(err) => {
            warnings.push(format!(
                "{path}: png optimization failed, kept original ({err})"
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(path: &str, contents: &[u8], opts: &SquashOptions) -> (Option<Vec<u8>>, Vec<String>) {
        let mut warnings = Vec::new();
        let out = squash_file(path, contents, opts, &mut warnings);
        (out, warnings)
    }

    /// A 16x16 RGBA PNG with redundant data that oxipng can shrink.
    fn make_png() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder.write_header().unwrap();
            let data = [40u8, 100, 180, 128].repeat(16 * 16);
            writer.write_image_data(&data).unwrap();
        }
        buf
    }

    fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut decoder = png::Decoder::new(bytes);
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().expect("output must decode as PNG");
        let mut out = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut out).expect("decode frame");
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        out.truncate(info.buffer_size());
        (info.width, info.height, out)
    }

    #[test]
    fn json_is_minified_when_smaller() {
        let input = b"{\n  \"a\": 1,\n  \"b\": [1, 2, 3]\n}\n";
        let (out, warnings) = run("foo.json", input, &SquashOptions::default());
        assert_eq!(out.unwrap(), br#"{"a":1,"b":[1,2,3]}"#);
        assert!(warnings.is_empty());
    }

    #[test]
    fn already_minified_json_returns_none() {
        let (out, _) = run("foo.json", br#"{"a":1}"#, &SquashOptions::default());
        assert!(out.is_none());
    }

    #[test]
    fn mcmeta_is_treated_as_json() {
        let input = b"{\n  \"pack\": {\n    \"pack_format\": 34\n  }\n}";
        let (out, _) = run("pack.mcmeta", input, &SquashOptions::default());
        assert_eq!(out.unwrap(), br#"{"pack":{"pack_format":34}}"#);
    }

    #[test]
    fn invalid_json_passes_through_with_warning() {
        let (out, warnings) = run("bad.json", b"{ nope ", &SquashOptions::default());
        assert!(out.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("bad.json"));
    }

    #[test]
    fn json_disabled_returns_none() {
        let opts = SquashOptions {
            json: false,
            ..Default::default()
        };
        assert!(run("foo.json", b"{\n  \"a\": 1\n}", &opts).0.is_none());
    }

    #[test]
    fn png_shrinks_and_preserves_pixels() {
        let input = make_png();
        for png in [PngLevel::Fast, PngLevel::Max] {
            let opts = SquashOptions {
                png,
                ..Default::default()
            };
            let out = run("tex.png", &input, &opts)
                .0
                .expect("redundant PNG shrinks");
            assert!(out.len() < input.len());
            assert_eq!(decode_png(&out), decode_png(&input));
        }
    }

    #[test]
    fn png_disabled_returns_none() {
        assert!(run("tex.png", &make_png(), &SquashOptions::default())
            .0
            .is_none());
    }
}
