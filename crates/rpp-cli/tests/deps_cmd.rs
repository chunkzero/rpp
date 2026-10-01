//! Tests for `rpp add` and `rpp remove` against local (`path:`) dependencies, so no
//! network is involved.

mod common;

use std::path::Path;

use serde_json::{json, Value};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    common::command(root).args(args).output().expect("run rpp")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write_tool(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("rpp.json"),
        format!(r#"{{"name": "{name}", "version": "0.1.0"}}"#),
    )
    .unwrap();
}

/// A temp dir holding an empty `project` next to a `tool` package.
fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    write_tool(&temp.path().join("tool"), "tool");
    (temp, project)
}

#[test]
fn add_path_dependency_creates_manifest() {
    let (_temp, project) = workspace();
    let out = run(&project, &["add", "path:../tool"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        read_json(&project.join("rpp.json")),
        json!({"dependencies": {"tool": "path:../tool"}})
    );
    let lock = std::fs::read_to_string(project.join("rpp.lock")).unwrap_or_default();
    assert!(!lock.contains("[[package]]"), "{lock}");
}

#[test]
fn add_preserves_other_keys_and_order() {
    let (_temp, project) = workspace();
    std::fs::write(
        project.join("rpp.json"),
        r#"{"zeta": 1, "dependencies": {"b": "path:../b"}, "alpha": [true]}"#,
    )
    .unwrap();
    write_tool(&project.parent().unwrap().join("b"), "b");
    let out = run(&project, &["add", "path:../tool"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = std::fs::read_to_string(project.join("rpp.json")).unwrap();
    assert!(text.ends_with("}\n"));
    assert!(text.contains("\n  \"zeta\": 1,"), "{text}");
    let order = [
        "\"zeta\"",
        "\"dependencies\"",
        "\"b\"",
        "\"tool\"",
        "\"alpha\"",
    ];
    let positions: Vec<_> = order.iter().map(|k| text.find(k).unwrap()).collect();
    assert!(positions.is_sorted(), "{text}");
}

#[test]
fn remove_drops_dependency() {
    let (_temp, project) = workspace();
    assert!(run(&project, &["add", "path:../tool"]).status.success());
    let out = run(&project, &["remove", "tool"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        read_json(&project.join("rpp.json")),
        json!({"dependencies": {}})
    );
    let out = run(&project, &["remove", "tool"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("not a dependency"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn refuses_legacy_project() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("rpp.toml"), "[pack]\nname = \"p\"\n").unwrap();
    let out = run(temp.path(), &["remove", "tool"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("configured by rpp.toml; use `rpp plugin remove`"),
        "{}",
        stderr(&out)
    );
    assert!(!temp.path().join("rpp.json").exists());
}

#[test]
fn failed_add_changes_nothing() {
    let (_temp, project) = workspace();
    let out = run(&project, &["add", "path:missing"]);
    assert!(!out.status.success());
    assert!(!project.join("rpp.json").exists());
    assert!(!project.join("rpp.lock").exists());
}
