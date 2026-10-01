//! End-to-end tests of the checked-in example pack and plugins, driven through the real
//! `rpp` binary against temporary copies so the examples' own outputs stay untouched.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if ["target", ".rpp", "dist", "generated", "guest"]
            .iter()
            .any(|skip| name == *skip)
        {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Copies `examples/pack` to `<tmp>/pack` and the shared plugins beside it, so the pack's
/// `path:../plugins/...` dependencies resolve. Returns the pack root.
fn copy_pack(tmp: &Path) -> PathBuf {
    copy_dir(&examples().join("pack"), &tmp.join("pack"));
    copy_dir(&examples().join("plugins"), &tmp.join("plugins"));
    tmp.join("pack")
}

fn build(root: &Path, args: &[&str]) -> Output {
    common::command(root)
        .arg("build")
        .args(args)
        .output()
        .expect("run rpp build")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn example_pack_builds_and_caches() {
    let tmp = tempfile::tempdir().unwrap();
    let root = copy_pack(tmp.path());

    let out = build(&root, &["--jobs", "2"]);
    assert!(
        out.status.success(),
        "first build failed:\n{}",
        stderr(&out)
    );

    // json-minify: the whitespace-heavy block model is compact but intact.
    let model =
        std::fs::read_to_string(root.join("dist/assets/minecraft/models/block/rpp_bricks.json"))
            .unwrap();
    assert!(
        !model.contains('\n') && !model.contains("  "),
        "not minified: {model:?}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&model).unwrap();
    assert_eq!(parsed["parent"], "minecraft:block/cube_all");
    assert_eq!(parsed["textures"]["all"], "minecraft:block/rpp_bricks");

    // hash-rename: the custom texture is fingerprinted, vanilla ones keep their names.
    assert!(!root
        .join("dist/assets/minecraft/textures/custom/gem.png")
        .exists());
    assert!(root
        .join("dist/assets/minecraft/textures/item/diamond_sword.png")
        .is_file());
    let map = read_json(&root.join("dist/rename_map.json"));
    let hashed = map["assets/minecraft/textures/custom/gem.png"]
        .as_str()
        .unwrap();
    assert!(root.join("dist").join(hashed).is_file(), "{hashed} missing");
    assert!(
        !root.join("dist/notes/design.txt").exists(),
        ".rppignore is honored"
    );

    // catalog: references are repaired and assets generated from the discovered items.
    let reference = format!(
        "minecraft:{}",
        hashed
            .strip_prefix("assets/minecraft/textures/")
            .unwrap()
            .strip_suffix(".png")
            .unwrap()
    );
    let dist = |rel: &str| read_json(&root.join("dist").join(rel));
    assert_eq!(
        dist("assets/minecraft/models/item/magic_gem.json")["textures"]["layer0"],
        reference
    );
    assert_eq!(
        dist("assets/rpp/models/item/ember_gem.json")["textures"]["layer0"],
        reference
    );
    assert_eq!(
        dist("assets/rpp/lang/en_us.json")["item.rpp.ember_gem"],
        "Ember Gem"
    );
    let catalog = read_json(&root.join("generated/catalog/items.json"));
    assert_eq!(catalog["items"]["ember_gem"]["model"], "rpp:item/ember_gem");
    assert_eq!(catalog["items"]["ember_gem"]["texture"], reference);
    assert!(!root.join("dist/items/ember_gem.ts").exists());

    // Release zip: pack.mcmeta first, no authoring inputs or external outputs inside.
    let zip_path = root.join("dist/rpp-example-pack.zip");
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    assert_eq!(archive.by_index(0).unwrap().name(), "pack.mcmeta");
    assert!(archive.by_name("items/ember_gem.ts").is_err());
    assert!(archive.by_name("generated/catalog/items.json").is_err());
    assert!(archive
        .by_name("assets/rpp/models/item/ember_gem.json")
        .is_ok());
    let first_zip = std::fs::read(&zip_path).unwrap();

    // An unchanged build replays everything from the cache.
    let out = build(&root, &["--jobs", "2"]);
    let report = stderr(&out);
    assert!(out.status.success(), "{report}");
    assert!(
        report.contains("processed 0"),
        "second build must be cached: {report}"
    );
    assert_eq!(
        std::fs::read(&zip_path).unwrap(),
        first_zip,
        "zip must be reproducible"
    );

    // Editing one file reprocesses only that file.
    std::fs::write(
        root.join("src/assets/minecraft/lang/en_us.json"),
        "{ \"pack.rpp.example.title\" : \"Edited Title\" }",
    )
    .unwrap();
    let out = build(&root, &["--jobs", "2"]);
    let report = stderr(&out);
    assert!(out.status.success(), "{report}");
    assert!(
        report.contains("processed 1,"),
        "only the edited file reprocesses: {report}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("dist/assets/minecraft/lang/en_us.json")).unwrap(),
        "{\"pack.rpp.example.title\":\"Edited Title\"}"
    );

    // Renaming an item definition removes its stale assets and catalog entry.
    std::fs::remove_file(root.join("src/items/ember_gem.ts")).unwrap();
    std::fs::write(
        root.join("src/items/frost_gem.ts"),
        "export default { name: \"Frost Gem\", texture: \"minecraft:custom/gem\" };\n",
    )
    .unwrap();
    let out = build(&root, &["--jobs", "2"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!root
        .join("dist/assets/rpp/models/item/ember_gem.json")
        .exists());
    assert!(root
        .join("dist/assets/rpp/models/item/frost_gem.json")
        .is_file());
    let language = dist("assets/rpp/lang/en_us.json");
    assert_eq!(language["item.rpp.frost_gem"], "Frost Gem");
    assert!(language.get("item.rpp.ember_gem").is_none());
    let updated = read_json(&root.join("generated/catalog/items.json"));
    assert!(updated["items"].get("ember_gem").is_none());
    assert_eq!(updated["items"]["frost_gem"]["model"], "rpp:item/frost_gem");
}

#[test]
fn mcmeta_validate_fails_on_bad_animation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = copy_pack(tmp.path());
    std::fs::write(
        root.join("src/assets/minecraft/textures/block/ember.png.mcmeta"),
        "{ \"animation\": { \"frametime\": 0 } }",
    )
    .unwrap();

    let out = build(&root, &[]);
    let report = stderr(&out);
    assert!(!out.status.success());
    assert!(report.contains("frametime"), "{report}");
}

#[test]
fn hash_rename_uses_processed_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    copy_dir(
        &examples().join("plugins/hash-rename"),
        &root.join("plugins/hash-rename"),
    );
    let write = |rel: &str, contents: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };
    write(
        "rpp.json",
        r#"{ "dependencies": { "modify": "path:plugins/modify", "hash-rename": "path:plugins/hash-rename" } }"#,
    );
    write(
        "rpp.config.ts",
        r##"import { defineConfig, plugin } from "#rpp/config";
import hashRename from "#plugins/hash-rename";

export default defineConfig({
  pack: { name: "processed-hash" },
  build: { squash: { enabled: false } },
  plugins: [plugin("modify"), hashRename({ files: ["assets/*/textures/custom/**/*.png"] })],
});
"##,
    );
    write(
        "plugins/modify/rpp.json",
        r#"{ "name": "modify", "version": "1.0.0", "entry": "plugin.ts" }"#,
    );
    write(
        "plugins/modify/plugin.ts",
        r##"import { definePlugin } from "#rpp";

export default definePlugin({
  processors: {
    modify: { files: "**/*.png", priority: 5, run(_ctx, file) { file.text = file.text.toUpperCase(); } },
  },
});
"##,
    );
    write("src/assets/test/textures/custom/gem.png", "raw");

    let out = build(root, &[]);
    assert!(out.status.success(), "build failed:\n{}", stderr(&out));
    let map = read_json(&root.join("dist/rename_map.json"));
    let hashed = map["assets/test/textures/custom/gem.png"].as_str().unwrap();
    assert_eq!(
        std::fs::read(root.join("dist").join(hashed)).unwrap(),
        b"RAW"
    );
    assert!(!root
        .join("dist/assets/test/textures/custom/gem.png")
        .exists());
}

fn wasip2_available() -> bool {
    let Ok(output) = Command::new("rustc").args(["--print", "sysroot"]).output() else {
        return false;
    };
    Path::new(String::from_utf8_lossy(&output.stdout).trim())
        .join("lib/rustlib/wasm32-wasip2/lib")
        .is_dir()
}

fn build_guest(target_dir: &Path) -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-wasip2",
        ])
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(examples().join("plugins/grayscale-wasm/guest"))
        .output()
        .expect("build grayscale guest");
    assert!(
        output.status.success(),
        "guest build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    target_dir.join("wasm32-wasip2/release/grayscale_wasm_guest.wasm")
}

