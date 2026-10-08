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
        "import {{ defineConfig, plugin }} from \"rpp:config\";\n\nexport default defineConfig({{\n{body}\n}});\n"
    )
}

/// A plugin that minifies every `data.json`.
const MINIFY_PLUGIN: &str = r##"import { definePlugin } from "rpp";

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

/// Give `path` an old modification time, so a rewrite is observable.
fn backdate(path: &Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old_time()))
        .unwrap();
}

fn old_time() -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000)
}

/// Build `root` and report whether the backdated release archive was rewritten.
fn build_rewrites_zip(root: &Path, args: &[&str]) -> bool {
    let zip = root.join("dist/test-pack.zip");
    backdate(&zip);
    let out = build(root, args);
    assert!(out.status.success(), "build failed:\n{}", stderr(&out));
    std::fs::metadata(&zip).unwrap().modified().unwrap() != old_time()
}

#[test]
fn unchanged_release_archive_is_reused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);
    assert!(build(root, &[]).status.success());
    let zip = root.join("dist/test-pack.zip");
    let archive = std::fs::read(&zip).unwrap();

    assert!(
        !build_rewrites_zip(root, &[]),
        "unchanged build kept the archive"
    );

    let loose = root.join("dist/assets/minecraft/release.json");
    let restored = std::fs::read(&loose).unwrap();
    std::fs::write(&loose, "edited by hand").unwrap();
    assert!(build_rewrites_zip(root, &[]), "restored loose output");
    assert_eq!(std::fs::read(&loose).unwrap(), restored);
    assert_eq!(std::fs::read(&zip).unwrap(), archive);

    std::fs::write(&zip, "tampered").unwrap();
    assert!(build_rewrites_zip(root, &[]), "modified archive is rebuilt");
    assert_eq!(std::fs::read(&zip).unwrap(), archive);

    assert!(build_rewrites_zip(root, &["--no-cache"]));

    let config = std::fs::read_to_string(root.join("rpp.config.ts")).unwrap();
    std::fs::write(
        root.join("rpp.config.ts"),
        config.replace("png: false", "png: \"fast\""),
    )
    .unwrap();
    assert!(build_rewrites_zip(root, &[]), "squash settings changed");
    assert!(!build_rewrites_zip(root, &[]));

    std::fs::write(
        root.join("src/assets/minecraft/release.json"),
        r#"{"edited":true}"#,
    )
    .unwrap();
    assert!(build_rewrites_zip(root, &[]), "source edited");
    assert_eq!(
        read_zip_entry(
            &std::fs::read(&zip).unwrap(),
            "assets/minecraft/release.json"
        ),
        r#"{"edited":true}"#
    );
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
