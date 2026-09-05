//! End-to-end build engine tests: chain ordering, generators, incremental cache.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{PluginDir, Project};
use rpp::engine::Engine;
use rpp::lua::{LuaPluginFactory, LuaPluginLimits, PackInfo, RuntimeAccess};
use rpp::model::PluginFactory;

fn build(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> rpp::engine::BuildResult {
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugins(plugins)
        .build_engine()
        .unwrap();
    engine.build().unwrap()
}

fn external_factory(plugin: &PluginDir, root: &std::path::Path) -> Arc<dyn PluginFactory> {
    let access = RuntimeAccess::sandboxed(root.to_path_buf())
        .with_outputs(BTreeMap::from([("code".to_string(), "generated".into())]));
    Arc::new(
        LuaPluginFactory::load(
            plugin.path(),
            toml::Value::Table(Default::default()),
            PackInfo {
                name: "test-pack".into(),
                description: None,
                format: Some(34),
            },
            LuaPluginLimits::default(),
            access,
        )
        .unwrap(),
    )
}

#[test]
fn basic_processor_writes_output() {
    let project = Project::new();
    project.write_src("a.txt", "hello");

    let upper = PluginDir::lua(
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

    let result = build(&project, vec![upper.factory_arc("")]);
    assert_eq!(result.processed, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("HELLO"));
}

#[test]
fn chain_ordering_across_plugins() {
    // Two plugins both append a marker; priority + plugin order decide sequence.
    let project = Project::new();
    project.write_src("x.txt", "");

    let make = |id: &str, mark: &str, prio: i32| {
        PluginDir::lua(
            id,
            &format!(
                r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", {{ files = {{ "**/*" }}, priority = {prio} }}, function(ctx, file)
    file.text = file.text .. "{mark}"
end)
return plugin
"#
            ),
        )
    };

    // late has higher priority number => runs after early.
    let early = make("early", "A", 10);
    let late = make("late", "B", 20);

    // Register late first in config order to prove priority wins over order.
    let result = build(&project, vec![late.factory_arc(""), early.factory_arc("")]);
    assert_eq!(result.processed, 1);
    assert_eq!(project.read_out("x.txt").as_deref(), Some("AB"));
}

#[test]
fn chain_tie_broken_by_plugin_order() {
    let project = Project::new();
    project.write_src("x.txt", "");

    let make = |id: &str, mark: &str| {
        PluginDir::lua(
            id,
            &format!(
                r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", {{ files = {{ "**/*" }} }}, function(ctx, file)
    file.text = file.text .. "{mark}"
end)
return plugin
"#
            ),
        )
    };

    let first = make("first", "1");
    let second = make("second", "2");
    // Same priority (0); config order decides.
    let result = build(
        &project,
        vec![first.factory_arc(""), second.factory_arc("")],
    );
    assert_eq!(result.processed, 1);
    assert_eq!(project.read_out("x.txt").as_deref(), Some("12"));
}

#[test]
fn dropped_file_excluded_from_output() {
    let project = Project::new();
    project.write_src("keep.txt", "k");
    project.write_src("drop.txt", "d");

    let dropper = PluginDir::lua(
        "dropper",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("d", { files = { "drop.txt" } }, function(ctx, file)
    file:drop()
end)
return plugin
"#,
    );

    let result = build(&project, vec![dropper.factory_arc("")]);
    assert_eq!(result.dropped, 1);
    assert!(project.out_exists("keep.txt"));
    assert!(!project.out_exists("drop.txt"));
}

#[test]
fn generator_emit_and_read() {
    let project = Project::new();
    project.write_src("assets/a.txt", "one");
    project.write_src("assets/b.txt", "two");

    let gen = PluginDir::lua(
        "indexer",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("index", function(ctx)
    local names = {}
    for _, path in ipairs(ctx:files("assets/*.txt")) do
        local body = ctx:read(path)
        names[#names + 1] = path .. "=" .. body
    end
    table.sort(names)
    ctx:emit("index.txt", table.concat(names, "\n"))
end)
return plugin
"#,
    );

    let result = build(&project, vec![gen.factory_arc("")]);
    assert_eq!(result.generated, 1);
    assert_eq!(
        project.read_out("index.txt").as_deref(),
        Some("assets/a.txt=one\nassets/b.txt=two")
    );
}

#[test]
fn generator_read_source() {
    let project = Project::new();
    project.write_src("raw.txt", "RAWBODY");

    // A processor mutates the processed copy, but read_source sees the original.
    let plugin = PluginDir::lua(
        "src-reader",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("up", { files = { "raw.txt" } }, function(ctx, file)
    file.text = "PROCESSED"
end)
plugin:generator("g", function(ctx)
    local raw = ctx:read_source("raw.txt")
    local proc = ctx:read("raw.txt")
    ctx:emit("report.txt", raw .. "|" .. proc)
end)
return plugin
"#,
    );

    build(&project, vec![plugin.factory_arc("")]);
    assert_eq!(
        project.read_out("report.txt").as_deref(),
        Some("RAWBODY|PROCESSED")
    );
}

#[test]
fn incremental_second_build_all_cached() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("b.txt", "b");

    let upper = PluginDir::lua(
        "u",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("u", { files = { "**/*.txt" } }, function(ctx, file)
    file.text = string.upper(file.text)
end)
return plugin
"#,
    );

    let first = build(&project, vec![upper.factory_arc("")]);
    assert_eq!(first.processed, 2);
    assert_eq!(first.cached, 0);

    let second = build(&project, vec![upper.factory_arc("")]);
    assert_eq!(second.processed, 0);
    assert_eq!(second.cached, 2);
    // Outputs still correct.
    assert_eq!(project.read_out("a.txt").as_deref(), Some("A"));
}

#[test]
fn incremental_touch_one_file_reprocesses_only_it() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("b.txt", "b");

    let upper = PluginDir::lua(
        "u",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("u", { files = { "**/*.txt" } }, function(ctx, file)
    file.text = string.upper(file.text)
end)
return plugin
"#,
    );

    build(&project, vec![upper.factory_arc("")]);

    // Change one file's content.
    project.write_src("a.txt", "changed");

    let second = build(&project, vec![upper.factory_arc("")]);
    assert_eq!(second.processed, 1);
    assert_eq!(second.cached, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("CHANGED"));
}

