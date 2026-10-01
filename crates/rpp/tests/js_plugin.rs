//! TypeScript plugin semantics: processors, generators, hooks, errors and cache keys.

#![cfg(feature = "js")]

use std::path::Path;

use rpp::host::{PackInfo, RuntimeAccess};
use rpp::js::{JsPluginFactory, JsPluginLimits};
use rpp::model::{
    BuildStats, GeneratorHost, PackFile, PluginFactory, PluginInstance, ProcessOutcome,
};
use rpp::Error;
use tempfile::TempDir;

fn write_plugin(source: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("plugin.toml"),
        "[plugin]\nid = \"ts-test\"\nversion = \"1.0.0\"\nentry = \"src/plugin.ts\"\n",
    )
    .unwrap();
    write_file(dir.path(), "src/plugin.ts", source);
    dir
}

fn write_file(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn pack() -> PackInfo {
    PackInfo {
        name: "test-pack".into(),
        description: None,
        format: Some(34),
    }
}

fn try_load_with(dir: &Path, options: &str, access: RuntimeAccess) -> rpp::Result<JsPluginFactory> {
    JsPluginFactory::load(
        dir,
        toml::from_str(options).unwrap(),
        pack(),
        JsPluginLimits::default(),
        access,
    )
}

fn try_load(dir: &Path, options: &str) -> rpp::Result<JsPluginFactory> {
    try_load_with(dir, options, RuntimeAccess::sandboxed(".".into()))
}

fn load(dir: &Path, options: &str) -> JsPluginFactory {
    try_load(dir, options).unwrap()
}

fn process(
    instance: &mut dyn PluginInstance,
    processor: &str,
    path: &str,
    body: &str,
) -> (PackFile, ProcessOutcome) {
    let mut file = PackFile::new(path, body.as_bytes().to_vec());
    let outcome = instance.process(processor, &mut file).unwrap();
    (file, outcome)
}

fn text(file: &PackFile) -> &str {
    std::str::from_utf8(&file.contents).unwrap()
}

#[derive(Default)]
struct Recorder {
    outputs: Vec<(String, Vec<u8>)>,
    sources: Vec<(String, Vec<u8>)>,
    emitted: Vec<(String, Vec<u8>)>,
    removed: Vec<String>,
    root_outputs: Vec<(String, String, Vec<u8>)>,
}

impl GeneratorHost for Recorder {
    fn list_files(&mut self, glob: Option<&str>) -> Vec<String> {
        let all = self.outputs.iter().map(|(path, _)| path.clone());
        match glob {
            Some(glob) => all
                .filter(|path| glob::Pattern::new(glob).unwrap().matches(path))
                .collect(),
            None => all.collect(),
        }
    }

    fn list_source_files(&mut self, _glob: Option<&str>) -> Vec<String> {
        self.sources.iter().map(|(path, _)| path.clone()).collect()
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let found = self.outputs.iter().find(|(p, _)| p == path);
        found.map(|(_, bytes)| bytes.clone())
    }

    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        let found = self.sources.iter().find(|(p, _)| p == path);
        found.map(|(_, bytes)| bytes.clone())
    }

    fn emit(&mut self, path: &str, contents: Vec<u8>) {
        self.emitted.push((path.into(), contents));
    }

    fn remove(&mut self, path: &str) {
        self.removed.push(path.into());
    }

    fn emit_output(&mut self, root: &str, path: &str, contents: Vec<u8>) {
        self.root_outputs.push((root.into(), path.into(), contents));
    }
}

#[test]
fn processor_mutates_text() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  processors: {
    up: { files: "**/*.txt", run(ctx, file) { file.text = file.text.toUpperCase(); } },
    noop: { files: ["**/*"], run() {} },
  },
});
"##,
    );
    let factory = load(dir.path(), "");
    assert_eq!(factory.processors().len(), 2);
    let mut instance = factory.instantiate().unwrap();
    let (file, outcome) = process(instance.as_mut(), "up", "a/b.txt", "hello");
    assert_eq!(outcome, ProcessOutcome::Modified);
    assert_eq!(text(&file), "HELLO");
    let (file, outcome) = process(instance.as_mut(), "noop", "a/b.txt", "same");
    assert_eq!(outcome, ProcessOutcome::Unchanged);
    assert_eq!(text(&file), "same");
}

#[test]
fn processor_renames_and_drops() {
    let dir = write_plugin(
        r##"
import { definePlugin, path } from "#rpp";
export default definePlugin({
  processors: {
    rename: { files: "**/*.txt", run(ctx, file) { file.path = path.withExt(file.path, "md"); } },
    drop: { files: "**/*", run(ctx, file) { file.drop(); } },
  },
});
"##,
    );
    let factory = load(dir.path(), "");
    let mut instance = factory.instantiate().unwrap();
    let (file, outcome) = process(instance.as_mut(), "rename", "docs/a.txt", "x");
    assert_eq!(outcome, ProcessOutcome::Modified);
    assert_eq!(file.path, "docs/a.md");
    let (_, outcome) = process(instance.as_mut(), "drop", "docs/a.md", "x");
    assert_eq!(outcome, ProcessOutcome::Dropped);
}

