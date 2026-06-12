//! Tests for `rpp plugin` add/remove/list against local (`path:`) sources, so
//! no network is involved. Lockfile interaction is exercised implicitly: path
//! sources are never pinned, so `rpp.lock` stays absent/empty.

use std::path::Path;
use std::process::Command;

fn rpp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rpp")
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(rpp_bin())
        .current_dir(root)
        .args(args)
        .output()
        .expect("run rpp")
}

/// Scaffold a project with one local plugin, plus a second local plugin dir
/// (not yet referenced) that `plugin add` can resolve.
fn scaffold(root: &Path) {
    std::fs::write(
        root.join("rpp.toml"),
        "# header comment\n[pack]\nname = \"p\"  # inline\n\n[[plugin]]\nsource = \"path:plugins/a\"\n",
    )
    .unwrap();

    for (name, id) in [("a", "alpha"), ("b", "beta")] {
        let dir = root.join("plugins").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            format!("[plugin]\nid = \"{id}\"\nversion = \"0.1.0\"\nruntime = \"lua\"\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("init.lua"),
            "local rpp = require(\"rpp\")\nreturn rpp.plugin()\n",
        )
        .unwrap();
    }
}

#[test]
fn add_then_remove_preserves_formatting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    // Add the second plugin.
    let out = run(root, &["plugin", "add", "path:plugins/b"]);
    assert!(
        out.status.success(),
        "add failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let toml = std::fs::read_to_string(root.join("rpp.toml")).unwrap();
    assert!(toml.contains("# header comment"), "comments preserved");
    assert!(toml.contains("# inline"), "inline comment preserved");
    assert!(toml.contains("source = \"path:plugins/b\""));
    // Output reports the id/version.
    assert!(String::from_utf8_lossy(&out.stdout).contains("beta"));

    // No lockfile for path sources.
    assert!(
        !root.join("rpp.lock").exists(),
        "path sources are not locked"
    );

    // List shows both.
    let list = run(root, &["plugin", "list"]);
    let list_out = String::from_utf8_lossy(&list.stdout);
    assert!(list_out.contains("alpha"), "{list_out}");
    assert!(list_out.contains("beta"), "{list_out}");

    // Remove by id.
    let rm = run(root, &["plugin", "remove", "beta"]);
    assert!(
        rm.status.success(),
        "remove failed:\n{}",
        String::from_utf8_lossy(&rm.stderr)
    );
    let toml = std::fs::read_to_string(root.join("rpp.toml")).unwrap();
    assert!(!toml.contains("path:plugins/b"));
    assert!(toml.contains("path:plugins/a"), "other plugin retained");
}

#[test]
fn add_rejects_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    let out = run(root, &["plugin", "add", "path:plugins/a"]);
    assert!(!out.status.success(), "duplicate add should fail");
    assert!(String::from_utf8_lossy(&out.stderr).contains("already configured"));
}