#[test]
fn changing_options_reprocesses() {
    let project = Project::new();
    project.write_src("a.txt", "x");

    let plugin = PluginDir::lua(
        "opt",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", { files = { "**/*" } }, function(ctx, file)
    file.text = ctx.options.tag .. file.text
end)
return plugin
"#,
    );

    let first = build(&project, vec![plugin.factory_arc("tag = \"A\"")]);
    assert_eq!(first.processed, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("Ax"));

    // Different options => different cache_key => reprocess.
    let second = build(&project, vec![plugin.factory_arc("tag = \"B\"")]);
    assert_eq!(second.processed, 1);
    assert_eq!(second.cached, 0);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("Bx"));
}

#[test]
fn changing_plugin_source_reprocesses() {
    let project = Project::new();
    project.write_src("a.txt", "x");

    let v1 = PluginDir::lua(
        "p",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", { files = { "**/*" } }, function(ctx, file)
    file.text = "v1:" .. file.text
end)
return plugin
"#,
    );
    let first = build(&project, vec![v1.factory_arc("")]);
    assert_eq!(first.processed, 1);

    let v2 = PluginDir::lua(
        "p",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", { files = { "**/*" } }, function(ctx, file)
    file.text = "v2:" .. file.text
end)
return plugin
"#,
    );
    let second = build(&project, vec![v2.factory_arc("")]);
    assert_eq!(second.processed, 1);
    assert_eq!(second.cached, 0);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("v2:x"));
}

#[test]
fn output_sync_removes_stale_files() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("b.txt", "b");

    let passthrough = PluginDir::lua(
        "noop",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("n", { files = { "**/*" } }, function(ctx, file) end)
return plugin
"#,
    );

    build(&project, vec![passthrough.factory_arc("")]);
    assert!(project.out_exists("a.txt"));
    assert!(project.out_exists("b.txt"));

    // Remove a source file; rebuild should delete its output.
    std::fs::remove_file(project.src().join("b.txt")).unwrap();
    let report = build(&project, vec![passthrough.factory_arc("")]);
    assert!(project.out_exists("a.txt"));
    assert!(!project.out_exists("b.txt"));
    assert!(report.changes.removed.contains(&"b.txt".to_string()));
}

