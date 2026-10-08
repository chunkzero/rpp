//! Host calls exposed to TypeScript plugins: toml, hash, path, process, and randomness.

#![cfg(feature = "js")]

mod common;

use common::js::{load, plugin, process, text, try_load_with, write_plugin, Recorder};
use rpp::config::SecurityMode;
use rpp::model::PluginFactory;
use rpp::Error;

#[test]
fn toml_and_hash_helpers() {
    let dir = write_plugin(
        r##"
import { definePlugin, hash, path, toml } from "rpp";
export default definePlugin({
  processors: {
    info: {
      files: "**/*",
      run(ctx, file) {
        file.text = JSON.stringify({
          parsed: toml.parse(file.text),
          encoded: toml.stringify({ a: 1, b: { c: "d" } }),
          sha: hash.sha256("abc"),
          crc: hash.crc32("abc"),
          joined: path.join("a/./b", "../c", "/d.txt"),
          ext: path.ext("x/y.tar.gz"),
          matches: path.match("**/*.json", "a/b.json"),
        });
      },
    },
  },
});
"##,
    );
    let mut instance = load(dir.path(), "{}").instantiate().unwrap();
    let (file, _) = process(instance.as_mut(), "info", "a.toml", "k = [1, 2]\n");
    let value: serde_json::Value = serde_json::from_str(text(&file)).unwrap();
    assert_eq!(value["parsed"], serde_json::json!({ "k": [1, 2] }));
    assert_eq!(value["encoded"], "a = 1\n\n[b]\nc = \"d\"\n");
    assert_eq!(
        value["sha"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(value["crc"], 891568578);
    assert_eq!(value["joined"], "a/c/d.txt");
    assert_eq!(value["ext"], "gz");
    assert_eq!(value["matches"], true);
}

#[test]
fn process_requires_trusted_permission() {
    let dir = write_plugin(
        r##"
import { definePlugin, process } from "rpp";
export default definePlugin({
  generate() { process.run({ program: "true" }); },
});
"##,
    );
    let mut instance = load(dir.path(), "{}").instantiate().unwrap();
    let error = instance.generate(&mut Recorder::default()).unwrap_err();
    assert!(matches!(error, Error::Generator { .. }), "{error}");
    assert!(
        error
            .to_string()
            .contains("requires trusted process permissions"),
        "{error}"
    );
}

#[test]
fn process_cwd_must_stay_inside_project() {
    let dir = write_plugin(
        r##"
import { definePlugin, process } from "rpp";
export default definePlugin({
  generate() { process.run({ program: "true", cwd: "../outside" }); },
});
"##,
    );
    let mut plugin = plugin("{}");
    plugin.security = SecurityMode::Trusted;
    plugin.permissions.process = vec!["true".into()];
    let mut instance = try_load_with(dir.path(), &plugin)
        .unwrap()
        .instantiate()
        .unwrap();
    let error = instance.generate(&mut Recorder::default()).unwrap_err();
    assert!(error.to_string().contains("must not contain"), "{error}");
}

#[test]
fn deterministic_random_per_file() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "rpp";
export default definePlugin({
  processors: { rand: { files: "**/*", run(ctx, file) { file.text = String(Math.random()); } } },
});
"##,
    );
    let factory = load(dir.path(), "{}");
    let run = |path: &str| {
        let mut instance = factory.instantiate().unwrap();
        text(&process(instance.as_mut(), "rand", path, "").0).to_string()
    };
    assert_eq!(run("a.txt"), run("a.txt"));
    assert_ne!(run("a.txt"), run("b.txt"));
}

#[test]
fn module_level_randomness_validates_and_absent_description_is_empty() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "rpp";
export default definePlugin({
  processors: {
    [String(Math.random())]: {
      files: "**/*",
      run(ctx, file) { file.text = String(ctx.pack.description === ""); },
    },
  },
});
"##,
    );
    let factory = load(dir.path(), "{}");
    let name = factory.processors()[0].name.clone();
    let mut instance = factory.instantiate().unwrap();
    let (file, _) = process(instance.as_mut(), &name, "a.txt", "");
    assert_eq!(text(&file), "true");
}

#[test]
fn clocks_alone_keep_randomness_fixed() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "rpp";
export default definePlugin({
  processors: { rand: { files: "**/*", run(ctx, file) { file.text = String(Math.random()); } } },
});
"##,
    );
    let mut plugin = plugin("{}");
    plugin.security = SecurityMode::Trusted;
    plugin.permissions.clocks = true;
    let factory = try_load_with(dir.path(), &plugin).unwrap();
    let run = || {
        let mut instance = factory.instantiate().unwrap();
        text(&process(instance.as_mut(), "rand", "a.txt", "").0).to_string()
    };
    assert_eq!(run(), run());
}
