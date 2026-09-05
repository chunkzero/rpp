//! Integration tests for rpp-squash (offline).

use std::fs;
use std::io::Read;
use std::path::Path;

use rpp_squash::{
    run_packsquash, squash_dir, squash_file, write_zip, Error, PngLevel, SquashOptions, ZipOptions,
};

// ---------------------------------------------------------------------------
// JSON minification
// ---------------------------------------------------------------------------

#[test]
fn json_is_minified_when_smaller() {
    let opts = SquashOptions::builder().json(true).build();
    let input = b"{\n  \"a\": 1,\n  \"b\": [1, 2, 3]\n}\n".to_vec();
    let out = squash_file("foo.json", input.clone(), &opts).unwrap();
    let out = out.expect("pretty JSON should minify smaller");
    assert!(out.len() < input.len());
    assert_eq!(out, br#"{"a":1,"b":[1,2,3]}"#);
}

#[test]
fn already_minified_json_returns_none() {
    let opts = SquashOptions::builder().json(true).build();
    let input = br#"{"a":1}"#.to_vec();
    assert!(squash_file("foo.json", input, &opts).unwrap().is_none());
}

#[test]
fn mcmeta_is_treated_as_json() {
    let opts = SquashOptions::builder().json(true).build();
    let input = b"{\n  \"pack\": {\n    \"pack_format\": 34\n  }\n}".to_vec();
    let out = squash_file("pack.mcmeta", input, &opts).unwrap();
    assert_eq!(out.unwrap(), br#"{"pack":{"pack_format":34}}"#);
}

#[test]
fn invalid_json_passes_through_with_warning() {
    let opts = SquashOptions::builder().json(true).build();
    // squash_file emits warnings via tracing only; verify it does not error and
    // returns None (passthrough). The collecting path is exercised via squash_dir.
    let input = b"{ this is not valid json ".to_vec();
    assert!(squash_file("bad.json", input, &opts).unwrap().is_none());
}

#[test]
fn json_disabled_returns_none() {
    let opts = SquashOptions::builder().json(false).build();
    let input = b"{\n  \"a\": 1\n}".to_vec();
    assert!(squash_file("foo.json", input, &opts).unwrap().is_none());
}

// ---------------------------------------------------------------------------
// PNG optimization
// ---------------------------------------------------------------------------

/// Build a 16x16 RGBA PNG with redundant data that oxipng can shrink.
fn make_png() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // No filtering / fast compression so the encoder leaves plenty for oxipng.
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().unwrap();
        // Solid color: 16*16*4 bytes, all identical -> highly compressible and
        // oxipng will reduce the color type / palette.
        let data = vec![128u8; 16 * 16 * 4];
        writer.write_image_data(&data).unwrap();
    }
    buf
}

fn is_valid_png(bytes: &[u8]) -> (u32, u32) {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().expect("output must decode as PNG");
    let mut out = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut out).expect("decode frame");
    (info.width, info.height)
}

#[test]
fn png_fast_shrinks_and_stays_valid() {
    let opts = SquashOptions::builder().png(PngLevel::Fast).build();
    let input = make_png();
    let out = squash_file("tex.png", input.clone(), &opts).unwrap();
    let out = out.expect("oxipng should shrink a redundant PNG");
    assert!(out.len() < input.len(), "expected smaller output");
    assert_eq!(is_valid_png(&out), (16, 16));
}

#[test]
fn png_max_shrinks_and_stays_valid() {
    let opts = SquashOptions::builder().png(PngLevel::Max).build();
    let input = make_png();
    let out = squash_file("tex.png", input.clone(), &opts).unwrap();
    let out = out.expect("oxipng max should shrink a redundant PNG");
    assert!(out.len() < input.len());
    assert_eq!(is_valid_png(&out), (16, 16));
}

#[test]
fn png_disabled_returns_none() {
    let opts = SquashOptions::builder().png(PngLevel::Off).build();
    assert!(squash_file("tex.png", make_png(), &opts).unwrap().is_none());
}

// ---------------------------------------------------------------------------
// squash_dir end-to-end
// ---------------------------------------------------------------------------