#[test]
fn generator_incremental_reuses_when_inputs_unchanged() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let gen = PluginDir::lua(
        "g",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("g", function(ctx)
    local n = #ctx:files("**/*.txt")
    ctx:emit("count.txt", tostring(n))
end)
return plugin
"#,
    );

    let first = build(&project, vec![gen.factory_arc("")]);
    assert_eq!(first.generated, 1);
    assert_eq!(project.read_out("count.txt").as_deref(), Some("1"));

    // Second build with no changes: generator output reused from cache.
    let second = build(&project, vec![gen.factory_arc("")]);
    assert_eq!(project.read_out("count.txt").as_deref(), Some("1"));
    // No source change, no rewrite expected.
    assert!(second.changes.written.is_empty());
}

#[test]
fn generator_reruns_when_read_set_changes() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let gen = PluginDir::lua(
        "g",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("g", function(ctx)
    local n = #ctx:files("**/*.txt")
    ctx:emit("count.txt", tostring(n))
end)
return plugin
"#,
    );

    build(&project, vec![gen.factory_arc("")]);
    assert_eq!(project.read_out("count.txt").as_deref(), Some("1"));

    // Add a file: the generator's file-list read-set changes => rerun.
    project.write_src("b.txt", "b");
    build(&project, vec![gen.factory_arc("")]);
    assert_eq!(project.read_out("count.txt").as_deref(), Some("2"));
}

#[test]
fn lua_generator_discovers_and_loads_dropped_raw_sources() {
    let project = Project::new();
    project.write_src("window/z.lua", "z");
    project.write_src("window/a.lua", "a");

    let plugin = PluginDir::lua(
        "source-generator",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("drop-sources", { files = { "window/**" } }, function(ctx, file)
    file:drop()
end)
plugin:generator("sources", function(ctx)
    local documents = {}
    for _, path in ipairs(ctx:source_files("window/**")) do
        documents[#documents + 1] = path .. "=" .. ctx:read_source(path)
    end
    ctx:emit("documents.txt", table.concat(documents, "\n"))
end)
return plugin
"#,
    );

    let first = build(&project, vec![plugin.factory_arc("")]);
    assert_eq!(first.generated, 1);
    assert_eq!(
        project.read_out("documents.txt").as_deref(),
        Some("window/a.lua=a\nwindow/z.lua=z")
    );
    assert!(!project.out_exists("window/a.lua"));

    let unchanged = build(&project, vec![plugin.factory_arc("")]);
    assert_eq!(unchanged.generated, 0);

    project.write_src("window/a.lua", "changed");
    let changed = build(&project, vec![plugin.factory_arc("")]);
    assert_eq!(changed.generated, 1);
    assert_eq!(
        project.read_out("documents.txt").as_deref(),
        Some("window/a.lua=changed\nwindow/z.lua=z")
    );

    std::fs::remove_file(project.src().join("window/z.lua")).unwrap();
    let removed = build(&project, vec![plugin.factory_arc("")]);
    assert_eq!(removed.generated, 1);
    assert_eq!(
        project.read_out("documents.txt").as_deref(),
        Some("window/a.lua=changed")
    );
}

#[test]
fn clean_removes_output_and_cache() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let noop = PluginDir::lua(
        "n",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("n", { files = { "**/*" } }, function(ctx, file) end)
return plugin
"#,
    );

    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(noop.factory_arc(""))
        .build_engine()
        .unwrap();
    engine.build().unwrap();
    assert!(project.out_exists("a.txt"));

    engine.clean().unwrap();
    assert!(!project.root().join("dist").exists());
    assert!(!project.root().join(".rpp").exists());
}

