//! Window-shaped end-to-end host contract: TypeScript authoring sources call a typed
//! WASIp2 component and emit binary pack files plus declared external sources.

mod common;

use std::path::{Path, PathBuf};

use rpp::engine::BuildResult;
use rpp_cli::project::Project;

use common::{build_wasm_guest, wasip2_available, write};

const INPUT_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 0, 0xff, 0x1a, b'\n'];

/// Build the project at `root` with one worker.
fn build(root: &Path) -> anyhow::Result<BuildResult> {
    let mut project = Project::discover(root)?;
    project.config.build.workers = 1;
    Ok(project.build_engine(&mut None)?.build()?)
}

fn clean(root: &Path) {
    Project::discover(root).unwrap().clean_artifacts().unwrap();
}

/// Sorted `(relative path, bytes)` snapshot of a directory tree.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                out.push((relative, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    if root.exists() {
        walk(root, root, &mut out);
    }
    out.sort();
    out
}

fn no_changes(result: &BuildResult) -> bool {
    result.changes.written.is_empty()
        && result.changes.removed.is_empty()
        && result.changes.external.written.is_empty()
        && result.changes.external.removed.is_empty()
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/window-host-component")
}

fn build_component(target_dir: &Path, v2: bool) -> PathBuf {
    let features: &[&str] = if v2 { &["--features", "v2"] } else { &[] };
    build_wasm_guest(
        &fixture(),
        target_dir,
        "window_host_component.wasm",
        false,
        features,
    )
}

fn config(hud_shaders: bool) -> String {
    format!(
        r##"import {{ defineConfig, plugin }} from "#rpp/config";

export default defineConfig({{
  pack: {{ name: "window-host-fixture", format: 84 }},
  build: {{ source: "src", output: "dist", workers: 1 }},
  plugins: [
    plugin(
      "window-host",
      {{ namespace: "window", kotlinPackage: "dev.example.generated", hudShaders: {hud_shaders} }},
      {{ outputs: {{ kotlin: "server/generated" }} }},
    ),
  ],
}});
"##
    )
}

/// A plugin that compiles `window/**` sources through the `compiler` component.
const COMPILE_PLUGIN: &str = r##"import { components, definePlugin } from "#rpp";

interface Options {
  namespace: string;
  kotlinPackage: string;
  hudShaders?: boolean;
}

export default definePlugin<Options>({
  generate(ctx) {
    const compiler = components.load("compiler");
    const project = {
      themes: [],
      windows: [] as unknown[],
      huds: [],
      options: { hud_shaders: ctx.options.hudShaders === true },
      target: { pack_format: ctx.pack.format.min },
    };
    const files: { path: string; contents: Uint8Array }[] = [];
    for (const path of ctx.sourceFiles("window/**")) {
      if (path.endsWith(".json")) {
        const document = JSON.parse(ctx.readSourceText(path)!);
        project.windows.push(...(document.windows ?? []));
      } else {
        files.push({ path, contents: ctx.readSource(path)! });
      }
      ctx.remove(path);
    }

    const result = compiler.exports.compile(
      ctx.options.namespace,
      JSON.stringify(project),
      files,
      ctx.options.kotlinPackage,
    );
    for (const file of result.files) ctx.emit(file.path, file.contents);
    for (const file of result.kotlinFiles) ctx.emitOutput("kotlin", file.path, file.contents);
    for (const warning of result.warnings) console.warn(warning);
  },
});
"##;

fn scaffold(root: &Path, component: &Path) {
    write(root, "rpp.config.ts", config(true));
    write(
        root,
        "rpp.json",
        r#"{ "dependencies": { "window-host": "path:plugin" } }"#,
    );
    write(
        root,
        "src/window/ui.json",
        r#"{ "windows": [{ "name": "fixture" }] }"#,
    );
    write(root, "src/window/input.bin", INPUT_BYTES);

    write(
        root,
        "plugin/rpp.json",
        r#"{
  "name": "window-host",
  "version": "0.1.0",
  "entry": "src/plugin.ts",
  "overrides": ["window/**"],
  "components": { "compiler": "compiler.wasm" }
}"#,
    );
    std::fs::copy(component, root.join("plugin/compiler.wasm")).unwrap();
    write(root, "plugin/src/plugin.ts", COMPILE_PLUGIN);
}

/// Two cold builds must be byte-identical, and a warm build a no-op.
fn assert_cold_builds_deterministic_and_warm_build_is_noop(root: &Path) {
    let first = build(root).unwrap();
    assert_eq!(first.generated, 1);
    let first_dist = snapshot(&root.join("dist"));
    let first_external = snapshot(&root.join("server"));
    clean(root);
    let second = build(root).unwrap();
    assert_eq!(second.generated, 1);
    assert_eq!(snapshot(&root.join("dist")), first_dist);
    assert_eq!(snapshot(&root.join("server")), first_external);
    let warm = build(root).unwrap();
    assert_eq!(warm.generated, 0);
    assert!(no_changes(&warm), "warm build rewrote outputs");
}

fn assert_generated_bin(root: &Path, version: u8) {
    let mut expected = vec![b'R', b'P', b'P', version, 0, 0xff];
    expected.extend(INPUT_BYTES);
    assert_eq!(
        std::fs::read(root.join("dist/assets/window/generated.bin")).unwrap(),
        expected
    );
}

