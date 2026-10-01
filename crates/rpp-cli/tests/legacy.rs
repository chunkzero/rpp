//! Projects still configured by `rpp.toml` are rejected with a pointer to the migration guide.

mod common;

use std::path::Path;

fn write(root: &Path, rel: &str, contents: &str) {
    std::fs::write(root.join(rel), contents).unwrap();
}

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
