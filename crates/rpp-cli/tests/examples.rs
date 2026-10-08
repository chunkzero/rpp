//! End-to-end tests of the checked-in example pack and plugins, driven through the real
//! `rpp` binary against temporary copies so the examples' own outputs stay untouched.

mod common;

use std::path::{Path, PathBuf};

use common::{build, copy_dir, examples_dir, read_json, stderr, write};

/// Copies `examples/pack` to `<tmp>/pack` and the shared plugins beside it, so the pack's
/// `path:../plugins/...` dependencies resolve. Returns the pack root.
fn copy_pack(tmp: &Path) -> PathBuf {
    copy_dir(&examples_dir().join("pack"), &tmp.join("pack"));
    copy_dir(&examples_dir().join("plugins"), &tmp.join("plugins"));
    tmp.join("pack")
}

fn dist_json(root: &Path, rel: &str) -> serde_json::Value {
    read_json(&root.join("dist").join(rel))
}

/// json-minify: the whitespace-heavy block model is compact but intact.
fn assert_block_model_minified(root: &Path) {
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
}

/// hash-rename: the custom texture is fingerprinted, vanilla ones keep their names.
/// Returns the hashed output path of the custom texture.
fn assert_texture_hashed(root: &Path) -> String {
    assert!(!root
        .join("dist/assets/minecraft/textures/custom/gem.png")
        .exists());
    assert!(root
        .join("dist/assets/minecraft/textures/item/diamond_sword.png")
        .is_file());
    let map = dist_json(root, "rename_map.json");
    let hashed = map["assets/minecraft/textures/custom/gem.png"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        root.join("dist").join(&hashed).is_file(),
        "{hashed} missing"
    );
    assert!(
        !root.join("dist/notes/design.txt").exists(),
        ".rppignore is honored"
    );
    hashed
}

/// catalog: references are repaired and assets generated from the discovered items.
fn assert_catalog_generated(root: &Path, hashed: &str) {
    let reference = format!(
        "minecraft:{}",
        hashed
            .strip_prefix("assets/minecraft/textures/")
            .unwrap()
            .strip_suffix(".png")
            .unwrap()
    );
    assert_eq!(
        dist_json(root, "assets/minecraft/models/item/magic_gem.json")["textures"]["layer0"],
        reference
    );
    assert_eq!(
        dist_json(root, "assets/rpp/models/item/ember_gem.json")["textures"]["layer0"],
        reference
    );
    assert_eq!(
        dist_json(root, "assets/rpp/lang/en_us.json")["item.rpp.ember_gem"],
        "Ember Gem"
    );
    let catalog = read_json(&root.join("generated/catalog/items.json"));
    assert_eq!(catalog["items"]["ember_gem"]["model"], "rpp:item/ember_gem");
    assert_eq!(catalog["items"]["ember_gem"]["texture"], reference);
    assert!(!root.join("dist/items/ember_gem.ts").exists());
}

/// Release zip: pack.mcmeta first, no authoring inputs or external outputs inside.
fn assert_release_zip_contents(zip_path: &Path) {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path).unwrap()).unwrap();
    assert_eq!(archive.by_index(0).unwrap().name(), "pack.mcmeta");
    assert!(archive.by_name("items/ember_gem.ts").is_err());
    assert!(archive.by_name("generated/catalog/items.json").is_err());
    assert!(archive
        .by_name("assets/rpp/models/item/ember_gem.json")
        .is_ok());
}

/// An unchanged build replays everything from the cache.
fn assert_unchanged_build_replays(root: &Path, zip_path: &Path, first_zip: &[u8]) {
    let out = build(root, &["--jobs", "2"]);
    let report = stderr(&out);
    assert!(out.status.success(), "{report}");
    assert!(
        report.contains("processed 0"),
        "second build must be cached: {report}"
    );
    assert_eq!(
        std::fs::read(zip_path).unwrap(),
        first_zip,
        "zip must be reproducible"
    );
}

