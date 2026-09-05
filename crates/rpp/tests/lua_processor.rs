//! Lua processor semantics: mutate, rename, drop, options, ctx.

mod common;

use common::PluginDir;
use rpp::model::{PackFile, PluginFactory, ProcessOutcome};

fn process(
    factory: &dyn PluginFactory,
    processor: &str,
    path: &str,
    body: &str,
) -> (PackFile, ProcessOutcome) {
    let mut inst = factory.instantiate().unwrap();
    let mut file = PackFile::new(path, body.as_bytes().to_vec());
    let outcome = inst.process(processor, &mut file).unwrap();
    (file, outcome)
}

#[test]
fn processor_mutates_text() {
    let p = PluginDir::lua(
        "upper",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("up", { files = { "**/*.txt" } }, function(ctx, file)
    file.text = string.upper(file.text)
end)
return plugin
"#,
    );
    let f = p.factory("");
    let (file, outcome) = process(&f, "up", "a/b.txt", "hello");
    assert_eq!(outcome, ProcessOutcome::Modified);
    assert_eq!(file.contents, b"HELLO");
}

#[test]
fn processor_unchanged_when_no_mutation() {
    let p = PluginDir::lua(
        "noop",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("noop", { files = { "**/*" } }, function(ctx, file) end)
return plugin
"#,
    );
    let f = p.factory("");
    let (file, outcome) = process(&f, "noop", "x.txt", "data");
    assert_eq!(outcome, ProcessOutcome::Unchanged);
    assert_eq!(file.contents, b"data");
}

#[test]
fn processor_renames_via_path() {
    let p = PluginDir::lua(
        "rename",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("rename", { files = { "**/*" } }, function(ctx, file)
    file.path = rpp.path.with_ext(file.path, "out")
end)
return plugin
"#,
    );
    let f = p.factory("");
    let (file, outcome) = process(&f, "rename", "dir/name.txt", "x");
    assert_eq!(outcome, ProcessOutcome::Modified);
    assert_eq!(file.path, "dir/name.out");
}

#[test]
fn processor_drops_file() {
    let p = PluginDir::lua(
        "dropper",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("drop", { files = { "**/*" } }, function(ctx, file)
    file:drop()
end)
return plugin
"#,
    );
    let f = p.factory("");
    let (_file, outcome) = process(&f, "drop", "x.txt", "x");
    assert_eq!(outcome, ProcessOutcome::Dropped);
}

#[test]
fn processor_reads_options() {
    let p = PluginDir::lua(
        "opt",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("opt", { files = { "**/*" } }, function(ctx, file)
    file.text = ctx.options.prefix .. file.text
end)
return plugin
"#,
    );
    let f = p.factory("prefix = \"P:\"");
    let (file, _o) = process(&f, "opt", "x.txt", "body");
    assert_eq!(file.contents, b"P:body");
}

#[test]
fn processor_sees_pack_info() {
    let p = PluginDir::lua(
        "packinfo",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", { files = { "**/*" } }, function(ctx, file)
    file.text = ctx.pack.name .. ":" .. tostring(ctx.pack.format)
end)
return plugin
"#,
    );
    let f = p.factory("");
    let (file, _o) = process(&f, "p", "x.txt", "");
    assert_eq!(file.contents, b"test-pack:34");
}

#[test]
fn processor_error_attributes_plugin_and_file() {
    let p = PluginDir::lua(
        "boom",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("boom", { files = { "**/*" } }, function(ctx, file)
    error("kaboom")
end)
return plugin
"#,
    );
    let f = p.factory("");
    let mut inst = f.instantiate().unwrap();
    let mut file = PackFile::new("bad.txt", b"x".to_vec());
    let err = inst.process("boom", &mut file).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("boom"), "{msg}");
    assert!(msg.contains("bad.txt"), "{msg}");
    assert!(msg.contains("kaboom"), "{msg}");
}

#[test]
fn processor_defs_extracted_with_priority() {
    let p = PluginDir::lua(
        "defs",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("a", { files = { "**/*.json" }, priority = 10 }, function() end)
plugin:processor("b", { files = { "**/*.png" } }, function() end)
return plugin
"#,
    );
    let f = p.factory("");
    let defs = f.processors();
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[0].name, "a");
    assert_eq!(defs[0].priority, 10);
    assert_eq!(defs[1].name, "b");
    assert_eq!(defs[1].priority, 0);
}

#[test]
fn bytes_and_text_are_aliases() {
    let p = PluginDir::lua(
        "alias",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("a", { files = { "**/*" } }, function(ctx, file)
    assert(file.bytes == file.text)
    file.bytes = "via-bytes"
    assert(file.text == "via-bytes")
end)
return plugin
"#,
    );
    let f = p.factory("");
    let (file, outcome) = process(&f, "a", "x", "start");
    assert_eq!(outcome, ProcessOutcome::Modified);
    assert_eq!(file.contents, b"via-bytes");
}