fn encode_png(
    color: png::ColorType,
    size: (u32, u32),
    data: &[u8],
    trns: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size.0, size.1);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    if let Some(trns) = trns {
        encoder.set_trns(trns);
    }
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    out
}

fn decode_gray_alpha(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let mut reader = png::Decoder::new(bytes.as_slice()).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::GrayscaleAlpha);
    pixels.truncate(info.buffer_size());
    pixels
}

#[test]
fn grayscale_example_converts_textures() {
    if !wasip2_available() {
        eprintln!("SKIP: wasm32-wasip2 target is unavailable");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let plugin = root.join("plugins/grayscale-wasm");
    copy_dir(&examples().join("plugins/grayscale-wasm"), &plugin);
    std::fs::copy(
        build_guest(&tmp.path().join("guest-target")),
        plugin.join("grayscale.wasm"),
    )
    .unwrap();

    let write = |rel: &str, contents: &[u8]| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };
    write(
        "rpp.json",
        br#"{ "dependencies": { "grayscale-wasm": "path:plugins/grayscale-wasm" } }"#,
    );
    write(
        "rpp.config.ts",
        br##"import { defineConfig, plugin } from "#rpp/config";

export default defineConfig({
  pack: { name: "gray", packFormat: 34 },
  build: { workers: 1 },
  plugins: [plugin("grayscale-wasm")],
});
"##,
    );
    write(
        "src/pack.mcmeta",
        br#"{"pack":{"pack_format":34,"description":""}}"#,
    );
    let (red, translucent_blue) = ([255, 0, 0, 255], [0, 0, 255, 128]);
    write(
        "src/assets/minecraft/textures/block/stone.png",
        &encode_png(
            png::ColorType::Rgba,
            (2, 1),
            &[red, translucent_blue].concat(),
            None,
        ),
    );
    write(
        "src/assets/minecraft/textures/block/transparent.png",
        &encode_png(
            png::ColorType::Rgb,
            (2, 1),
            &[255, 0, 0, 0, 255, 0],
            Some(vec![0, 255, 0, 0, 0, 0]),
        ),
    );

    let out = build(&root, &["--no-squash"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let textures = root.join("dist/assets/minecraft/textures/block");
    // luma(red) = 76, alpha kept; luma(blue) = 29, alpha 128.
    assert_eq!(
        decode_gray_alpha(&textures.join("stone.png")),
        [76, 255, 29, 128]
    );
    assert_eq!(
        decode_gray_alpha(&textures.join("transparent.png")),
        [76, 0, 149, 255]
    );

    let out = build(&root, &["--no-squash"]);
    assert!(stderr(&out).contains("processed 0"), "{}", stderr(&out));
}