#[test]
fn lifecycle_hooks_run() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    // on_start/on_finish just must not error; use log.
    let plugin = PluginDir::lua(
        "hooks",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:on_start(function(ctx) ctx.log.info("start") end)
plugin:on_finish(function(ctx, stats) ctx.log.info("finish " .. stats.processed) end)
plugin:processor("n", { files = { "**/*" } }, function(ctx, file) end)
return plugin
"#,
    );

    let result = build(&project, vec![plugin.factory_arc("")]);
    assert_eq!(result.processed, 1);
}

#[test]
fn failing_finish_hook_does_not_commit_outputs_or_manifest() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = PluginDir::lua(
        "finish-error",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("g", function(ctx)
    ctx:emit("generated.txt", "generated")
end)
plugin:on_finish(function()
    error("finish failed")
end)
return plugin
"#,
    );
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(plugin.factory_arc(""))
        .build_engine()
        .unwrap();
    let error = engine.build().unwrap_err().to_string();
    assert!(error.contains("finish failed"), "{error}");
    assert!(!project.root().join("dist/generated.txt").exists());
    assert!(!project.root().join(".rpp/cache/manifest.bin").exists());
}

#[test]
fn rppignore_excludes_files() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("ignored.tmp", "x");
    std::fs::write(project.src().join(".rppignore"), "*.tmp\n").unwrap();

    let noop = PluginDir::lua(
        "n",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("n", { files = { "**/*" } }, function(ctx, file) end)
return plugin
"#,
    );

    build(&project, vec![noop.factory_arc("")]);
    assert!(project.out_exists("a.txt"));
    assert!(!project.out_exists("ignored.tmp"));
}

#[test]
fn processor_cannot_escape_output_directory() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = PluginDir::lua(
        "escape",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", { files = { "**/*" } }, function(ctx, file)
    file.path = "../escaped.txt"
end)
return plugin
"#,
    );
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(plugin.factory_arc(""))
        .build_engine()
        .unwrap();
    assert!(engine.build().is_err());
    assert!(!project.root().join("escaped.txt").exists());
}

#[test]
fn generator_cannot_escape_source_or_output_directories() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    std::fs::write(project.root().join("secret.txt"), "secret").unwrap();
    let plugin = PluginDir::lua(
        "escape",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("g", function(ctx)
    ctx:emit("../escaped.txt", ctx:read_source("../secret.txt") or "missing")
end)
return plugin
"#,
    );
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(plugin.factory_arc(""))
        .build_engine()
        .unwrap();
    assert!(engine.build().is_err());
    assert!(!project.root().join("escaped.txt").exists());
}

#[test]
fn cached_generator_replays_removals() {
    let project = Project::new();
    project.write_src("remove.txt", "remove me");
    let plugin = PluginDir::lua(
        "remover",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("g", function(ctx)
    ctx:remove("remove.txt")
end)
return plugin
"#,
    );

    build(&project, vec![plugin.factory_arc("")]);
    assert!(!project.out_exists("remove.txt"));
    let second = build(&project, vec![plugin.factory_arc("")]);
    assert!(!project.out_exists("remove.txt"));
    assert!(second.changes.written.is_empty());
}

#[test]
fn colliding_processor_outputs_fail_deterministically() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("b.txt", "b");
    let plugin = PluginDir::lua(
        "rename",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:processor("p", { files = { "**/*" } }, function(ctx, file)
    file.path = "same.txt"
end)
return plugin
"#,
    );
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(plugin.factory_arc(""))
        .build_engine()
        .unwrap();
    let error = engine.build().unwrap_err().to_string();
    assert!(error.contains("both produce `same.txt`"), "{error}");
}

