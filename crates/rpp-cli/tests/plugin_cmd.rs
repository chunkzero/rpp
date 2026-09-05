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
        .env("RPP_HOME", root.join(".test-rpp-home"))
        .args(args)
        .output()
        .expect("run rpp")
}

fn run_with_home(root: &Path, home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(rpp_bin())
        .current_dir(root)
        .env("RPP_HOME", home)
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
            format!("[plugin]\nid = \"{id}\"\nversion = \"0.1.0\"\n"),
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

#[test]
fn add_accepts_bare_directory_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root);

    let out = run(root, &["plugin", "add", "plugins/b", "--project"]);
    assert!(
        out.status.success(),
        "add failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let toml = std::fs::read_to_string(root.join("rpp.toml")).unwrap();
    assert!(toml.contains("source = \"path:"));
    assert!(toml.contains("/plugins/b\""));
}

#[test]
fn global_directory_plugin_is_copied_and_project_plugin_overrides_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("home");
    let global_source = root.join("window");
    write_text_plugin(&global_source, "shared", "global");

    let install = run_with_home(root, &home, &["plugin", "add", "window", "--global"]);
    assert!(
        install.status.success(),
        "global add failed:\n{}",
        String::from_utf8_lossy(&install.stderr)
    );
    let manifest = std::fs::read_to_string(home.join("plugins.toml")).unwrap();
    assert!(manifest.contains("source = \"path:plugins/shared\""));
    assert!(home.join("plugins/shared/init.lua").is_file());

    let project = root.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/value.txt"), "start").unwrap();
    std::fs::write(
        project.join("rpp.toml"),
        "[pack]\nname = \"test\"\n\n[build]\nsource = \"src\"\noutput = \"dist\"\n",
    )
    .unwrap();

    let build = run_with_home(&project, &home, &["build", "--no-squash"]);
    assert!(
        build.status.success(),
        "global build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(project.join("dist/value.txt")).unwrap(),
        "global"
    );

    let local_source = project.join("plugins/local");
    write_text_plugin(&local_source, "shared", "local");
    std::fs::write(
        project.join("rpp.toml"),
        "[pack]\nname = \"test\"\n\n[build]\nsource = \"src\"\noutput = \"dist\"\n\n\
         [[plugin]]\nsource = \"path:plugins/local\"\n",
    )
    .unwrap();

    let build = run_with_home(&project, &home, &["build", "--no-squash"]);
    assert!(
        build.status.success(),
        "override build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(project.join("dist/value.txt")).unwrap(),
        "local"
    );

    let remove = run_with_home(&project, &home, &["plugin", "remove", "shared", "--global"]);
    assert!(
        remove.status.success(),
        "global remove failed:\n{}",
        String::from_utf8_lossy(&remove.stderr)
    );
    assert!(!home.join("plugins/shared").exists());
}

#[test]
fn project_plugin_can_reference_global_plugin_by_id() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("home");
    let global_source = root.join("global-shared");
    write_option_text_plugin(&global_source, "shared");

    let install = run_with_home(root, &home, &["plugin", "add", "global-shared", "--global"]);
    assert!(
        install.status.success(),
        "global add failed:\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    let project = root.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/value.txt"), "start").unwrap();
    std::fs::write(
        project.join("rpp.toml"),
        "[pack]\nname = \"test\"\n\n[build]\nsource = \"src\"\noutput = \"dist\"\n\n\
         [[plugin]]\nid = \"shared\"\n[plugin.options]\nvalue = \"project\"\n",
    )
    .unwrap();

    let build = run_with_home(&project, &home, &["build", "--no-squash"]);
    assert!(
        build.status.success(),
        "id reference build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(project.join("dist/value.txt")).unwrap(),
        "project"
    );
    assert_eq!(
        std::fs::read_to_string(project.join(".rpp/api/shared.lua")).unwrap(),
        "---@meta shared\n"
    );
}

fn write_text_plugin(root: &Path, id: &str, value: &str) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join("plugin.toml"),
        format!("[plugin]\nid = \"{id}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("init.lua"),
        format!(
            "local rpp = require(\"rpp\")\n\
             local plugin = rpp.plugin()\n\
             plugin:processor(\"replace\", {{ files = {{ \"**/*.txt\" }} }}, function(ctx, file)\n\
                 file.text = \"{value}\"\n\
             end)\n\
             return plugin\n"
        ),
    )
    .unwrap();
}

fn write_option_text_plugin(root: &Path, id: &str) {
    std::fs::create_dir_all(root.join("luals")).unwrap();
    std::fs::write(
        root.join("plugin.toml"),
        format!("[plugin]\nid = \"{id}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("init.lua"),
        "local rpp = require(\"rpp\")\n\
         local plugin = rpp.plugin()\n\
         plugin:processor(\"replace\", { files = { \"**/*.txt\" } }, function(ctx, file)\n\
             file.text = ctx.options.value or \"global\"\n\
         end)\n\
         return plugin\n",
    )
    .unwrap();
    std::fs::write(root.join("luals/shared.lua"), "---@meta shared\n").unwrap();
}

#[test]
fn remove_id_only_override() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("rpp.toml");
    std::fs::write(
        &config,
        r#"[pack]
name = "test"
[[plugin]]
id = "window"
[plugin.options]
enabled = false
"#,
    )
    .unwrap();
    let result = run(dir.path(), &["plugin", "remove", "window"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!std::fs::read_to_string(&config)
        .unwrap()
        .contains("[[plugin]]"));
    let result = run(dir.path(), &["plugin", "remove", "window"]);
    assert!(!result.status.success());
}

#[test]
fn cached_subdir_add_and_remove_keep_config_and_lock_aligned() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let cache = root.join("cache");
    std::fs::write(root.join("rpp.toml"), "[pack]\nname = \"p\"\n").unwrap();
    let mut lock = rpp_fetch::Lockfile::new();
    for (subdir, id) in [("a", "alpha"), ("b", "beta")] {
        write_text_plugin(
            &cache.join(format!("github/owner/repo/commit/{subdir}")),
            id,
            id,
        );
        lock.upsert(rpp_fetch::LockedPlugin {
            source: "github:owner/repo".into(),
            ref_: "main".into(),
            requested_ref: None,
            commit: "commit".into(),
            subdir: Some(subdir.into()),
        });
    }
    lock.save(&root.join("rpp.lock")).unwrap();
    let run_cached = |args: &[&str]| {
        let result = Command::new(rpp_bin())
            .current_dir(root)
            .env("RPP_HOME", root.join("home"))
            .env("RPP_CACHE_DIR", &cache)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    for subdir in [" ./a// ", " ./b// "] {
        run_cached(&[
            "plugin",
            "add",
            "github:owner/repo.git",
            "--subdir",
            subdir,
            "--project",
        ]);
    }
    run_cached(&["plugin", "remove", "beta"]);
    let config = rpp::config::Config::load(root.join("rpp.toml")).unwrap();
    assert_eq!(config.plugins.len(), 1);
    assert_eq!(config.plugins[0].subdir.as_deref(), Some("a"));
    let lock = rpp_fetch::Lockfile::load(&root.join("rpp.lock")).unwrap();
    assert_eq!(lock.plugins().len(), 1);
    assert_eq!(lock.plugins()[0].subdir.as_deref(), Some("a"));
    assert!(lock.get_for("github:owner/repo", None, Some("a")).is_some());
}
