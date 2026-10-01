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
    let globals = std::fs::read_to_string(root.join(".rpp/sdk/globals.d.ts")).unwrap();
    assert!(globals.contains("declare var crypto"));
    assert!(!globals.contains("structuredClone"));
    let tsconfig = std::fs::read_to_string(root.join(".rpp/tsconfig.json")).unwrap();
    assert!(tsconfig.contains("\"#rpp\": [\"./sdk/index.ts\"]"));
    let root_config = std::fs::read_to_string(root.join("tsconfig.json")).unwrap();
    assert!(root_config.contains("./.rpp/tsconfig.json"));
    assert!(root_config.contains("**/*.mts"));
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
fn codegen_works_in_json_plugin_dir() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = dir.path().join("plugin");
    std::fs::create_dir_all(plugin.join("src")).unwrap();
    std::fs::write(plugin.join("rpp.json"), r#"{"name":"p","version":"0.1.0"}"#).unwrap();
    std::fs::write(plugin.join("src/plugin.ts"), "").unwrap();

    let out = run(&plugin.join("src"), &["codegen"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tsconfig = std::fs::read_to_string(plugin.join(".rpp/tsconfig.json")).unwrap();
    assert!(
        tsconfig.contains("\"#rpp/config\": [\"./sdk/config.ts\"]"),
        "{tsconfig}"
    );
}

#[test]
fn codegen_ignores_project_rpp_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("rpp.config.ts"), "").unwrap();
    std::fs::write(root.join("rpp.json"), r#"{"dependencies":{}}"#).unwrap();

    let out = run(root, &["codegen"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!root.join(".rpp/generated").exists());
}

fn component_wasm() -> Vec<u8> {
    use wit_component::{ComponentEncoder, StringEncoding};
    use wit_parser::{ManglingAndAbi, Resolve};

    let mut resolve = Resolve::default();
    let package = resolve
        .push_str(
            "c.wit",
            "package t:c;\nworld w { export add: func(a: u32, b: u32) -> u32; }",
        )
        .unwrap();
    let world = resolve.select_world(&[package], None).unwrap();
    let mut module = wit_component::dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    wit_component::embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
        .unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .encode()
        .unwrap()
}

#[test]
fn codegen_writes_generated_component_dts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("plugin.toml"),
        "[plugin]\nid = \"p\"\nversion = \"0.1.0\"\nentry = \"src/plugin.ts\"\n\n[component.calc]\nmodule = \"calc.wasm\"\n",
    )
    .unwrap();

    let out = run(root, &["codegen"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("calc"));
    assert!(!root.join(".rpp/generated/calc.d.ts").exists());
    let root_config = std::fs::read_to_string(root.join("tsconfig.json")).unwrap();
    assert!(root_config.contains(".rpp/generated/*.d.ts"));

    std::fs::write(root.join("calc.wasm"), component_wasm()).unwrap();
    let out = run(root, &["codegen"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dts = std::fs::read_to_string(root.join(".rpp/generated/calc.d.ts")).unwrap();
    assert!(
        dts.contains("add: (a: number, b: number) => number;"),
        "{dts}"
    );
    assert!(dts.contains("calc: Calc;"), "{dts}");
    let sdk = std::fs::read_to_string(root.join(".rpp/sdk/index.ts")).unwrap();
    assert!(sdk.contains("): Component<ComponentMap[N]>;"), "{sdk}");
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

#[cfg(unix)]
#[test]
fn check_uses_compiler_bundled_beside_executable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    project(&root);
    let install = dir.path().join("install");
    let toolchain = install.join("toolchain/typescript/7.0.2");
    std::fs::create_dir_all(&toolchain).unwrap();
    let rpp = install.join("rpp");
    std::fs::copy(env!("CARGO_BIN_EXE_rpp"), &rpp).unwrap();
    let record = dir.path().join("args.txt");
    for (name, tag) in [("tsc", "bundled"), ("other-tsc", "env")] {
        let script = toolchain.join(name);
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho {tag} > '{}'\n", record.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let check = |tsc: Option<&Path>| {
        let mut command = std::process::Command::new(&rpp);
        command
            .current_dir(&root)
            .env("RPP_HOME", dir.path().join("home"))
            .env("RPP_CACHE_DIR", dir.path().join("cache"))
            .env_remove("RPP_TSC")
            .env("PATH", "/nonexistent")
            .arg("check");
        if let Some(tsc) = tsc {
            command.env("RPP_TSC", tsc);
        }
        command.output().unwrap()
    };
    let out = check(None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&record).unwrap().trim(), "bundled");

    let out = check(Some(&toolchain.join("other-tsc")));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&record).unwrap().trim(), "env");
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
