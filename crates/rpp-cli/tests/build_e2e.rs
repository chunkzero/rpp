//! End-to-end test of `rpp build` against a small project fixture built in a
//! tempdir. Exercises the real binary (via `CARGO_BIN_EXE_rpp`): a JSON file is
//! minified by a local Lua plugin, a zip is produced, and a second build is
//! fully cached.

use std::path::Path;
use std::process::Command;

fn rpp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rpp")
}

/// Write a minimal but complete project into `root`.
fn scaffold(root: &Path) {
    std::fs::write(
        root.join("rpp.toml"),
        r#"[pack]
name = "test-pack"
description = "fixture"
pack_format = 34

[build]
source = "src"
output = "dist"

[build.squash]
enabled = true
engine = "builtin"
json = true
png = false
zip = true

[[plugin]]
source = "path:plugins/minify"
"#,
    )
    .unwrap();

    let src = root.join("src");
    std::fs::create_dir_all(src.join("assets/minecraft")).unwrap();
    std::fs::write(
        src.join("pack.mcmeta"),
        "{\n  \"pack\": {\n    \"pack_format\": 34,\n    \"description\": \"fixture\"\n  }\n}\n",
    )
    .unwrap();
    // A pretty-printed JSON file the plugin will minify.
    std::fs::write(
        src.join("assets/minecraft/data.json"),
        "{\n    \"a\": 1,\n    \"b\": [\n        2,\n        3\n    ]\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("assets/minecraft/release.json"),
        "{\n  \"release_only\": true\n}\n",
    )
    .unwrap();

    let plugin = root.join("plugins/minify");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("plugin.toml"),
        "[plugin]\nid = \"minify\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        plugin.join("init.lua"),
        r#"local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("minify", { files = { "pack.mcmeta", "**/data.json" } }, function(ctx, file)
    local data = rpp.json.decode(file.text)
    if ctx.options.output ~= nil then
        data.marker = ctx.options.output.marker
    end
    file.text = rpp.json.encode(data)
end)
return plugin
"#,
    )
    .unwrap();
}

fn run_build(root: &Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["build"];
    args.extend_from_slice(extra);
    Command::new(rpp_bin())
        .current_dir(root)
        .args(&args)
        .output()
        .expect("run rpp build")
}

#[test]
fn build_minifies_and_zips_then_caches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    // First build.
    let out = run_build(root, &[]);
    assert!(
        out.status.success(),
        "first build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout1 = String::from_utf8_lossy(&out.stdout);

    // Output JSON is minified (no newlines / indentation from the source).
    let produced = root.join("dist/assets/minecraft/data.json");
    let body = std::fs::read_to_string(&produced).expect("output json present");
    assert_eq!(body, r#"{"a":1,"b":[2,3]}"#, "json minified");

    // Builtin squash applies only to the release archive, not loose output.
    let release_json = root.join("dist/assets/minecraft/release.json");
    let loose = std::fs::read_to_string(&release_json).expect("release json present");
    assert!(loose.contains('\n'), "loose output remains unsquashed");

    // Zip produced.
    let zip = root.join("dist/test-pack.zip");
    assert!(zip.is_file(), "zip produced at {}", zip.display());
    let archive_bytes = std::fs::read(&zip).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&archive_bytes)).unwrap();
    let mut archived_release = String::new();
    std::io::Read::read_to_string(
        &mut archive.by_name("assets/minecraft/release.json").unwrap(),
        &mut archived_release,
    )
    .unwrap();
    assert_eq!(archived_release, r#"{"release_only":true}"#);

    // First build processed at least one file.
    assert!(
        stdout1.contains("processed"),
        "expected build stats: {stdout1}"
    );

    // Second build: nothing changed -> fully cached (processed 0).
    let out2 = run_build(root, &[]);
    assert!(
        out2.status.success(),
        "second build failed:\n{}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout2.contains("processed 0"),
        "second build should be fully cached: {stdout2}"
    );
    assert!(
        stdout2.contains("0 removed"),
        "release archive should be preserved by output sync: {stdout2}"
    );

    assert_eq!(std::fs::read_to_string(&produced).unwrap(), body);
    assert_eq!(std::fs::read_to_string(&release_json).unwrap(), loose);
    assert_eq!(std::fs::read(&zip).unwrap(), archive_bytes);
}