#[test]
fn generator_reads_use_the_pre_generator_snapshot() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = PluginDir::lua(
        "snapshot",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("g", function(ctx)
    ctx:emit("marker.txt", "marker")
    ctx:files()
end)
return plugin
"#,
    );

    build(&project, vec![plugin.factory_arc("")]);
    let second = build(&project, vec![plugin.factory_arc("")]);
    assert!(second.changes.written.is_empty());
}

#[test]
fn corrupt_cache_object_is_rebuilt() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    build(&project, Vec::new());

    let objects = project.root().join(".rpp/cache/objects");
    let object = std::fs::read_dir(&objects)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(object, "corrupt").unwrap();

    // The output directory still holds the right bytes, so this is a hit.
    let second = build(&project, Vec::new());
    assert_eq!(second.cached, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("a"));

    // Once the output is gone the corrupt object cannot be materialized and
    // the file is reprocessed.
    std::fs::remove_dir_all(project.root().join("dist")).unwrap();
    let third = build(&project, Vec::new());
    assert_eq!(third.processed, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("a"));
}

#[test]
fn generator_reads_linked_output_when_cache_object_is_corrupt() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("trigger.txt", "first");
    let plugin = PluginDir::lua(
        "reader",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("reader", function(ctx)
    ctx:emit("report.txt", ctx:read("a.txt") .. ":" .. ctx:read_source("trigger.txt"))
end)
return plugin
"#,
    );

    build(&project, vec![plugin.factory_arc("")]);
    for entry in std::fs::read_dir(project.root().join(".rpp/cache/objects")).unwrap() {
        std::fs::write(entry.unwrap().path(), "corrupt").unwrap();
    }
    project.write_src("trigger.txt", "second");

    build(&project, vec![plugin.factory_arc("")]);
    assert_eq!(project.read_out("report.txt").as_deref(), Some("a:second"));
}

#[test]
fn same_size_source_change_with_preserved_mtime_is_rebuilt() {
    let project = Project::new();
    project.write_src("a.txt", "aa");
    build(&project, Vec::new());

    let source = project.src().join("a.txt");
    let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
    std::fs::write(&source, "bb").unwrap();
    std::fs::File::open(&source)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();

    let second = build(&project, Vec::new());
    assert_eq!(second.processed, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("bb"));
}

#[test]
fn materialized_outputs_do_not_share_writable_cache_inodes() {
    let project = Project::new();
    project.write_src("a.txt", "same");
    project.write_src("b.txt", "same");
    build(&project, Vec::new());

    std::fs::remove_dir_all(project.root().join("dist")).unwrap();
    let warm = build(&project, Vec::new());
    assert_eq!(warm.cached, 2);

    std::fs::write(project.root().join("dist/a.txt"), "changed").unwrap();
    assert_eq!(project.read_out("b.txt").as_deref(), Some("same"));
}

#[test]
fn external_outputs_have_durable_stale_ownership_and_clean_support() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = PluginDir::lua(
        "codegen",
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("codegen", function(ctx)
    ctx:emit_output("code", "Keep.kt", "keep-v1")
    ctx:emit_output("code", "Stale.kt", "stale")
end)
return plugin
"#,
    );

    let first = build(&project, vec![external_factory(&plugin, project.root())]);
    assert_eq!(first.changes.external.written.len(), 2);
    std::fs::write(project.root().join("generated/Manual.kt"), "manual").unwrap();

    std::fs::write(
        plugin.path().join("init.lua"),
        r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("codegen", function(ctx)
    ctx:emit_output("code", "Keep.kt", "keep-v2")
end)
return plugin
"#,
    )
    .unwrap();
    // Simulate `rpp build --no-cache`: ownership deliberately lives outside
    // this directory and must still remove Stale.kt.
    std::fs::remove_dir_all(project.root().join(".rpp/cache")).unwrap();
    let second = build(&project, vec![external_factory(&plugin, project.root())]);
    assert_eq!(
        std::fs::read_to_string(project.root().join("generated/Keep.kt")).unwrap(),
        "keep-v2"
    );
    assert!(!project.root().join("generated/Stale.kt").exists());
    assert!(project.root().join("generated/Manual.kt").exists());
    assert_eq!(second.changes.external.removed.len(), 1);

    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(external_factory(&plugin, project.root()))
        .build_engine()
        .unwrap();
    engine.clean().unwrap();
    assert!(!project.root().join("generated/Keep.kt").exists());
    assert!(project.root().join("generated/Manual.kt").exists());
}

