//! End-to-end test of `rpp build` against a small project fixture built in a
//! tempdir. Exercises the real binary (via `CARGO_BIN_EXE_rpp`): a JSON file is
//! minified by a local TypeScript plugin, a zip is produced, and a second build is
//! fully cached.

mod common;

use std::path::Path;

use common::{build, stderr, write};

/// An `rpp.config.ts` whose `defineConfig` object body is `body`.
fn config_ts(body: &str) -> String {
    format!(
        "import {{ defineConfig, plugin }} from \"#rpp/config\";\n\nexport default defineConfig({{\n{body}\n}});\n"
    )
}

/// A plugin that minifies every `data.json`.
const MINIFY_PLUGIN: &str = r##"import { definePlugin } from "#rpp";

export default definePlugin({
  processors: {
    minify: {
      files: ["**/data.json"],
      run(_ctx, file) {
        file.text = JSON.stringify(JSON.parse(file.text));
      },
    },
  },
});
"##;

/// Write a minimal but complete project into `root`.
fn scaffold(root: &Path) {
    write(
        root,
        "rpp.config.ts",
        config_ts(
            r#"  pack: { name: "test-pack", description: "fixture", format: 34 },
  build: {
    source: "src",
    output: "dist",
    squash: { enabled: true, engine: "builtin", json: true, png: false, zip: true },
  },
  plugins: [plugin("minify")],"#,
        ),
    );
    write(
        root,
        "rpp.json",
        r#"{ "dependencies": { "minify": "path:plugins/minify" } }"#,
    );

    let src = root.join("src");
    std::fs::create_dir_all(src.join("assets/minecraft")).unwrap();
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

    write(
        root,
        "plugins/minify/rpp.json",
        r#"{ "name": "minify", "version": "0.1.0", "entry": "src/plugin.ts" }"#,
    );
    write(root, "plugins/minify/src/plugin.ts", MINIFY_PLUGIN);
}

fn read_zip_entry(zip_bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
    let mut contents = String::new();
    std::io::Read::read_to_string(&mut archive.by_name(name).unwrap(), &mut contents).unwrap();
    contents
}

/// A second build with nothing changed is fully cached and keeps the release archive.
fn assert_second_build_fully_cached(root: &Path) {
    let out = build(root, &[]);
    assert!(
        out.status.success(),
        "second build failed:\n{}",
        stderr(&out)
    );
    let report = stderr(&out);
    assert!(
        report.contains("processed 0"),
        "second build should be fully cached: {report}"
    );
    assert!(
        report.contains("0 removed"),
        "release archive should be preserved by output sync: {report}"
    );
}

#[test]
fn build_minifies_and_zips_then_caches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    // First build.
    let out = build(root, &[]);
    assert!(
        out.status.success(),
        "first build failed:\n{}",
        stderr(&out)
    );
    assert!(out.stdout.is_empty(), "build status belongs on stderr");
    let report1 = stderr(&out);
    assert!(!report1.contains('\x1b'), "redirected output must be plain");

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
    assert_eq!(
        read_zip_entry(&archive_bytes, "assets/minecraft/release.json"),
        r#"{"release_only":true}"#
    );

    // First build processed at least one file.
    assert!(
        report1.contains("processed"),
        "expected build stats: {report1}"
    );

    assert_second_build_fully_cached(root);

    assert_eq!(std::fs::read_to_string(&produced).unwrap(), body);
    assert_eq!(std::fs::read_to_string(&release_json).unwrap(), loose);
    assert_eq!(std::fs::read(&zip).unwrap(), archive_bytes);
}

#[test]
fn no_squash_removes_stale_release_archive() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    assert!(build(root, &[]).status.success());
    let zip = root.join("dist/test-pack.zip");
    assert!(zip.is_file());

    std::fs::write(
        root.join("src/assets/minecraft/data.json"),
        r#"{"updated":true}"#,
    )
    .unwrap();
    let output = build(root, &["--no-squash"]);
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

    assert!(build(root, &[]).status.success());
    assert!(root.join("dist").is_dir());
    assert!(root.join(".rpp").is_dir());

    let out = common::command(root)
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
    write(
        root,
        "rpp.config.ts",
        config_ts(
            r#"  pack: { name: "test", format: 34 },
  plugins: [plugin("missing")],"#,
        ),
    );
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(root.join("dist/file.txt"), "x").unwrap();

    let out = common::command(root)
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
    write(
        &root,
        "rpp.config.ts",
        config_ts(
            r#"  pack: { name: "test", format: 34 },
  build: { output: "../victim" },"#,
        ),
    );

    let out = common::command(&root)
        .arg("clean")
        .output()
        .expect("run rpp clean");
    assert!(!out.status.success());
    assert!(victim.join("keep.txt").exists());
}
