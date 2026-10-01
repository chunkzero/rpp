//! Tests for `rpp.config.ts` projects.

mod common;

use std::path::Path;

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    common::command(root).args(args).output().expect("run rpp")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A project depending on the path plugin `suffix`, configured through its config module.
fn project(root: &Path, validate: &str, plugin_call: &str) {
    write(
        root,
        "rpp.json",
        r#"{ "dependencies": { "suffix": "path:plugins/suffix" } }"#,
    );
    write(
        root,
        "plugins/suffix/rpp.json",
        r#"{ "name": "suffix", "version": "0.1.0", "entry": "src/plugin.ts", "config": "src/config.ts" }"#,
    );
    write(
        root,
        "plugins/suffix/src/config.ts",
        &r##"import { definePluginConfig } from "#rpp/config";

export default definePluginConfig<{ text: string }>("suffix", {
  validate: (options) => VALIDATE,
});
"##
        .replace("VALIDATE", validate),
    );
    write(
        root,
        "plugins/suffix/src/plugin.ts",
        r##"import { definePlugin } from "#rpp";

export default definePlugin<{ text: string }>({
  processors: {
    suffix: {
      files: ["*.txt"],
      run(ctx, file) {
        file.text = file.text + ctx.options.text;
      },
    },
  },
});
"##,
    );
    write(
        root,
        "rpp.config.ts",
        &r##"import { defineConfig } from "#rpp/config";
import suffix from "#plugins/suffix";

export default defineConfig({
  pack: { name: "p" },
  plugins: [PLUGIN],
});
"##
        .replace("PLUGIN", plugin_call),
    );
    write(root, "src/a.txt", "hello");
}

#[test]
fn ts_project_builds_with_path_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(root, "undefined", r#"suffix({ text: "!" })"#);

    let out = run(root, &["build", "--no-squash"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(root.join("dist/a.txt")).unwrap(),
        "hello!"
    );
}

#[test]
fn rejects_both_config_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "rpp.toml", "[pack]\nname = \"p\"\n");
    write(
        root,
        "rpp.config.ts",
        "export default { pack: { name: \"p\" } };\n",
    );

    let out = run(root, &["build", "--no-squash"]);
    assert!(!out.status.success());
    let message = stderr(&out);
    assert!(
        message.contains("rpp.toml") && message.contains("rpp.config.ts"),
        "{message}"
    );
}

#[test]
fn unknown_plugin_package_errors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "rpp.config.ts",
        r##"import { defineConfig, plugin } from "#rpp/config";

export default defineConfig({ pack: { name: "p" }, plugins: [plugin("missing")] });
"##,
    );

    let out = run(root, &["build", "--no-squash"]);
    assert!(!out.status.success());
    let message = stderr(&out);
    assert!(message.contains("`missing`"), "{message}");
    assert!(message.contains("rpp add missing"), "{message}");
}

#[test]
fn config_errors_point_at_rpp_config_ts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(
        root,
        r#"options.text === "" ? ["text must not be empty"] : undefined"#,
        r#"suffix({ text: "" })"#,
    );

    let out = run(root, &["build", "--no-squash"]);
    assert!(!out.status.success());
    let message = stderr(&out);
    assert!(message.contains("rpp.config.ts"), "{message}");
    assert!(message.contains("text must not be empty"), "{message}");
}

#[test]
fn codegen_maps_plugin_config_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(root, "undefined", r#"suffix({ text: "!" })"#);

    let out = run(root, &["codegen"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let tsconfig = std::fs::read_to_string(root.join(".rpp/tsconfig.json")).unwrap();
    assert!(
        tsconfig.contains("\"#rpp/config\": [\"./sdk/config.ts\"]"),
        "{tsconfig}"
    );
    assert!(tsconfig.contains("\"#plugins/suffix\""), "{tsconfig}");
    assert!(
        tsconfig.contains("plugins/suffix/src/config.ts"),
        "{tsconfig}"
    );
    assert!(root.join(".rpp/sdk/config.ts").is_file());
}
