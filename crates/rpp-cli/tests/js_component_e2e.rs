//! TypeScript plugins calling a typed WASIp2 component through `components.load`.

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rpp_cli::project::Project;

use common::{build_wasm_guest, wasip2_available, write};

/// Build the guest fixture once per test binary and return the component path.
fn component() -> &'static Path {
    static COMPONENT: OnceLock<PathBuf> = OnceLock::new();
    COMPONENT.get_or_init(|| {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        build_wasm_guest(
            &manifest.join("tests/fixtures/js-component"),
            &manifest.join("../../target/js-component-fixture"),
            "js_component.wasm",
            false,
            &[],
        )
    })
}

/// A project whose only plugin is `plugin_ts`, with the fixture declared as component `c`.
fn scaffold(root: &Path, plugin_ts: &str) {
    write(
        root,
        "rpp.json",
        r#"{ "dependencies": { "js-component": "path:plugin" } }"#,
    );
    write(
        root,
        "rpp.config.ts",
        r##"import { defineConfig, plugin } from "#rpp/config";

export default defineConfig({
  pack: { name: "js-component", format: 34 },
  build: { workers: 1, wasm: { executionDeadlineSeconds: 1 } },
  plugins: [plugin("js-component")],
});
"##,
    );
    write(
        root,
        "plugin/rpp.json",
        r#"{ "name": "js-component", "version": "0.1.0", "entry": "plugin.ts",
  "components": { "c": "c.wasm" } }"#,
    );
    write(root, "plugin/plugin.ts", plugin_ts);
    std::fs::copy(component(), root.join("plugin/c.wasm")).unwrap();
    write(root, "src/a.txt", "a");
    write(root, "src/b.txt", "b");
}

/// Build a generator plugin whose `generate` body is `body`.
fn generate(body: &str) -> anyhow::Result<tempfile::TempDir> {
    let plugin = format!(
        r##"import {{ components, definePlugin, ComponentError, ComponentTimeoutError }} from "#rpp";
const json = (value: unknown) =>
  JSON.stringify(value, (_key, v) => (typeof v === "bigint" ? `${{v}}n` : v));
export default definePlugin({{
  generate(ctx) {{
    const c = components.load("c");
    {body}
  }},
}});
"##
    );
    build(&plugin)
}

fn build(plugin_ts: &str) -> anyhow::Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    scaffold(dir.path(), plugin_ts);
    let project = Project::discover(dir.path())?;
    project.build_engine(&mut None)?.build()?;
    Ok(dir)
}

fn output(dir: &tempfile::TempDir, name: &str) -> Vec<u8> {
    std::fs::read(dir.path().join("dist").join(name)).unwrap()
}

fn text(dir: &tempfile::TempDir, name: &str) -> String {
    String::from_utf8(output(dir, name)).unwrap()
}

macro_rules! require_wasip2 {
    () => {
        if !wasip2_available() {
            eprintln!("SKIP: wasm32-wasip2 target is unavailable");
            return;
        }
    };
}

#[test]
fn ts_plugin_calls_component_without_bridge() {
    require_wasip2!();
    let dir = generate(r#"ctx.emit("out.txt", String(c.exports.parse("41") + 1));"#).unwrap();
    assert_eq!(text(&dir, "out.txt"), "42");
}

#[test]
fn binary_payload_round_trips() {
    require_wasip2!();
    let dir = generate(
        r#"const all = Uint8Array.from({ length: 256 }, (_, i) => i);
    ctx.emit("all.bin", c.exports.echoBytes(all));
    ctx.emit("empty.bin", c.exports.echoBytes(new Uint8Array(0)));"#,
    )
    .unwrap();
    assert_eq!(output(&dir, "all.bin"), (0..=255).collect::<Vec<u8>>());
    assert!(output(&dir, "empty.bin").is_empty());
}

#[test]
fn bigint_and_nested_option_round_trip() {
    require_wasip2!();
    let dir = generate(
        r#"ctx.emit("wide.json", json(c.exports.wide(2n ** 64n - 1n, -(2n ** 63n))));
    ctx.emit("nest.json", json([
      c.exports.nest({ tag: "none" }),
      c.exports.nest({ tag: "some", val: undefined }),
      c.exports.nest({ tag: "some", val: 5 }),
    ]));"#,
    )
    .unwrap();
    assert_eq!(
        text(&dir, "wide.json"),
        r#"["18446744073709551615n","-9223372036854775808n"]"#
    );
    assert_eq!(
        text(&dir, "nest.json"),
        r#"[{"tag":"none"},{"tag":"some"},{"tag":"some","val":5}]"#
    );
}

#[test]
fn variant_enum_flags_record_camel_case() {
    require_wasip2!();
    let dir = generate(
        r#"const empty = c.exports.describe({
      fillColor: "red",
      perms: { read: true, writeAll: false },
      shape: { tag: "empty" },
    });
    const circle = c.exports.describe({
      fillColor: "dark-blue",
      perms: { read: false, writeAll: false },
      shape: { tag: "circle", val: 2 },
    });
    ctx.emit("describe.json", json([empty, circle]));"#,
    )
    .unwrap();
    assert_eq!(
        text(&dir, "describe.json"),
        concat!(
            r#"[{"fillColor":"dark-blue","perms":{"read":true,"writeAll":true},"#,
            r#""shape":{"tag":"circle","val":1.5}},"#,
            r#"{"fillColor":"red","perms":{"read":false,"writeAll":true},"#,
            r#""shape":{"tag":"circle","val":4}}]"#
        )
    );
}

