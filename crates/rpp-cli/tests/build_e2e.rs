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
        "[plugin]\nid = \"minify\"\nversion = \"0.1.0\"\nruntime = \"lua\"\n",
    )
    .unwrap();
    std::fs::write(
        plugin.join("init.lua"),
        r#"local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("minify", { files = { "pack.mcmeta", "**/data.json" } }, function(ctx, file)
    local data = rpp.json.decode(file.text)
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
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes)).unwrap();
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

    // Output and zip still present after the cached build.
    assert!(produced.is_file());
    assert!(zip.is_file());
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