fn assert_v1_outputs(root: &Path) {
    assert_generated_bin(root, b'1');
    assert!(!root.join("dist/window/ui.json").exists());
    assert!(!root.join("dist/window/input.bin").exists());
    assert_eq!(
        std::fs::read(root.join("server/generated/WindowPack.kt")).unwrap(),
        b"// generated schema v4 revision 1\nobject WindowPack\n"
    );
}

fn assert_v2_outputs_replace_v1_and_keep_handwritten(
    root: &Path,
    changed: &BuildResult,
    handwritten: &Path,
) {
    assert_generated_bin(root, b'2');
    assert!(!root.join("server/generated/WindowPack.kt").exists());
    assert_eq!(
        std::fs::read(handwritten).unwrap(),
        b"object HandWritten\n",
        "stale cleanup must preserve files RPP does not own"
    );
    let renamed = root.join("server/generated/RenamedWindowPack.kt");
    assert_eq!(
        std::fs::read(&renamed).unwrap(),
        b"// generated schema v4 revision 2\nobject RenamedWindowPack\n"
    );
    assert!(changed.changes.external.written.contains(&renamed));
    assert!(changed
        .changes
        .external
        .removed
        .contains(&root.join("server/generated/WindowPack.kt")));
}

#[test]
fn window_shaped_component_build_replays_and_invalidates() {
    if !wasip2_available() {
        eprintln!("SKIP: wasm32-wasip2 target is unavailable");
        return;
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    let component_target = temporary.path().join("component-target");
    std::fs::create_dir_all(&root).unwrap();

    let v1 = build_component(&component_target, false);
    scaffold(&root, &v1);
    assert_cold_builds_deterministic_and_warm_build_is_noop(&root);
    assert_v1_outputs(&root);
    let handwritten = root.join("server/generated/HandWritten.kt");
    std::fs::write(&handwritten, b"object HandWritten\n").unwrap();

    // Dropping only the cache (`rpp build --no-cache`) keeps external
    // ownership, so a rebuild neither rewrites nor removes anything.
    std::fs::remove_dir_all(root.join(".rpp/cache")).unwrap();
    let no_cache = build(&root).unwrap();
    assert_eq!(no_cache.generated, 1);
    assert!(no_changes(&no_cache));

    let v2 = build_component(&component_target, true);
    std::fs::copy(v2, root.join("plugin/compiler.wasm")).unwrap();
    let changed = build(&root).unwrap();
    assert_eq!(changed.generated, 1);
    assert_v2_outputs_replace_v1_and_keep_handwritten(&root, &changed, &handwritten);

    let v2_warm = build(&root).unwrap();
    assert_eq!(v2_warm.generated, 0);
    assert!(no_changes(&v2_warm));
}

#[test]
fn window_component_diagnostic_keeps_stable_plugin_context() {
    if !wasip2_available() {
        eprintln!("SKIP: wasm32-wasip2 target is unavailable");
        return;
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    let component_target = temporary.path().join("component-target");
    std::fs::create_dir_all(&root).unwrap();
    let component = build_component(&component_target, false);
    scaffold(&root, &component);

    write(&root, "rpp.config.ts", config(false));

    let error = format!("{:#}", build(&root).unwrap_err());
    assert!(
        error.contains("plugin `window-host` generator failed"),
        "{error}"
    );
    assert!(
        error.contains("Window schema v4 requires hud_shaders=true and host pack_format=84"),
        "{error}"
    );
}

/// A plugin that round-trips option-shaped values through the component twice.
const ROUND_TRIP_PLUGIN: &str = r##"import { components, definePlugin } from "#rpp";

const check = (condition: boolean, message: string): void => {
  if (!condition) throw new Error(message);
};

export default definePlugin({
  generate(ctx) {
    const compiler = components.load("compiler");
    let values: any = {
      items: [undefined, false, undefined],
      pair: [false, undefined],
      nested: [{ tag: "none" }, { tag: "some", val: undefined }, { tag: "some", val: false }],
      success: { tag: "ok", val: undefined },
      failure: { tag: "err", val: undefined },
      boolean: { tag: "ok", val: false },
      empty: { tag: "ok" },
    };
    for (let round = 0; round < 2; round++) {
      values = compiler.exports.roundTripOptions(values);
      check(values.items.length === 3 && values.items[0] === undefined, "items");
      check(values.items[1] === false && values.items[2] === undefined, "items");
      check(values.pair.length === 2 && values.pair[0] === false, "pair");
      check(values.pair[1] === undefined, "pair");
      check(values.nested[0].tag === "none", "nested none");
      check(values.nested[1].tag === "some" && values.nested[1].val === undefined, "nested some");
      check(values.nested[2].val === false, "nested value");
      check(values.success.tag === "ok" && values.success.val === undefined, "success");
      check(values.failure.tag === "err" && values.failure.val === undefined, "failure");
      check(values.boolean.tag === "ok" && values.boolean.val === false, "boolean");
      check(values.empty.tag === "ok", "empty");
    }
    ctx.emit("round-trip.txt", "passed");
  },
});
"##;

#[test]
fn component_options_round_trip_without_losing_positions_or_branches() {
    if !wasip2_available() {
        eprintln!("SKIP: wasm32-wasip2 target is unavailable");
        return;
    }
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.path().join("component-target"))
        .join("option-guest");
    let component = build_component(&target, false);
    scaffold(&root, &component);
    write(&root, "plugin/src/plugin.ts", ROUND_TRIP_PLUGIN);
    assert_eq!(build(&root).unwrap().generated, 1);
    assert_eq!(
        std::fs::read(root.join("dist/round-trip.txt")).unwrap(),
        b"passed"
    );
}