/// Editing one file reprocesses only that file.
fn assert_single_edit_reprocesses_one_file(root: &Path) {
    std::fs::write(
        root.join("src/assets/minecraft/lang/en_us.json"),
        "{ \"pack.rpp.example.title\" : \"Edited Title\" }",
    )
    .unwrap();
    let out = build(root, &["--jobs", "2"]);
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
}

/// Renaming an item definition removes its stale assets and catalog entry.
fn assert_item_rename_cleans_stale_outputs(root: &Path) {
    std::fs::remove_file(root.join("src/items/ember_gem.ts")).unwrap();
    std::fs::write(
        root.join("src/items/frost_gem.ts"),
        "export default { name: \"Frost Gem\", texture: \"minecraft:custom/gem\" };\n",
    )
    .unwrap();
    let out = build(root, &["--jobs", "2"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!root
        .join("dist/assets/rpp/models/item/ember_gem.json")
        .exists());
    assert!(root
        .join("dist/assets/rpp/models/item/frost_gem.json")
        .is_file());
    let language = dist_json(root, "assets/rpp/lang/en_us.json");
    assert_eq!(language["item.rpp.frost_gem"], "Frost Gem");
    assert!(language.get("item.rpp.ember_gem").is_none());
    let updated = read_json(&root.join("generated/catalog/items.json"));
    assert!(updated["items"].get("ember_gem").is_none());
    assert_eq!(updated["items"]["frost_gem"]["model"], "rpp:item/frost_gem");
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

    assert_block_model_minified(&root);
    let hashed = assert_texture_hashed(&root);
    assert_catalog_generated(&root, &hashed);
    let zip_path = root.join("dist/rpp-example-pack.zip");
    assert_release_zip_contents(&zip_path);
    let first_zip = std::fs::read(&zip_path).unwrap();

    assert_unchanged_build_replays(&root, &zip_path, &first_zip);
    assert_single_edit_reprocesses_one_file(&root);
    assert_item_rename_cleans_stale_outputs(&root);
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
fn json_minify_keeps_number_text() {
    let tmp = tempfile::tempdir().unwrap();
    let root = copy_pack(tmp.path());
    std::fs::write(
        root.join("src/assets/minecraft/models/block/big.json"),
        "{ \"seed\": 9007199254740993, \"scale\": 1.50 }",
    )
    .unwrap();

    let out = build(&root, &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let model =
        std::fs::read_to_string(root.join("dist/assets/minecraft/models/block/big.json")).unwrap();
    assert_eq!(model, r#"{"seed":9007199254740993,"scale":1.50}"#);
}

#[test]
fn hash_rename_uses_processed_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    copy_dir(
        &examples_dir().join("plugins/hash-rename"),
        &root.join("plugins/hash-rename"),
    );
    write(
        root,
        "rpp.json",
        r#"{ "dependencies": { "modify": "path:plugins/modify", "hash-rename": "path:plugins/hash-rename" } }"#,
    );
    write(
        root,
        "rpp.config.ts",
        r##"import { defineConfig, plugin } from "rpp:config";
import hashRename from "plugin:hash-rename";

export default defineConfig({
  pack: { name: "processed-hash", format: 34 },
  build: { squash: { enabled: false } },
  plugins: [plugin("modify"), hashRename({ files: ["assets/*/textures/custom/**/*.png"] })],
});
"##,
    );
    write(
        root,
        "plugins/modify/rpp.json",
        r#"{ "name": "modify", "version": "1.0.0", "entry": "plugin.ts" }"#,
    );
    write(
        root,
        "plugins/modify/plugin.ts",
        r##"import { definePlugin } from "rpp";

export default definePlugin({
  processors: {
    modify: { files: "**/*.png", priority: 5, run(_ctx, file) { file.text = file.text.toUpperCase(); } },
  },
});
"##,
    );
    write(root, "src/assets/test/textures/custom/gem.png", "raw");

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
