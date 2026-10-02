//! Projects still configured by `rpp.toml` are rejected with a pointer to the migration guide.

mod common;

use std::path::Path;

use common::write;

fn build_error(root: &Path) -> String {
    let out = common::command(root)
        .args(["build", "--no-squash"])
        .output()
        .expect("run rpp build");
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn rpp_toml_project_is_rejected_with_guide() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "rpp.toml", "[pack]\nname = \"p\"\n");

    let message = build_error(dir.path());
    assert!(
        message.contains("`rpp.toml` is no longer supported"),
        "{message}"
    );
    assert!(message.contains("rpp.config.ts"), "{message}");
    assert!(message.contains(rpp::MIGRATION_GUIDE), "{message}");
}

#[test]
fn both_configs_ask_to_delete_rpp_toml() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "rpp.toml", "[pack]\nname = \"p\"\n");
    write(
        dir.path(),
        "rpp.config.ts",
        "export default { pack: { name: \"p\" } };\n",
    );

    let message = build_error(dir.path());
    assert!(message.contains("delete `rpp.toml`"), "{message}");
    assert!(message.contains(rpp::MIGRATION_GUIDE), "{message}");
}

#[test]
fn init_rejects_legacy_rpp_toml() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "rpp.toml", "[pack]\nname = \"p\"\n");

    let out = common::command(dir.path())
        .args(["init", "--yes"])
        .output()
        .expect("run rpp init");
    assert!(!out.status.success());
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(
        message.contains("`rpp.toml` is no longer supported"),
        "{message}"
    );
    assert!(message.contains(rpp::MIGRATION_GUIDE), "{message}");
    assert!(!dir.path().join("rpp.config.ts").exists());
    assert!(!dir.path().join("rpp.json").exists());
}

#[test]
fn path_dependency_with_plugin_toml_is_rejected_with_guide() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("plugins/old")).unwrap();
    write(root, "plugins/old/plugin.toml", "name = \"old\"\n");
    write(
        root,
        "rpp.json",
        r#"{ "dependencies": { "old": "path:plugins/old" } }"#,
    );
    write(
        root,
        "rpp.config.ts",
        "export default { pack: { name: \"p\" } };\n",
    );

    let message = build_error(root);
    assert!(message.contains("`plugin.toml` plugins"), "{message}");
    assert!(message.contains(rpp::MIGRATION_GUIDE), "{message}");

    let out = common::command(root)
        .args(["add", "path:plugins/old"])
        .output()
        .expect("run rpp add");
    assert!(!out.status.success());
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(message.contains(rpp::MIGRATION_GUIDE), "{message}");
}