#[test]
fn processor_receives_options_and_pack() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin<{ suffix: string }>({
  processors: {
    tag: {
      files: "**/*",
      priority: 3,
      run(ctx, file) { file.text = `${file.text}${ctx.options.suffix}|${ctx.plugin}|${ctx.pack.name}|${ctx.pack.format}`; },
    },
  },
});
"##,
    );
    let factory = load(dir.path(), "suffix = \"!\"");
    assert_eq!(factory.processors()[0].priority, 3);
    let mut instance = factory.instantiate().unwrap();
    let (file, _) = process(instance.as_mut(), "tag", "a.txt", "x");
    assert_eq!(text(&file), "x!|ts-test|test-pack|34");
}

#[test]
fn module_state_persists_across_files_on_one_instance() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
let count = 0;
export default definePlugin({
  processors: { count: { files: "**/*", run(ctx, file) { file.text = String(++count); } } },
});
"##,
    );
    let factory = load(dir.path(), "");
    let mut first = factory.instantiate().unwrap();
    assert_eq!(text(&process(first.as_mut(), "count", "a", "").0), "1");
    assert_eq!(text(&process(first.as_mut(), "count", "b", "").0), "2");
    let mut second = factory.instantiate().unwrap();
    assert_eq!(text(&process(second.as_mut(), "count", "a", "").0), "1");
}

const GENERATOR: &str = r##"
import { definePlugin } from "#rpp";
let calls = 0;
export default definePlugin({
  async generate(ctx) {
    calls++;
    const names = ctx.files("**/*.json");
    ctx.emit("index.txt", names.join(",") + ":" + calls);
    ctx.emit("bytes.bin", new Uint8Array([1, 2]));
    ctx.emitOutput("docs", "a/b.md", ctx.readText("a.json") ?? "missing");
    ctx.remove("old.txt");
    ctx.emit("source.txt", ctx.readSourceText("s.txt") ?? "none");
    ctx.emit("absent.txt", String(ctx.read("nope.json")));
  },
});
"##;

#[test]
fn generator_reads_and_emits() {
    let dir = write_plugin(GENERATOR);
    let factory = load(dir.path(), "");
    assert!(factory.has_generator());
    let mut host = Recorder {
        outputs: vec![("a.json".into(), b"{}".to_vec()), ("b.txt".into(), vec![])],
        sources: vec![("s.txt".into(), b"src".to_vec())],
        ..Recorder::default()
    };
    factory.instantiate().unwrap().generate(&mut host).unwrap();
    let emitted: Vec<_> = host
        .emitted
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect();
    assert_eq!(
        emitted,
        vec![
            ("index.txt", &b"a.json:1"[..]),
            ("bytes.bin", &[1, 2][..]),
            ("source.txt", &b"src"[..]),
            ("absent.txt", &b"undefined"[..]),
        ]
    );
    assert_eq!(host.removed, vec!["old.txt"]);
    assert_eq!(
        host.root_outputs,
        vec![("docs".into(), "a/b.md".into(), b"{}".to_vec())]
    );
}

#[test]
fn generator_starts_from_fresh_module_state() {
    let dir = write_plugin(GENERATOR);
    let factory = load(dir.path(), "");
    let mut instance = factory.instantiate().unwrap();
    for _ in 0..2 {
        let mut host = Recorder::default();
        instance.generate(&mut host).unwrap();
        assert_eq!(host.emitted[0].1, b":1");
    }
}

#[test]
fn hooks_run() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  onStart(ctx) { throw new Error(`start ${ctx.plugin}`); },
  async onFinish(ctx, stats) { throw new Error(`finish ${stats.processed}/${stats.cached}/${stats.generated}/${stats.dropped}`); },
});
"##,
    );
    let mut instance = load(dir.path(), "").instantiate().unwrap();
    let error = instance.on_build_start().unwrap_err();
    assert!(matches!(error, Error::Hook { .. }), "{error}");
    assert!(error.to_string().contains("start ts-test"), "{error}");
    let stats = BuildStats {
        processed: 3,
        cached: 2,
        generated: 1,
        dropped: 4,
    };
    let error = instance.on_build_finish(&stats).unwrap_err();
    assert!(error.to_string().contains("finish 3/2/1/4"), "{error}");
}