#[test]
fn no_squash_removes_stale_release_archive() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    assert!(run_build(root, &[]).status.success());
    let zip = root.join("dist/test-pack.zip");
    assert!(zip.is_file());

    std::fs::write(
        root.join("src/pack.mcmeta"),
        r#"{"pack":{"pack_format":34,"description":"updated"}}"#,
    )
    .unwrap();
    let output = run_build(root, &["--no-squash"]);
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!zip.exists(), "--no-squash must not leave a stale zip");
}

#[test]
fn clean_removes_output_and_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    assert!(run_build(root, &[]).status.success());
    assert!(root.join("dist").is_dir());
    assert!(root.join(".rpp").is_dir());

    let out = Command::new(rpp_bin())
        .current_dir(root)
        .arg("clean")
        .output()
        .expect("run rpp clean");
    assert!(out.status.success());
    assert!(!root.join("dist").exists(), "output removed");
    assert!(!root.join(".rpp").exists(), "cache removed");
}

#[test]
fn clean_does_not_load_plugins() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("rpp.toml"),
        "[pack]\nname = \"test\"\n[[plugin]]\nsource = \"path:missing\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(root.join("dist/file.txt"), "x").unwrap();

    let out = Command::new(rpp_bin())
        .current_dir(root)
        .arg("clean")
        .output()
        .expect("run rpp clean");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!root.join("dist").exists());
}

