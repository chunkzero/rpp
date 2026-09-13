//! Tests for `rpp init` scaffolding (non-interactive) and that the scaffolded
//! project builds.

mod common;

use std::path::Path;

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    common::command(root).args(args).output().expect("run rpp")
}

#[test]
fn init_scaffolds_a_buildable_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let out = run(
        root,
        &[
            "init",
            "--yes",
            "--name",
            "scaffolded",
            "--description",
            "a test",
            "--pack-format",
            "34",
        ],
    );
    assert!(
        out.status.success(),
        "init failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Scaffold contents.
    assert!(root.join("rpp.toml").is_file());
    assert!(root.join("src/pack.mcmeta").is_file());
    assert!(root.join("plugins/hello/plugin.toml").is_file());
    assert!(root.join("plugins/hello/init.lua").is_file());
    assert!(root.join(".gitignore").is_file());
    // LuaLS definitions written.
    assert!(root.join(".rpp/api/rpp.lua").is_file());
    assert!(root.join(".rpp/api/file.lua").is_file());

    let toml = std::fs::read_to_string(root.join("rpp.toml")).unwrap();
    assert!(toml.contains("name = \"scaffolded\""));
    assert!(toml.contains("pack_format = 34"));

    // The scaffolded project builds successfully.
    let build = run(root, &["build"]);
    assert!(
        build.status.success(),
        "build of scaffolded project failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    // The starter generator emits a marker file.
    assert!(root.join("dist/rpp_build.txt").is_file());
}

#[test]
fn init_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("rpp.toml"), "[pack]\nname=\"x\"\n").unwrap();

    let out = run(root, &["init", "--yes"]);
    assert!(!out.status.success(), "should refuse to overwrite");
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
}

#[test]
fn init_refuses_to_overwrite_any_scaffold_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(".gitignore"), "keep\n").unwrap();

    let out = common::command(root)
        .args(["init", "--yes"])
        .output()
        .expect("run init");
    assert!(!out.status.success());
    assert_eq!(
        std::fs::read_to_string(root.join(".gitignore")).unwrap(),
        "keep\n"
    );
    assert!(!root.join("rpp.toml").exists());
}

#[test]
fn init_escapes_user_strings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("quoted");

    let out = common::command(dir.path())
        .args([
            "init",
            root.to_str().unwrap(),
            "--yes",
            "--name",
            "quoted \" pack",
            "--description",
            "line \" one",
        ])
        .output()
        .expect("run init");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    rpp::config::Config::load(root.join("rpp.toml")).unwrap();
    let mcmeta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("src/pack.mcmeta")).unwrap()).unwrap();
    assert_eq!(mcmeta["pack"]["description"], "line \" one");
}
