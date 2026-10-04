//! TypeScript processors: file mutation, renames, drops, options, module state and errors.

#![cfg(feature = "js")]

mod common;

use common::js::{load, process, text, write_plugin};
use rpp::model::{PackFile, PluginFactory, ProcessOutcome};
use rpp::Error;

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
    let factory = load(dir.path(), "{}");
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
    let factory = load(dir.path(), "{}");
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
      run(ctx, file) { file.text = `${file.text}${ctx.options.suffix}|${ctx.plugin}|${ctx.pack.name}|${ctx.pack.format.min}-${ctx.pack.format.max}`; },
    },
  },
});
"##,
    );
    let factory = load(dir.path(), r#"{"suffix":"!"}"#);
    assert_eq!(factory.processors()[0].priority, 3);
    let mut instance = factory.instantiate().unwrap();
    let (file, _) = process(instance.as_mut(), "tag", "a.txt", "x");
    assert_eq!(text(&file), "x!|ts-test|test-pack|34-34");
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
    let factory = load(dir.path(), "{}");
    let mut first = factory.instantiate().unwrap();
    assert_eq!(text(&process(first.as_mut(), "count", "a", "").0), "1");
    assert_eq!(text(&process(first.as_mut(), "count", "b", "").0), "2");
    let mut second = factory.instantiate().unwrap();
    assert_eq!(text(&process(second.as_mut(), "count", "a", "").0), "1");
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
    let mut instance = load(dir.path(), "{}").instantiate().unwrap();
    let mut file = PackFile::new("a.txt", Vec::new());
    let error = instance.process("boom", &mut file).unwrap_err();
    assert!(matches!(error, Error::Processor { .. }), "{error}");
    let message = error.to_string();
    assert!(message.contains("boom"), "{message}");
    assert!(message.contains("src/plugin.ts:"), "{message}");
    // The instance keeps working after a thrown error.
    assert!(instance.process("boom", &mut file).is_err());
}
