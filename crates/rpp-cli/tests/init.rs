//! Tests for `rpp init` scaffolding (non-interactive) and that the scaffolded
//! project builds.

mod common;

use common::run;

#[test]
fn init_scaffolds_a_buildable_ts_project() {
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

    for file in [
        "rpp.config.ts",
        "rpp.json",
        "plugins/hello/rpp.json",
        "plugins/hello/src/plugin.ts",
        ".gitignore",
    ] {
        assert!(root.join(file).is_file(), "{file} missing");
    }
    let config = std::fs::read_to_string(root.join("rpp.config.ts")).unwrap();
    assert!(config.contains("name: \"scaffolded\""));
    assert!(config.contains("format: 34"));

    // The scaffolded project builds successfully.
    let build = run(root, &["build"]);
    assert!(
        build.status.success(),
        "build of scaffolded project failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    // The starter generator emits a marker file.
    assert!(root.join("dist/rpp_build.txt").is_file());
    let mcmeta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("dist/pack.mcmeta")).unwrap()).unwrap();
    assert_eq!(
        mcmeta,
        serde_json::json!({ "pack": { "description": "a test", "pack_format": 34 } })
    );
}

#[test]
fn init_uses_defaults_without_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let out = common::command(root)
        .arg("init")
        .env("TERM", "xterm-256color")
        .output()
        .expect("run init");
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{report}");
    assert!(out.stdout.is_empty(), "init status belongs on stderr");
    assert!(report.contains("Initialized rpp project"), "{report}");
    assert!(!report.contains('\x1b'), "redirected output must be plain");

    let project = rpp_cli::project::Project::discover(root).unwrap();
    assert_eq!(
        project.config.pack.name,
        root.file_name().unwrap().to_string_lossy()
    );
    assert_eq!(project.config.pack.description, "A Minecraft resource pack");
    assert_eq!(project.config.pack.format.max, 34);
}

#[test]
fn init_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("rpp.config.ts"), "export default {};\n").unwrap();

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
    assert!(!root.join("rpp.config.ts").exists());
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
    let project = rpp_cli::project::Project::discover(&root).unwrap();
    assert_eq!(project.config.pack.name, "quoted \" pack");
    assert_eq!(project.config.pack.description, "line \" one");
}
