//! Tests for `rpp codegen`, `rpp check`, and TypeScript plugin builds.

mod common;

use std::path::Path;

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    common::command(root).args(args).output().expect("run rpp")
}

fn project(root: &Path) {
    std::fs::write(root.join("rpp.toml"), "[pack]\nname = \"p\"\n").unwrap();
}

#[test]
fn codegen_writes_sdk_and_tsconfig() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(root);

    let out = run(root, &["codegen"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    for (path, contents) in rpp::js::SDK_FILES {
        let written = std::fs::read_to_string(root.join(".rpp/sdk").join(path)).unwrap();
        assert_eq!(&written, contents);
    }
    assert!(root.join(".rpp/sdk/globals.d.ts").is_file());
    let tsconfig = std::fs::read_to_string(root.join(".rpp/tsconfig.json")).unwrap();
    assert!(tsconfig.contains("\"#rpp\": [\"./sdk/index.ts\"]"));
    let root_config = std::fs::read_to_string(root.join("tsconfig.json")).unwrap();
    assert!(root_config.contains("./.rpp/tsconfig.json"));
}

#[test]
fn codegen_keeps_existing_tsconfig() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(root);
    std::fs::write(root.join("tsconfig.json"), "{ \"custom\": true }").unwrap();

    assert!(run(root, &["codegen"]).status.success());
    assert_eq!(
        std::fs::read_to_string(root.join("tsconfig.json")).unwrap(),
        "{ \"custom\": true }"
    );
}

#[test]
fn codegen_works_in_plugin_dir() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = dir.path().join("plugin");
    std::fs::create_dir_all(plugin.join("src")).unwrap();
    std::fs::write(
        plugin.join("plugin.toml"),
        "[plugin]\nid = \"p\"\nversion = \"0.1.0\"\nentry = \"src/plugin.ts\"\n",
    )
    .unwrap();

    let out = run(&plugin.join("src"), &["codegen"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(plugin.join(".rpp/sdk/index.ts").is_file());
    assert!(plugin.join("tsconfig.json").is_file());
}

#[test]
fn check_reports_missing_compiler() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(root);

    let out = common::command(root)
        .env("RPP_TSC", root.join("no-such-tsc"))
        .arg("check")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("TypeScript 7+"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn check_runs_configured_compiler() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(root);
    let script = root.join("fake-tsc");
    let record = root.join("args.txt");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$PWD $@\" > '{}'\nexit \"${{FAKE_TSC_EXIT:-0}}\"\n",
            record.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let check = |exit: &str| {
        common::command(root)
            .env("RPP_TSC", &script)
            .env("FAKE_TSC_EXIT", exit)
            .arg("check")
            .output()
            .unwrap()
    };
    assert!(check("0").status.success());
    let args = std::fs::read_to_string(&record).unwrap();
    assert!(args.contains("--noEmit --pretty"), "{args}");
    assert!(args.contains(&format!("-p {}", root.join("tsconfig.json").display())));

    let failed = check("1");
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("type checking failed"));
}

#[test]
fn build_runs_typescript_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("rpp.toml"),
        "[pack]\nname = \"p\"\n\n[[plugin]]\nsource = \"path:plugins/upper\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), "hello").unwrap();
    let plugin = root.join("plugins/upper");
    std::fs::create_dir_all(plugin.join("src")).unwrap();
    std::fs::write(
        plugin.join("plugin.toml"),
        "[plugin]\nid = \"upper\"\nversion = \"0.1.0\"\nentry = \"src/plugin.ts\"\n",
    )
    .unwrap();
    std::fs::write(
        plugin.join("src/plugin.ts"),
        r##"import { definePlugin } from "#rpp";

export default definePlugin({
  processors: {
    upper: {
      files: ["*.txt"],
      run(_ctx, file) {
        file.text = file.text.toUpperCase();
      },
    },
  },
});
"##,
    )
    .unwrap();

    let out = run(root, &["build", "--no-squash"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join("dist/a.txt")).unwrap(),
        "HELLO"
    );
}
