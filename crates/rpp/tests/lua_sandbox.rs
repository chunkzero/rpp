//! Sandbox enforcement and builtin module tests.

mod common;

use common::PluginDir;
use rpp::model::{PackFile, PluginFactory};

/// Run a one-shot processor and return its produced contents (or the load/run error message).
fn run(entry: &str, input: &str) -> Result<Vec<u8>, String> {
    let p = PluginDir::lua("t", entry);
    let options: toml::Value = toml::Value::Table(Default::default());
    let factory = match common::load_plugin(p.path(), options) {
        Ok(f) => f,
        Err(e) => return Err(format!("{e}")),
    };
    let mut inst = factory.instantiate().map_err(|e| format!("{e}"))?;
    let mut file = PackFile::new("in.txt", input.as_bytes().to_vec());
    inst.process("t", &mut file).map_err(|e| format!("{e}"))?;
    Ok(file.contents)
}

fn processor_wrap(body: &str) -> String {
    format!(
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("t", {{ files = {{ "**/*" }} }}, function(ctx, file)
{body}
end)
return plugin
"#
    )
}

#[test]
fn io_is_unavailable() {
    let err = run(&processor_wrap("io.write('x')"), "").unwrap_err();
    assert!(err.contains("'io'"), "{err}");
}

#[test]
fn os_execute_is_unavailable() {
    let err = run(&processor_wrap("os.execute('echo hi')"), "").unwrap_err();
    assert!(err.contains("'os'"), "{err}");
}

#[test]
fn host_time_is_unavailable_without_a_clock_grant() {
    let err = run(&processor_wrap("local _ = os.clock()"), "").unwrap_err();
    assert!(err.contains("'os'"), "{err}");
}

#[test]
fn host_randomness_is_unavailable_without_a_random_grant() {
    let err = run(&processor_wrap("local _ = math.random()"), "").unwrap_err();
    assert!(err.contains("'random'"), "{err}");
}

#[test]
fn load_is_unavailable() {
    let err = run(&processor_wrap("load('return 1')()"), "").unwrap_err();
    assert!(err.contains("'load'"), "{err}");
}

#[test]
fn dofile_is_unavailable() {
    let err = run(&processor_wrap("dofile('/etc/passwd')"), "").unwrap_err();
    assert!(err.contains("'dofile'"), "{err}");
}

#[test]
fn debug_is_unavailable() {
    let err = run(&processor_wrap("debug.getinfo(1)"), "").unwrap_err();
    assert!(err.contains("'debug'"), "{err}");
}

#[test]
fn require_parent_escape_blocked() {
    let err = run(&processor_wrap("require('../secret')"), "").unwrap_err();
    assert!(
        err.contains("not allowed") || err.contains("escape"),
        "{err}"
    );
}

#[test]
fn require_unknown_rpp_module_fails() {
    let err = run(&processor_wrap("require('rpp.bogus')"), "").unwrap_err();
    assert!(err.contains("bogus") || err.contains("not found"), "{err}");
}

#[test]
fn print_maps_to_log() {
    // print must exist and not error (mapped to log.info).
    let out = run(
        &processor_wrap("print('hello from plugin'); file.text='p'"),
        "",
    )
    .unwrap();
    assert_eq!(out, b"p");
}

#[test]
fn require_local_module_works() {
    let p = PluginDir::lua(
        "withmod",
        r#"
local rpp = require("rpp")
local helper = require("helper")
local plugin = rpp.plugin()
plugin:processor("t", { files = { "**/*" } }, function(ctx, file)
    file.text = helper.shout(file.text)
end)
return plugin
"#,
    )
    .with_module(
        "helper.lua",
        "return { shout = function(s) return s .. '!' end }",
    );

    let f = p.factory("");
    let mut inst = f.instantiate().unwrap();
    let mut file = PackFile::new("x.txt", b"hi".to_vec());
    inst.process("t", &mut file).unwrap();
    assert_eq!(file.contents, b"hi!");
}

#[test]
fn require_nested_init_module_works() {
    let p = PluginDir::lua(
        "nested",
        r#"
local rpp = require("rpp")
local lib = require("lib.core")
local plugin = rpp.plugin()
plugin:processor("t", { files = { "**/*" } }, function(ctx, file)
    file.text = lib.tag(file.text)
end)
return plugin
"#,
    )
    .with_module(
        "lib/core.lua",
        "return { tag = function(s) return '[' .. s .. ']' end }",
    );

    let f = p.factory("");
    let mut inst = f.instantiate().unwrap();
    let mut file = PackFile::new("x.txt", b"v".to_vec());
    inst.process("t", &mut file).unwrap();
    assert_eq!(file.contents, b"[v]");
}