#[test]
fn colliding_external_outputs_fail_with_plugin_attribution() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    build(&project, Vec::new());
    project.write_src("a.txt", "changed");
    let make = |id: &str| {
        PluginDir::lua(
            id,
            r#"
local rpp = require("rpp")
local plugin = rpp.plugin()
plugin:generator("g", function(ctx)
    ctx:emit_output("code", "Same.kt", "generated")
end)
return plugin
"#,
        )
    };
    let first = make("first");
    let second = make("second");
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugins([
            external_factory(&first, project.root()),
            external_factory(&second, project.root()),
        ])
        .build_engine()
        .unwrap();
    let error = engine.build().unwrap_err().to_string();
    assert!(error.contains("`first` and `second`"), "{error}");
    assert!(error.contains("Same.kt"), "{error}");
    assert_eq!(project.read_out("a.txt").as_deref(), Some("a"));
    assert!(!project.root().join("generated/Same.kt").exists());
}

#[cfg(unix)]
#[test]
fn cached_symlink_output_is_replaced_even_without_cas() {
    for remove_cas in [false, true] {
        let project = Project::new();
        project.write_src("a.txt", "same");
        build(&project, Vec::new());
        let output = project.root().join("dist/a.txt");
        std::fs::remove_file(&output).unwrap();
        std::os::unix::fs::symlink(project.src().join("a.txt"), &output).unwrap();
        if remove_cas {
            std::fs::remove_dir_all(project.root().join(".rpp/cache/objects")).unwrap();
        }
        build(&project, Vec::new());
        assert_eq!(project.read_out("a.txt").as_deref(), Some("same"));
        assert!(!std::fs::symlink_metadata(output)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn destination_boundaries_reject_ancestor_symlinks_before_build_and_clean() {
    use std::os::unix::fs::symlink;

    let project = Project::new();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(outside.path().join("victim")).unwrap();
    let keep = outside.path().join("victim/keep.txt");
    std::fs::write(&keep, "keep").unwrap();
    let mut config = project.config();
    config.build.output = "alias/victim".into();
    let engine = Engine::builder(config.clone())
        .project_root(project.root())
        .build_engine()
        .unwrap();
    symlink(outside.path(), project.root().join("alias")).unwrap();
    assert!(engine.build().is_err());
    assert!(rpp::engine::clean_project_artifacts(&config, project.root()).is_err());
    assert_eq!(std::fs::read_to_string(keep).unwrap(), "keep");
    assert!(!project.root().join(".rpp").exists());
}

#[cfg(unix)]
#[test]
fn destination_boundaries_reject_external_aliases_and_protected_roots() {
    use std::os::unix::fs::symlink;

    let project = Project::new();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), "keep").unwrap();
    symlink(outside.path(), project.root().join("alias")).unwrap();
    symlink(project.src(), project.root().join("source-alias")).unwrap();
    for root in [
        "alias",
        "source-alias",
        "src/generated",
        "dist/generated",
        ".rpp/generated",
    ] {
        let config = rpp::config::Config::parse(
            &format!("[pack]\nname = 'test'\n[[plugin]]\nsource = 'path:plugin'\n[plugin.outputs]\ncode = '{root}'\n"),
            "rpp.toml",
        ).unwrap();
        assert!(
            Engine::builder(config.clone())
                .project_root(project.root())
                .build_engine()
                .is_err(),
            "{root}"
        );
        assert!(
            rpp::engine::clean_project_artifacts(&config, project.root()).is_err(),
            "{root}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn destination_boundaries_allow_sibling_external_outputs_and_clean() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    let plugin = PluginDir::lua(
        "sibling",
        r#"
        local rpp = require('rpp')
        local plugin = rpp.plugin()
        plugin:generator('code', function(ctx) ctx:emit_output('code', 'nested/generated.txt', 'generated') end)
        return plugin
    "#,
    );
    let factory = LuaPluginFactory::load(
        plugin.path(),
        toml::Value::Table(Default::default()),
        PackInfo {
            name: "test".into(),
            description: None,
            format: None,
        },
        LuaPluginLimits::default(),
        RuntimeAccess::sandboxed(project.clone())
            .with_outputs(BTreeMap::from([("code".into(), "../sibling".into())])),
    )
    .unwrap();
    let config = Project::new().config();
    let engine = Engine::builder(config)
        .project_root(&project)
        .plugin(Arc::new(factory))
        .build_engine()
        .unwrap();
    engine.build().unwrap();
    let generated = parent.path().join("sibling/nested/generated.txt");
    assert_eq!(std::fs::read_to_string(&generated).unwrap(), "generated");
    std::fs::write(parent.path().join("sibling/keep.txt"), "keep").unwrap();
    engine.clean().unwrap();
    assert!(!generated.exists());
    assert_eq!(
        std::fs::read_to_string(parent.path().join("sibling/keep.txt")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn destination_boundaries_reject_retargeted_owned_external_parent() {
    let project = Project::new();
    let plugin = PluginDir::lua(
        "external",
        r#"
        local rpp = require('rpp')
        local plugin = rpp.plugin()
        plugin:generator('code', function(ctx) ctx:emit_output('code', 'nested/keep.txt', 'generated') end)
        return plugin
    "#,
    );
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(external_factory(&plugin, project.root()))
        .build_engine()
        .unwrap();
    engine.build().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), "keep").unwrap();
    std::fs::remove_dir_all(project.root().join("generated/nested")).unwrap();
    std::os::unix::fs::symlink(outside.path(), project.root().join("generated/nested")).unwrap();
    assert!(engine.build().is_err());
    assert!(engine.clean().is_err());
    assert_eq!(
        std::fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn destination_boundaries_allow_symlinked_source_and_sibling_parents() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(parent.path().join("real-src")).unwrap();
    std::fs::write(parent.path().join("real-src/a.txt"), "a").unwrap();
    symlink(parent.path().join("real-src"), project.join("src")).unwrap();
    std::fs::create_dir_all(parent.path().join("real-server")).unwrap();
    symlink(
        parent.path().join("real-server"),
        parent.path().join("server"),
    )
    .unwrap();

    let plugin = PluginDir::lua(
        "sibling",
        r#"
        local rpp = require('rpp')
        local plugin = rpp.plugin()
        plugin:generator('code', function(ctx) ctx:emit_output('code', 'generated.txt', 'generated') end)
        return plugin
    "#,
    );
    let factory = |root: &str| {
        Arc::new(
            LuaPluginFactory::load(
                plugin.path(),
                toml::Value::Table(Default::default()),
                PackInfo {
                    name: "test".into(),
                    description: None,
                    format: None,
                },
                LuaPluginLimits::default(),
                RuntimeAccess::sandboxed(project.clone())
                    .with_outputs(BTreeMap::from([("code".into(), root.into())])),
            )
            .unwrap(),
        )
    };
    let engine = Engine::builder(Project::new().config())
        .project_root(&project)
        .plugin(factory("../server/generated"))
        .build_engine()
        .unwrap();
    engine.build().unwrap();
    assert_eq!(
        std::fs::read_to_string(parent.path().join("real-server/generated/generated.txt")).unwrap(),
        "generated"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("dist/a.txt")).unwrap(),
        "a"
    );

    // A symlinked parent that resolves into the pack output is still rejected.
    symlink(project.join("dist"), parent.path().join("alias")).unwrap();
    assert!(Engine::builder(Project::new().config())
        .project_root(&project)
        .plugin(factory("../alias/generated"))
        .build_engine()
        .is_err());
}
