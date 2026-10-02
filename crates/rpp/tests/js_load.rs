//! Loading TypeScript plugins: option and permission validation, entry checks and cache keys.

#![cfg(feature = "js")]

mod common;

use common::js::{load, plugin, process, text, try_load, try_load_with, write_file, write_plugin};
use rpp::model::PluginFactory;
use rpp::Error;
use tempfile::TempDir;

#[test]
fn options_reach_plugins_in_canonical_key_order() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  processors: {
    dump: { files: "**/*", run(ctx, file) { file.text = JSON.stringify(ctx.options); } },
  },
});
"##,
    );
    let first = load(dir.path(), r#"{"z":1,"a":{"y":1,"x":2}}"#);
    let second = load(dir.path(), r#"{"a":{"x":2,"y":1},"z":1}"#);
    assert_eq!(first.processor_key(), second.processor_key());
    for factory in [first, second] {
        let mut instance = factory.instantiate().unwrap();
        let (file, _) = process(instance.as_mut(), "dump", "a.txt", "");
        assert_eq!(text(&file), r#"{"a":{"x":2,"y":1},"z":1}"#);
    }
}

#[test]
fn load_rejects_permissions_on_sandboxed_plugin() {
    let dir = write_plugin("export default {};\n");
    let mut entry = plugin("{}");
    entry.permissions.process = vec!["git".into()];
    let error = try_load_with(dir.path(), &entry).err().unwrap();
    assert!(matches!(error, Error::Config { .. }), "{error}");
    assert!(
        error.to_string().contains("uses `security: \"sandboxed\"`"),
        "{error}"
    );
}

#[test]
fn load_rejects_null_options() {
    let dir = write_plugin("export default {};\n");
    for (options, path) in [
        ("null", "`options`"),
        (r#"{"nested":[null]}"#, "`options.nested[0]`"),
    ] {
        let error = try_load(dir.path(), options).err().unwrap();
        assert!(matches!(error, Error::Config { .. }), "{error}");
        assert!(
            error
                .to_string()
                .contains(&format!("{path} must not be null")),
            "{error}"
        );
    }
}

#[test]
fn missing_default_export_fails_to_load() {
    let dir = write_plugin("export const plugin = {};\n");
    assert!(matches!(
        try_load(dir.path(), "{}"),
        Err(Error::PluginLoad { .. })
    ));
    let dir = write_plugin("export default 42;\n");
    let error = try_load(dir.path(), "{}").err().unwrap();
    assert!(matches!(error, Error::PluginLoad { .. }), "{error}");
}

fn helper_plugin(constant: &str) -> TempDir {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
import { value } from "./helper";
export default definePlugin({
  processors: { set: { files: "**/*", run(ctx, file) { file.text = value; } } },
});
"##,
    );
    write_file(
        dir.path(),
        "src/helper.ts",
        &format!("export const value = \"{constant}\";"),
    );
    dir
}

#[test]
fn helper_edit_changes_cache_key() {
    let dir = helper_plugin("one");
    let before = load(dir.path(), "{}");
    let (processor, generator) = (before.processor_key(), before.cache_key());
    assert_eq!(load(dir.path(), "{}").cache_key(), generator);
    write_file(dir.path(), "src/helper.ts", "export const value = \"two\";");
    let after = load(dir.path(), "{}");
    assert_ne!(after.processor_key(), processor);
    assert_ne!(after.cache_key(), generator);
}

#[test]
fn options_change_cache_key() {
    let dir = helper_plugin("one");
    assert_ne!(
        load(dir.path(), r#"{"a":1}"#).cache_key(),
        load(dir.path(), r#"{"a":2}"#).cache_key()
    );
}
