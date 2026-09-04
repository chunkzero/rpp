//! Window-shaped end-to-end host contract: Lua authoring sources call a typed
//! WASIp2 component and emit binary pack files plus declared external sources.

use std::path::{Path, PathBuf};
use std::process::Command;

use rpp_cli::harness::BuildHarness;

const INPUT_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 0, 0xff, 0x1a, b'\n'];

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/window-host-component")
}

fn wasip2_available() -> bool {
    let Ok(output) = Command::new("rustc").args(["--print", "sysroot"]).output() else {
        return false;
    };
    Path::new(String::from_utf8_lossy(&output.stdout).trim())
        .join("lib/rustlib/wasm32-wasip2/lib")
        .is_dir()
}

fn build_component(target_dir: &Path, v2: bool) -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args([
            "build",
            "--locked",
            "--target",
            "wasm32-wasip2",
        ])
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(fixture());
    if v2 {
        command.args(["--features", "v2"]);
    }
    let output = command.output().expect("build component fixture");
    assert!(
        output.status.success(),
        "component fixture failed to build:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    target_dir.join("wasm32-wasip2/debug/window_host_component.wasm")
}

fn scaffold(root: &Path, component: &Path) {
    std::fs::write(
        root.join("rpp.toml"),
        r#"[pack]
name = "window-host-fixture"
pack_format = 84

[build]
source = "src"
output = "dist"
workers = 1

[[plugin]]
source = "path:plugin"
[plugin.options]
namespace = "window"
kotlin_package = "dev.example.generated"
hud_shaders = true
[plugin.outputs]
kotlin = "server/generated"
"#,
    )
    .unwrap();

    let sources = root.join("src/window");
    std::fs::create_dir_all(&sources).unwrap();
    std::fs::write(
        root.join("src/pack.mcmeta"),
        r#"{"pack":{"pack_format":84,"description":"fixture"}}"#,
    )
    .unwrap();
    std::fs::write(
        sources.join("ui.lua"),
        r#"return { windows = { { name = "fixture" } } }"#,
    )
    .unwrap();
    std::fs::write(sources.join("input.bin"), INPUT_BYTES).unwrap();

    let plugin = root.join("plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::copy(component, plugin.join("compiler.wasm")).unwrap();
    std::fs::write(
        plugin.join("plugin.toml"),
        r#"[plugin]
id = "window-host"
version = "0.1.0"
entry = "init.lua"

[component.compiler]
module = "compiler.wasm"
"#,
    )
    .unwrap();
    std::fs::write(
        plugin.join("init.lua"),
        r#"local rpp = require("rpp")
local json = require("rpp.json")
local compiler = require("rpp.component").load("compiler")
local plugin = rpp.plugin()

plugin:generator("window", function(ctx)
    local project = {
        themes = {},
        windows = {},
        huds = {},
        options = { hud_shaders = ctx.options.hud_shaders == true },
        target = { pack_format = ctx.pack.format },
    }
    local files = {}
    for _, path in ipairs(ctx:source_files("window/**")) do
        if string.sub(path, -4) == ".lua" then
            local document = ctx:load_source(path)
            for _, window in ipairs(document.windows or {}) do
                project.windows[#project.windows + 1] = window
            end
        else
            files[#files + 1] = { path = path, contents = ctx:read_source(path) }
        end
        ctx:remove(path)
    end

    local result = compiler:call(
        "compile",
        ctx.options.namespace,
        json.encode(project),
        files,
        ctx.options.kotlin_package
    )
    if result.err ~= nil then
        error(result.err)
    end
    for _, file in ipairs(result.ok.files) do
        ctx:emit(file.path, file.contents)
    end
    for _, file in ipairs(result.ok["kotlin-files"]) do
        ctx:emit_output("kotlin", file.path, file.contents)
    end
    for _, warning in ipairs(result.ok.warnings) do
        ctx.log.warn(warning)
    end
end)

return plugin
"#,
    )
    .unwrap();
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
    let harness = BuildHarness::new(&root);

    let reproducible = harness.verify_reproducible().unwrap();
    assert_eq!(reproducible.cold.generated, 1);
    assert_eq!(reproducible.warm.generated, 0);
    let mut expected_v1 = vec![b'R', b'P', b'P', b'1', 0, 0xff];
    expected_v1.extend(INPUT_BYTES);
    assert_eq!(
        std::fs::read(root.join("dist/assets/window/generated.bin")).unwrap(),
        expected_v1
    );
    assert!(!root.join("dist/window/ui.lua").exists());
    assert!(!root.join("dist/window/input.bin").exists());
    assert_eq!(
        std::fs::read(root.join("server/generated/WindowPack.kt")).unwrap(),
        b"// generated schema v4 revision 1\nobject WindowPack\n"
    );
    let handwritten = root.join("server/generated/HandWritten.kt");
    std::fs::write(&handwritten, b"object HandWritten\n").unwrap();

    let no_cache = harness.build_no_cache().unwrap();
    assert_eq!(no_cache.result.generated, 1);
    assert!(no_cache.result.changes.written.is_empty());
    assert!(no_cache.result.changes.removed.is_empty());
    assert!(no_cache.result.changes.external.written.is_empty());
    assert!(no_cache.result.changes.external.removed.is_empty());

    let v2 = build_component(&component_target, true);
    std::fs::copy(v2, root.join("plugin/compiler.wasm")).unwrap();
    let changed = harness.build().unwrap();
    assert_eq!(changed.result.generated, 1);
    let mut expected_v2 = vec![b'R', b'P', b'P', b'2', 0, 0xff];
    expected_v2.extend(INPUT_BYTES);
    assert_eq!(
        std::fs::read(root.join("dist/assets/window/generated.bin")).unwrap(),
        expected_v2
    );
    assert!(!root.join("server/generated/WindowPack.kt").exists());
    assert_eq!(
        std::fs::read(&handwritten).unwrap(),
        b"object HandWritten\n",
        "stale cleanup must preserve files RPP does not own"
    );
    let renamed = root.join("server/generated/RenamedWindowPack.kt");
    assert_eq!(
        std::fs::read(&renamed).unwrap(),
        b"// generated schema v4 revision 2\nobject RenamedWindowPack\n"
    );
    assert!(changed.result.changes.external.written.contains(&renamed));
    assert!(changed
        .result
        .changes
        .external
        .removed
        .contains(&root.join("server/generated/WindowPack.kt")));

    let v2_warm = harness.build().unwrap();
    assert_eq!(v2_warm.result.generated, 0);
    assert!(v2_warm.result.changes.written.is_empty());
    assert!(v2_warm.result.changes.external.written.is_empty());
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

    let config_path = root.join("rpp.toml");
    let config = std::fs::read_to_string(&config_path)
        .unwrap()
        .replace("hud_shaders = true", "hud_shaders = false");
    std::fs::write(&config_path, config).unwrap();

    let error = BuildHarness::new(&root).build().unwrap_err().to_string();
    assert!(
        error.contains("plugin `window-host` generator failed"),
        "{error}"
    );
    assert!(
        error.contains("Window schema v4 requires hud_shaders=true and host pack_format=84"),
        "{error}"
    );
}