#[test]
fn local_module_diagnostics_use_stable_relative_paths() {
    let plugin = PluginDir::lua(
        "stable-diagnostic",
        r#"
local rpp = require("rpp")
local helper = require("helper")
local plugin = rpp.plugin()
plugin:processor("t", { files = { "**/*" } }, function()
    helper.fail()
end)
return plugin
"#,
    )
    .with_module(
        "helper.lua",
        "return { fail = function() error('broken') end }",
    );

    let factory = plugin.factory("");
    let mut instance = factory.instantiate().unwrap();
    let mut file = PackFile::new("x.txt", Vec::new());
    let error = instance.process("t", &mut file).unwrap_err().to_string();

    assert!(error.contains("helper.lua"), "{error}");
    assert!(
        !error.contains(&plugin.path().display().to_string()),
        "{error}"
    );
}

#[test]
fn json_builtin_roundtrips() {
    let out = run(
        &processor_wrap(
            r#"
local data = rpp.json.decode(file.text)
data.added = true
file.text = rpp.json.encode(data)
"#,
        ),
        "{\"a\":1}",
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["a"], 1);
    assert_eq!(v["added"], true);
}

#[test]
fn toml_builtin_roundtrips() {
    let out = run(
        &processor_wrap(
            r#"
local data = rpp.toml.decode(file.text)
file.text = rpp.toml.encode({ name = data.name, n = 5 })
"#,
        ),
        "name = \"abc\"\n",
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("name = \"abc\""), "{s}");
    assert!(s.contains("n = 5"), "{s}");
}