#[test]
fn errors_show_typescript_locations() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  processors: { boom: { files: "**/*", run() { throw new Error("boom"); } } },
});
"##,
    );
    let mut instance = load(dir.path(), "").instantiate().unwrap();
    let mut file = PackFile::new("a.txt", Vec::new());
    let error = instance.process("boom", &mut file).unwrap_err();
    assert!(matches!(error, Error::Processor { .. }), "{error}");
    let message = error.to_string();
    assert!(message.contains("boom"), "{message}");
    assert!(message.contains("src/plugin.ts:"), "{message}");
    // The instance keeps working after a thrown error.
    assert!(instance.process("boom", &mut file).is_err());
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
    let before = load(dir.path(), "").cache_key();
    assert_eq!(load(dir.path(), "").cache_key(), before);
    write_file(dir.path(), "src/helper.ts", "export const value = \"two\";");
    assert_ne!(load(dir.path(), "").cache_key(), before);
}

#[test]
fn options_change_cache_key() {
    let dir = helper_plugin("one");
    assert_ne!(
        load(dir.path(), "a = 1").cache_key(),
        load(dir.path(), "a = 2").cache_key()
    );
}

#[test]
fn toml_and_hash_helpers() {
    let dir = write_plugin(
        r##"
import { definePlugin, hash, path, toml } from "#rpp";
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
    let mut instance = load(dir.path(), "").instantiate().unwrap();
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
import { definePlugin, process } from "#rpp";
export default definePlugin({
  generate() { process.run({ program: "true" }); },
});
"##,
    );
    let mut instance = load(dir.path(), "").instantiate().unwrap();
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
fn missing_default_export_fails_to_load() {
    let dir = write_plugin("export const plugin = {};\n");
    assert!(matches!(
        try_load(dir.path(), ""),
        Err(Error::PluginLoad { .. })
    ));
    let dir = write_plugin("export default 42;\n");
    let error = try_load(dir.path(), "").err().unwrap();
    assert!(matches!(error, Error::PluginLoad { .. }), "{error}");
}

#[test]
fn deterministic_random_per_file() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  processors: { rand: { files: "**/*", run(ctx, file) { file.text = String(Math.random()); } } },
});
"##,
    );
    let factory = load(dir.path(), "");
    let run = |path: &str| {
        let mut instance = factory.instantiate().unwrap();
        text(&process(instance.as_mut(), "rand", path, "").0).to_string()
    };
    assert_eq!(run("a.txt"), run("a.txt"));
    assert_ne!(run("a.txt"), run("b.txt"));
}

#[test]
fn module_level_randomness_validates_and_absent_pack_fields_are_undefined() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  processors: {
    [String(Math.random())]: {
      files: "**/*",
      run(ctx, file) { file.text = String(ctx.pack.description === undefined); },
    },
  },
});
"##,
    );
    let factory = load(dir.path(), "");
    let name = factory.processors()[0].name.clone();
    let mut instance = factory.instantiate().unwrap();
    let (file, _) = process(instance.as_mut(), &name, "a.txt", "");
    assert_eq!(text(&file), "true");
}

#[test]
fn clocks_alone_keep_randomness_fixed() {
    let dir = write_plugin(
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  processors: { rand: { files: "**/*", run(ctx, file) { file.text = String(Math.random()); } } },
});
"##,
    );
    let mut access = RuntimeAccess::sandboxed(".".into());
    access.permissions.clocks = true;
    let factory = try_load_with(dir.path(), "", access).unwrap();
    let run = || {
        let mut instance = factory.instantiate().unwrap();
        text(&process(instance.as_mut(), "rand", "a.txt", "").0).to_string()
    };
    assert_eq!(run(), run());
}

#[test]
fn process_cwd_must_stay_inside_project() {
    let dir = write_plugin(
        r##"
import { definePlugin, process } from "#rpp";
export default definePlugin({
  generate() { process.run({ program: "true", cwd: "../outside" }); },
});
"##,
    );
    let mut access = RuntimeAccess::sandboxed(".".into());
    access.security = rpp::config::SecurityMode::Trusted;
    access.permissions.process = vec!["true".into()];
    let mut instance = try_load_with(dir.path(), "", access)
        .unwrap()
        .instantiate()
        .unwrap();
    let error = instance.generate(&mut Recorder::default()).unwrap_err();
    assert!(error.to_string().contains("must not contain"), "{error}");
}

#[test]
fn loads_plugin_with_only_rpp_json() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "rpp.json",
        r#"{ "name": "json-only", "version": "1.0.0" }"#,
    );
    write_file(
        dir.path(),
        "src/plugin.ts",
        r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  processors: { set: { files: "**/*", run(ctx, file) { file.text = "ok"; } } },
});
"##,
    );
    let factory = load(dir.path(), "");
    let mut instance = factory.instantiate().unwrap();
    let (file, _) = process(instance.as_mut(), "set", "a.txt", "x");
    assert_eq!(text(&file), "ok");
}
