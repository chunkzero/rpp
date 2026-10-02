//! Integration tests for rpp-squash (offline).

use std::fs;
use std::io::Read;
use std::path::Path;

use rpp_squash::{run_packsquash, squash_dir, write_zip, PngLevel, SquashOptions};

// ---------------------------------------------------------------------------
// squash_dir end-to-end
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
        // A solid, translucent color is compressible and exercises alpha preservation.
        let data = [40u8, 100, 180, 128].repeat(16 * 16);
        writer.write_image_data(&data).unwrap();
    }
    buf
}

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

    let opts = SquashOptions {
        png: PngLevel::Fast,
        strip: vec!["**/.DS_Store".into()],
        ..Default::default()
    };

    let report = squash_dir(root, &opts).unwrap();

    // .DS_Store removed.
    assert!(!root.join(".DS_Store").exists());
    assert_eq!(report.files_stripped, 1);

    // a.json, pack.mcmeta, tex.png optimized = 3 (broken.json not, already-min not).
    assert_eq!(report.files_optimized, 3, "report: {report:?}");
    assert!(report.bytes_after < report.bytes_before);

    // a.json minified on disk.
    let on_disk = fs::read(root.join("assets/a.json")).unwrap();
    assert_eq!(on_disk, br#"{"x":1,"y":2}"#);

    // broken.json untouched + produced a warning.
    assert_eq!(fs::read(root.join("broken.json")).unwrap(), b"{ not json ");
    assert_eq!(report.warnings.len(), 1, "warnings: {:?}", report.warnings);
    assert!(report.warnings[0].contains("broken.json"));
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
    write_zip(root, &za).unwrap();
    write_zip(root, &zb).unwrap();
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
    write_zip(root, &zip_path).unwrap();

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
    write_zip(root, &zip_path).unwrap();

    let reader = std::io::Cursor::new(fs::read(&zip_path).unwrap());
    let mut archive = zip::ZipArchive::new(reader).unwrap();
    let paths = [
        "pack.mcmeta",
        "assets/minecraft/apple.json",
        "assets/minecraft/zebra.json",
        "readme.txt",
    ];
    assert_eq!(archive.len(), paths.len());
    for path in paths {
        let mut contents = Vec::new();
        archive
            .by_name(path)
            .unwrap()
            .read_to_end(&mut contents)
            .unwrap();
        assert_eq!(contents, fs::read(root.join(path)).unwrap(), "{path}");
    }
}

// ---------------------------------------------------------------------------
// PackSquash
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires a real packsquash binary on PATH"]
fn packsquash_real_invocation() {
    let dir = tempfile::tempdir().unwrap();
    let metadata = br#"{"pack":{"pack_format":34,"description":"PackSquash integration test"}}"#;
    let language = br#"{"item.rpp.test":"Test item"}"#;
    fs::write(dir.path().join("pack.mcmeta"), metadata).unwrap();
    fs::create_dir_all(dir.path().join("assets/rpp/lang")).unwrap();
    fs::write(dir.path().join("assets/rpp/lang/en_us.json"), language).unwrap();
    let output_dir = tempfile::tempdir().unwrap();
    let out = output_dir.path().join("out.zip");
    run_packsquash("packsquash", dir.path(), &out, None).unwrap();

    let mut archive = zip::ZipArchive::new(fs::File::open(out).unwrap()).unwrap();
    for (path, expected) in [
        ("pack.mcmeta", metadata.as_slice()),
        ("assets/rpp/lang/en_us.json", language.as_slice()),
    ] {
        let actual: serde_json::Value =
            serde_json::from_reader(archive.by_name(path).unwrap()).unwrap();
        let expected: serde_json::Value = serde_json::from_slice(expected).unwrap();
        assert_eq!(actual, expected, "archive contents differ for {path}");
    }
}