#[test]
fn hash_builtins() {
    let out = run(
        &processor_wrap(
            r#"
local parts = {
    rpp.hash.xxh3("abc"),
    rpp.hash.sha256("abc"),
    rpp.hash.md5("abc"),
    tostring(rpp.hash.crc32("abc")),
}
file.text = table.concat(parts, "|")
"#,
        ),
        "",
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let parts: Vec<&str> = s.split('|').collect();
    // sha256("abc")
    assert_eq!(
        parts[1],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // md5("abc")
    assert_eq!(parts[2], "900150983cd24fb0d6963f7d28e17f72");
    // crc32("abc") = 891568578
    assert_eq!(parts[3], "891568578");
    // xxh3 is 16 hex chars
    assert_eq!(parts[0].len(), 16);
}

#[test]
fn path_builtins() {
    let out = run(
        &processor_wrap(
            r#"
local p = "a/b/c.txt"
file.text = table.concat({
    rpp.path.dirname(p),
    rpp.path.basename(p),
    rpp.path.ext(p),
    rpp.path.with_ext(p, "json"),
    rpp.path.join("x", "y/z", "w"),
    tostring(rpp.path.match("a/**/*.txt", p)),
}, "|")
"#,
        ),
        "",
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    assert_eq!(s, "a/b|c.txt|txt|a/b/c.json|x/y/z/w|true");
}

#[test]
fn duplicate_processor_rejected() {
    let p = PluginDir::lua(
        "dup-proc",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("t", { files = { "**/*" } }, function() end)
plugin:processor("t", { files = { "*.txt" } }, function() end)
return plugin
"#,
    );
    let Err(err) = common::load_plugin(p.path(), toml::Value::Table(toml::map::Map::new())) else {
        panic!("expected duplicate processor registration to fail at load");
    };
    let msg = format!("{err}");
    assert!(msg.contains("already registered"), "{msg}");
}

#[test]
fn duplicate_generator_rejected() {
    let p = PluginDir::lua(
        "dup-gen",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("a", function() end)
plugin:generator("b", function() end)
return plugin
"#,
    );
    let Err(err) = common::load_plugin(p.path(), toml::Value::Table(toml::map::Map::new())) else {
        panic!("expected duplicate generator registration to fail at load");
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("at most one generator") || msg.contains("generator"),
        "{msg}"
    );
}

#[test]
fn cache_key_depends_on_options() {
    let entry = r#"
local rpp = require("rpp")
return rpp.plugin()
"#;
    let p = PluginDir::lua("opts", entry);

    let mut a = toml::map::Map::new();
    a.insert("flag".into(), toml::Value::Boolean(true));
    let f1 = common::load_plugin(p.path(), toml::Value::Table(a)).expect("load");

    let f2 =
        common::load_plugin(p.path(), toml::Value::Table(toml::map::Map::new())).expect("load");

    assert_ne!(f1.cache_key(), f2.cache_key());

    let f3 =
        common::load_plugin(p.path(), toml::Value::Table(toml::map::Map::new())).expect("load");
    assert_eq!(f2.cache_key(), f3.cache_key());
}

#[test]
fn cache_key_depends_on_declared_component_bytes() {
    let plugin = PluginDir::lua(
        "component-key",
        "local rpp = require('rpp')\nreturn rpp.plugin()\n",
    );
    std::fs::write(
        plugin.path().join("plugin.toml"),
        "[plugin]\nid = \"component-key\"\nversion = \"1.0.0\"\n\n[component.compiler]\nmodule = \"compiler.wasm\"\n",
    )
    .unwrap();
    std::fs::write(plugin.path().join("compiler.wasm"), b"first").unwrap();
    let first = plugin.factory("").cache_key();
    std::fs::write(plugin.path().join("compiler.wasm"), b"second").unwrap();
    let second = plugin.factory("").cache_key();
    assert_ne!(first, second);
}

#[test]
fn str_builtins() {
    let out = run(
        &processor_wrap(
            r#"
file.text = table.concat({
    tostring(rpp.str.starts_with("hello", "he")),
    tostring(rpp.str.ends_with("hello", "lo")),
    rpp.str.trim("  hi  "),
    table.concat(rpp.str.split("a,b,c", ","), "-"),
}, "|")
"#,
        ),
        "",
    )
    .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "true|true|hi|a-b-c");
}

#[test]
fn granted_load_stays_inside_the_sandbox() {
    use rpp::config::{LuaCapability, SecurityMode};
    use rpp::lua::{LuaPluginFactory, LuaPluginLimits, PackInfo, RuntimeAccess};

    let p = PluginDir::lua(
        "loader",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("t", { files = { "**/*" } }, function(ctx, file)
    local chunk = assert(load("return io, os, require"))
    local io_value, os_value, require_value = chunk()
    file.text = tostring(io_value) .. "," .. tostring(os_value) .. "," .. type(require_value)
end)
return plugin
"#,
    );
    let mut access = RuntimeAccess::sandboxed(".".into());
    access.security = SecurityMode::Trusted;
    access.permissions.lua = vec![LuaCapability::Load];
    let factory = LuaPluginFactory::load(
        p.path(),
        toml::Value::Table(Default::default()),
        PackInfo {
            name: "pack".into(),
            description: None,
            format: None,
        },
        LuaPluginLimits::default(),
        access,
    )
    .unwrap();
    let mut inst = factory.instantiate().unwrap();
    let mut file = PackFile::new("in.txt", Vec::new());
    inst.process("t", &mut file).unwrap();
    assert_eq!(file.contents, b"nil,nil,function");
}

/// Load a sandboxed plugin with explicit limits and run its `t` processor.
fn run_limited(entry: &str, limits: rpp::lua::LuaPluginLimits) -> Result<Vec<u8>, String> {
    use rpp::lua::{LuaPluginFactory, PackInfo, RuntimeAccess};
    let p = PluginDir::lua("t", entry);
    let factory = LuaPluginFactory::load(
        p.path(),
        toml::Value::Table(Default::default()),
        PackInfo {
            name: "pack".into(),
            description: None,
            format: None,
        },
        limits,
        RuntimeAccess::sandboxed(".".into()),
    )
    .map_err(|e| format!("{e}"))?;
    let mut inst = factory.instantiate().map_err(|e| format!("{e}"))?;
    let mut file = PackFile::new("in.txt", Vec::new());
    inst.process("t", &mut file).map_err(|e| format!("{e}"))?;
    Ok(file.contents)
}

#[test]
fn memory_limit_is_enforced() {
    let limits = rpp::lua::LuaPluginLimits {
        memory_limit: 2 * 1024 * 1024,
        ..Default::default()
    };
    let err = run_limited(
        &processor_wrap("local s = string.rep('x', 1024 * 1024); local t = {} for i = 1, 64 do t[i] = s .. i end"),
        limits,
    )
    .unwrap_err();
    assert!(err.contains("memory"), "{err}");
}

#[test]
fn execution_deadline_is_enforced() {
    let limits = rpp::lua::LuaPluginLimits {
        execution_limit: std::time::Duration::from_millis(200),
        ..Default::default()
    };
    let err = run_limited(&processor_wrap("while true do end"), limits).unwrap_err();
    assert!(err.contains("deadline"), "{err}");
}

#[test]
fn process_execution_requires_a_grant() {
    let err = run(
        &processor_wrap("require('rpp.process').run({ program = 'echo', args = { 'hi' } })"),
        "",
    )
    .unwrap_err();
    assert!(err.contains("process permissions"), "{err}");
}