#[test]
fn clean_rejects_output_outside_project() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("project");
    let victim = parent.path().join("victim");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep.txt"), "keep").unwrap();
    std::fs::write(
        root.join("rpp.toml"),
        "[pack]\nname = \"test\"\n[build]\noutput = \"../victim\"\n",
    )
    .unwrap();

    let out = Command::new(rpp_bin())
        .current_dir(&root)
        .arg("clean")
        .output()
        .expect("run rpp clean");
    assert!(!out.status.success());
    assert!(victim.join("keep.txt").exists());
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == ".rpp" || name == "dist" {
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

/// `examples/pack` builds through the CLI from its own `rpp.toml`, with the
/// shared Lua plugins, the pack-local catalog plugin, and builtin squash.
#[test]
fn example_pack_builds_from_its_own_config() {
    let dir = tempfile::tempdir().unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    copy_dir(&examples.join("pack"), &dir.path().join("pack"));
    for plugin in ["json-minify", "mcmeta-validate", "hash-rename"] {
        copy_dir(
            &examples.join("plugins").join(plugin),
            &dir.path().join("plugins").join(plugin),
        );
    }
    let root = dir.path().join("pack");

    let out = run_build(&root, &["--jobs", "2"]);
    assert!(
        out.status.success(),
        "first build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Loose output: minified JSON, fingerprinted custom texture, rename map.
    let model =
        std::fs::read_to_string(root.join("dist/assets/minecraft/models/block/rpp_bricks.json"))
            .unwrap();
    assert!(
        !model.contains('\n'),
        "model JSON should be minified: {model}"
    );
    assert!(!root
        .join("dist/assets/minecraft/textures/custom/gem.png")
        .exists());
    let map: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("dist/rename_map.json")).unwrap())
            .unwrap();
    let hashed = map["assets/minecraft/textures/custom/gem.png"]
        .as_str()
        .unwrap();
    assert!(root.join("dist").join(hashed).is_file(), "{hashed} missing");
    assert!(
        !root.join("dist/notes/design.txt").exists(),
        ".rppignore is honored"
    );

    let read_json = |path: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap()
    };
    let reference = format!(
        "minecraft:{}",
        hashed
            .strip_prefix("assets/minecraft/textures/")
            .unwrap()
            .strip_suffix(".png")
            .unwrap()
    );
    assert_eq!(
        read_json("dist/assets/minecraft/models/item/magic_gem.json")["textures"]["layer0"],
        reference
    );
    assert_eq!(
        read_json("dist/assets/rpp/models/item/ember_gem.json")["textures"]["layer0"],
        reference
    );
    assert_eq!(
        read_json("dist/assets/rpp/lang/en_us.json")["item.rpp.ember_gem"],
        "Ember Gem"
    );
    let catalog = read_json("generated/catalog/items.json");
    assert_eq!(catalog["items"]["ember_gem"]["model"], "rpp:item/ember_gem");
    assert_eq!(catalog["items"]["ember_gem"]["texture"], reference);
    assert!(!root.join("dist/items/ember_gem.lua").exists());

    // Release zip: deterministic, pack.mcmeta first, no stray archive inside.
    let zip_path = root.join("dist/rpp-example-pack.zip");
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    assert_eq!(archive.by_index(0).unwrap().name(), "pack.mcmeta");
    assert!(archive.by_name("rpp-example-pack.zip").is_err());
    assert!(archive.by_name("items/ember_gem.lua").is_err());
    assert!(archive.by_name("generated/catalog/items.json").is_err());
    assert!(archive
        .by_name("assets/rpp/models/item/ember_gem.json")
        .is_ok());
    let first_bytes = std::fs::read(&zip_path).unwrap();

    let out = run_build(&root, &["--jobs", "2"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("processed 0"),
        "second build must be cached: {stdout}"
    );
    assert_eq!(
        std::fs::read(&zip_path).unwrap(),
        first_bytes,
        "zip must be reproducible"
    );
    assert_eq!(read_json("generated/catalog/items.json"), catalog);

    // A source rename invalidates the list dependency and removes stale assets.
    std::fs::remove_file(root.join("src/items/ember_gem.lua")).unwrap();
    std::fs::write(
        root.join("src/items/frost_gem.lua"),
        r#"return { name = "Frost Gem", texture = "minecraft:custom/gem" }"#,
    )
    .unwrap();
    let out = run_build(&root, &["--jobs", "2"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!root
        .join("dist/assets/rpp/models/item/ember_gem.json")
        .exists());
    assert!(root
        .join("dist/assets/rpp/models/item/frost_gem.json")
        .is_file());
    let language = read_json("dist/assets/rpp/lang/en_us.json");
    assert_eq!(language["item.rpp.frost_gem"], "Frost Gem");
    assert!(language.get("item.rpp.ember_gem").is_none());
    let updated = read_json("generated/catalog/items.json");
    assert!(updated["items"].get("ember_gem").is_none());
    assert_eq!(updated["items"]["frost_gem"]["model"], "rpp:item/frost_gem");
}

#[test]
fn hash_rename_uses_processed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
    copy_dir(
        &examples.join("hash-rename"),
        &root.join("plugins/hash-rename"),
    );
    std::fs::create_dir_all(root.join("plugins/modify")).unwrap();
    std::fs::write(
        root.join("plugins/modify/plugin.toml"),
        "[plugin]\nid = \"modify\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("plugins/modify/init.lua"),
        r#"local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("modify", { files = { "**/*.png" }, priority = 5 }, function(ctx, file)
    file.bytes = string.upper(file.bytes)
end)
return plugin
"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src/assets/test/textures/custom")).unwrap();
    std::fs::write(root.join("src/assets/test/textures/custom/gem.png"), "raw").unwrap();
    std::fs::write(
        root.join("rpp.toml"),
        r#"[pack]
name = "processed-hash"

[build.squash]
enabled = false

[[plugin]]
source = "path:plugins/modify"

[[plugin]]
source = "path:plugins/hash-rename"
[plugin.options]
files = ["assets/*/textures/custom/**/*.png"]
"#,
    )
    .unwrap();

    let output = run_build(root, &[]);
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("dist/rename_map.json")).unwrap())
            .unwrap();
    let hashed = map["assets/test/textures/custom/gem.png"].as_str().unwrap();
    assert_eq!(
        std::fs::read(root.join("dist").join(hashed)).unwrap(),
        b"RAW"
    );
    assert!(!root
        .join("dist/assets/test/textures/custom/gem.png")
        .exists());
}
