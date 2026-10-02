//! TypeScript generators and lifecycle hooks.

#![cfg(feature = "js")]

mod common;

use common::js::{load, write_plugin, Recorder};
use rpp::model::{BuildStats, PluginFactory};
use rpp::Error;

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
    let factory = load(dir.path(), "{}");
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
    let factory = load(dir.path(), "{}");
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
    let mut instance = load(dir.path(), "{}").instantiate().unwrap();
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