#[test]
fn result_err_throws_component_error_with_payload() {
    require_wasip2!();
    let dir = generate(
        r#"try {
      c.exports.parse("nope");
    } catch (error) {
      if (!(error instanceof ComponentError)) throw error;
      ctx.emit("err.json", json({ component: error.component, export: error.export, payload: error.payload }));
    }"#,
    )
    .unwrap();
    assert_eq!(
        text(&dir, "err.json"),
        r#"{"component":"c","export":"parse","payload":{"code":7,"message":"invalid digit found in string"}}"#
    );
}

#[test]
fn spin_throws_component_timeout_error() {
    require_wasip2!();
    let dir = generate(
        r#"let report = "no error";
    try {
      c.exports.spin();
    } catch (error) {
      if (!(error instanceof ComponentTimeoutError)) throw error;
      report = `${error.component}:${error.export}`;
    }
    let poisoned = false;
    try {
      c.exports.parse("1");
    } catch {
      poisoned = true;
    }
    ctx.emit("timeout.txt", `${report}:${poisoned}`);"#,
    )
    .unwrap();
    assert_eq!(text(&dir, "timeout.txt"), "c:spin:true");
}

#[test]
fn handle_released_between_processor_files() {
    require_wasip2!();
    let error = build(
        r##"import { components, definePlugin } from "#rpp";
let c: ReturnType<typeof components.load> | undefined;
export default definePlugin({
  processors: {
    parse: {
      files: ["*.txt"],
      run(_ctx, file) {
        c ??= components.load("c");
        file.text = String(c.exports.parse("7"));
      },
    },
  },
});
"##,
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("component handle released"),
        "{error:#}"
    );
}

#[test]
fn interface_exports_are_namespaced() {
    require_wasip2!();
    let dir = generate(r#"ctx.emit("frob.txt", json(c.exports.tools.frob(5n)));"#).unwrap();
    assert_eq!(text(&dir, "frob.txt"), "\"15n\"");
}

#[test]
fn compiled_components_are_cached_outside_the_project() {
    require_wasip2!();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(
        root,
        r##"import { components, definePlugin } from "#rpp";
export default definePlugin({ generate(ctx) { ctx.emit("out.txt", String(components.load("c").exports.parse("1"))); } });
"##,
    );
    let wasmtime = root.join(".test-rpp-cache/wasmtime");
    let has_entries = || walk_files(&wasmtime) > 0;

    let built = common::build(root, &[]);
    assert!(built.status.success(), "{built:?}");
    assert!(
        has_entries(),
        "no compiled code under {}",
        wasmtime.display()
    );
    assert!(!root.join(".rpp/cache/wasmtime").exists());

    let cleaned = common::run(root, &["clean"]);
    assert!(cleaned.status.success(), "{cleaned:?}");
    assert!(
        has_entries(),
        "`rpp clean` removed the shared compilation cache"
    );

    let unusable = root.join("cache-file");
    std::fs::write(&unusable, "").unwrap();
    let uncached = common::command(root)
        .env("RPP_CACHE_DIR", &unusable)
        .arg("build")
        .output()
        .unwrap();
    assert!(uncached.status.success(), "{uncached:?}");
    assert!(String::from_utf8_lossy(&uncached.stderr).contains("without a persistent cache"));
    assert_eq!(text_at(root, "out.txt"), "1");
}

fn walk_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|entry| match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk_files(&entry.path()),
                _ => 1,
            })
            .sum()
    })
}

fn text_at(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join("dist").join(name)).unwrap()
}
