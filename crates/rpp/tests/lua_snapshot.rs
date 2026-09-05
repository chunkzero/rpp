#![cfg(feature = "lua")]

mod common;

use std::sync::Arc;

use rpp::engine::Engine;
use rpp::lua::LuaPluginFactory;
use rpp::model::PluginFactory;

use common::{PluginDir, Project};

const ENTRY: &str = r#"
local plugin = require('rpp').plugin()
local helper = require('helper')
plugin:processor('value', {files = {'**/*'}}, function(ctx, file)
    file.text = helper .. require('late')
end)
return plugin
"#;

fn engine(project: &Project, factory: LuaPluginFactory) -> Engine {
    let mut config = project.config();
    config.build.workers = 1;
    Engine::builder(config)
        .project_root(project.root())
        .plugin(Arc::new(factory))
        .build_engine()
        .unwrap()
}

fn assert_reloaded_output(plugin: &PluginDir, project: &Project, expected: &str) {
    let warm = engine(project, plugin.factory(""));
    assert_eq!(warm.build().unwrap().processed, 1);
    assert_eq!(project.read_out("input.txt").as_deref(), Some(expected));
    assert_eq!(warm.build().unwrap().cached, 1);
    let cached_output = project.read_out("input.txt");
    warm.clean().unwrap();
    let fresh = engine(project, plugin.factory(""));
    assert_eq!(fresh.build().unwrap().processed, 1);
    assert_eq!(project.read_out("input.txt"), cached_output);
}

#[test]
fn helper_edits_do_not_change_existing_factory_execution() {
    let plugin = PluginDir::lua("snapshot", ENTRY)
        .with_module("helper.lua", "return 'v1'")
        .with_module("late/init.lua", "return '-old'");
    let factory = plugin.factory("");
    let original_key = factory.cache_key();
    let project = Project::new();
    project.write_src("input.txt", "input");
    std::fs::write(plugin.path().join("helper.lua"), "return 'v2'").unwrap();
    std::fs::write(plugin.path().join("late/init.lua"), "return '-new'").unwrap();

    let frozen = engine(&project, factory.clone());
    frozen.build().unwrap();
    assert_eq!(project.read_out("input.txt").as_deref(), Some("v1-old"));
    assert_eq!(frozen.build().unwrap().cached, 1);
    frozen.clean().unwrap();
    engine(&project, factory).build().unwrap();
    assert_eq!(project.read_out("input.txt").as_deref(), Some("v1-old"));
    assert_ne!(original_key, plugin.factory("").cache_key());
    assert_reloaded_output(&plugin, &project, "v2-new");
}

#[test]
fn nonstandard_entry_extension_invalidates_replay() {
    let plugin = PluginDir::lua("snapshot", ENTRY)
        .with_module("helper.lua", "return 'v1'")
        .with_module("late.lua", "return ''");
    std::fs::rename(
        plugin.path().join("init.lua"),
        plugin.path().join("entry.txt"),
    )
    .unwrap();
    std::fs::write(
        plugin.path().join("plugin.toml"),
        "[plugin]\nid='snapshot'\nversion='1.0.0'\nentry='entry.txt'\n",
    )
    .unwrap();
    let project = Project::new();
    project.write_src("input.txt", "input");
    let first = engine(&project, plugin.factory(""));
    first.build().unwrap();
    assert_eq!(first.build().unwrap().cached, 1);
    std::fs::write(
        plugin.path().join("entry.txt"),
        ENTRY.replace("helper ..", "'changed-' .. helper .."),
    )
    .unwrap();
    assert_reloaded_output(&plugin, &project, "changed-v1");
}

#[test]
#[cfg(unix)]
fn confined_module_symlink_target_invalidates_replay() {
    let plugin = PluginDir::lua("snapshot", ENTRY)
        .with_module("data.txt", "return 'v1'")
        .with_module("late.lua", "return ''");
    std::os::unix::fs::symlink("data.txt", plugin.path().join("helper.lua")).unwrap();
    let project = Project::new();
    project.write_src("input.txt", "input");
    let first = engine(&project, plugin.factory(""));
    first.build().unwrap();
    assert_eq!(first.build().unwrap().cached, 1);
    std::fs::write(plugin.path().join("data.txt"), "return 'v2'").unwrap();
    assert_reloaded_output(&plugin, &project, "v2");
}

#[test]
#[cfg(unix)]
fn snapshot_rejects_directory_and_escaping_symlinks() {
    let plugin = PluginDir::lua("snapshot", "return require('rpp').plugin()")
        .with_module("subdir/init.lua", "return true");
    std::os::unix::fs::symlink("subdir", plugin.path().join("alias")).unwrap();
    let error = common::load_plugin(plugin.path(), toml::from_str("").unwrap())
        .err()
        .unwrap();
    assert!(error.to_string().contains("directory symlinks"), "{error}");
    std::fs::remove_file(plugin.path().join("alias")).unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), plugin.path().join("helper.lua")).unwrap();
    let error = common::load_plugin(plugin.path(), toml::from_str("").unwrap())
        .err()
        .unwrap();
    assert!(error.to_string().contains("escapes"), "{error}");
}