#[test]
fn squash_dir_optimizes_strips_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("assets")).unwrap();
    let pretty_json = b"{\n  \"x\": 1,\n  \"y\": 2\n}\n";
    fs::write(root.join("assets/a.json"), pretty_json).unwrap();
    fs::write(root.join("pack.mcmeta"), b"{\n  \"pack\": 1\n}").unwrap();
    fs::write(root.join("tex.png"), make_png()).unwrap();
    // Strippable junk + invalid JSON for the warning path.
    fs::write(root.join(".DS_Store"), b"junk").unwrap();
    fs::write(root.join("broken.json"), b"{ not json ").unwrap();

    let opts = SquashOptions::builder()
        .json(true)
        .png(PngLevel::Fast)
        .strip_pattern("**/.DS_Store")
        .build();

    let report = squash_dir(root, &opts).unwrap();

    // .DS_Store removed.
    assert!(!root.join(".DS_Store").exists());
    assert_eq!(report.files_stripped, 1);

    // a.json, pack.mcmeta, tex.png optimized = 3 (broken.json not, already-min not).
    assert_eq!(report.files_optimized, 3, "report: {report}");
    assert!(report.bytes_after < report.bytes_before);
    assert!(report.bytes_saved() > 0);

    // a.json minified on disk.
    let on_disk = fs::read(root.join("assets/a.json")).unwrap();
    assert_eq!(on_disk, br#"{"x":1,"y":2}"#);

    // broken.json untouched + produced a warning.
    assert_eq!(fs::read(root.join("broken.json")).unwrap(), b"{ not json ");
    assert_eq!(report.warnings.len(), 1, "warnings: {:?}", report.warnings);
    assert!(report.warnings[0].contains("broken.json"));

    // Display summary renders.
    let summary = report.to_string();
    assert!(summary.contains("optimized"));
}

// ---------------------------------------------------------------------------
// Deterministic zip
// ---------------------------------------------------------------------------

fn build_tree(root: &Path) {
    fs::create_dir_all(root.join("assets/minecraft")).unwrap();
    fs::write(root.join("pack.mcmeta"), br#"{"pack":{"pack_format":34}}"#).unwrap();
    fs::write(root.join("assets/minecraft/zebra.json"), b"z").unwrap();
    fs::write(root.join("assets/minecraft/apple.json"), b"a").unwrap();
    fs::write(root.join("readme.txt"), b"hi").unwrap();
}

fn zip_entry_names(zip_bytes: &[u8]) -> Vec<String> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).unwrap();
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect()
}

#[test]
fn zip_is_byte_identical_across_runs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    build_tree(root);

    // Write the archives to a directory *outside* the source tree, otherwise the
    // first archive would be picked up when walking for the second.
    let out = tempfile::tempdir().unwrap();
    let za = out.path().join("a.zip");
    let zb = out.path().join("b.zip");
    write_zip(root, &za, &ZipOptions::default()).unwrap();
    write_zip(root, &zb, &ZipOptions::default()).unwrap();
    let a = fs::read(&za).unwrap();
    let b = fs::read(&zb).unwrap();
    assert_eq!(a, b, "two zips of the same tree must be byte-identical");
}

#[test]
fn zip_orders_pack_mcmeta_first_then_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    build_tree(root);

    let out = tempfile::tempdir().unwrap();
    let zip_path = out.path().join("pack.zip");
    write_zip(root, &zip_path, &ZipOptions::default()).unwrap();

    let names = zip_entry_names(&fs::read(&zip_path).unwrap());
    assert_eq!(names.first().map(String::as_str), Some("pack.mcmeta"));

    // Remaining entries sorted lexicographically.
    let rest: Vec<&str> = names[1..].iter().map(String::as_str).collect();
    let mut sorted = rest.clone();
    sorted.sort_unstable();
    assert_eq!(rest, sorted, "entries after pack.mcmeta must be sorted");

    // Forward-slash names.
    assert!(names.iter().any(|n| n == "assets/minecraft/apple.json"));
}

#[test]
fn zip_roundtrips_contents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    build_tree(root);
    let out = tempfile::tempdir().unwrap();
    let zip_path = out.path().join("pack.zip");
    write_zip(root, &zip_path, &ZipOptions::default()).unwrap();

    let reader = std::io::Cursor::new(fs::read(&zip_path).unwrap());
    let mut archive = zip::ZipArchive::new(reader).unwrap();
    let mut file = archive.by_name("readme.txt").unwrap();
    let mut s = String::new();
    file.read_to_string(&mut s).unwrap();
    assert_eq!(s, "hi");
}

// ---------------------------------------------------------------------------
// PackSquash
// ---------------------------------------------------------------------------

#[test]
fn packsquash_missing_binary_errors_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let err = run_packsquash(
        "rpp-definitely-not-a-real-binary",
        dir.path(),
        &dir.path().join("out.zip"),
        None,
    )
    .unwrap_err();
    assert!(matches!(err, Error::PackSquashNotFound));
    // Error message must guide the user.
    assert!(err.to_string().contains("builtin"));
}

#[test]
fn packsquash_generates_options_file() {
    let pack = Path::new("/tmp/some pack");
    let zip = Path::new("/tmp/out.zip");
    let body = rpp_squash::render_options(pack, zip);
    assert!(body.contains("pack_directory = \"/tmp/some pack\""));
    assert!(body.contains("output_file_path = \"/tmp/out.zip\""));
}

#[test]
#[ignore = "requires a real packsquash binary on PATH"]
fn packsquash_real_invocation() {
    let dir = tempfile::tempdir().unwrap();
    build_tree(dir.path());
    let out = dir.path().join("out.zip");
    run_packsquash("packsquash", dir.path(), &out, None).unwrap();
    assert!(out.exists());
}
