//! A packable plugin fixture shared by the `plugin pack` and registry install tests.

use std::path::Path;

use serde_json::Value;

use super::{run, stderr, write};

pub const PLUGIN_MANIFEST: &str = r#"{
  "name": "packed",
  "version": "1.2.3",
  "description": "A packed plugin",
  "rpp": ">=0.1",
  "entry": "src/plugin.ts",
  "config": "src/config.ts",
  "components": { "tool": "tool.wasm" },
  "dependencies": { "dep": "^1" }
}
"#;

/// A plugin whose processor appends `dep`'s mark, or throws from `src/helper.ts` for
/// the text `explode`.
pub fn plugin(root: &Path) {
    write(root, "rpp.json", PLUGIN_MANIFEST);
    write(root, "tool.wasm", "(component)");
    write(
        root,
        "node_modules/dep/package.json",
        r#"{"name":"dep","version":"1.0.0","type":"module","exports":"./index.js"}"#,
    );
    write(
        root,
        "node_modules/dep/index.js",
        "export const mark = (text) => text + '!';\n",
    );
    write(
        root,
        "src/helper.ts",
        "export function check(text: string): void {\n  if (text === 'explode') {\n    throw new Error('boom from helper');\n  }\n}\n",
    );
    write(
        root,
        "src/config.ts",
        r##"import { definePluginConfig, type Access, type PluginEntry } from "rpp:config";

type Options = { text: string };
const config: (options: Options, access?: Access) => PluginEntry = definePluginConfig<Options>(
  "packed",
);
export default config;
"##,
    );
    write(
        root,
        "src/plugin.ts",
        r##"import { definePlugin } from "rpp";
import { mark } from "dep";
import { check } from "./helper";

export default definePlugin<{ text: string }>({
  processors: {
    mark: {
      files: ["*.txt"],
      run(_ctx, file) {
        check(file.text);
        file.text = mark(file.text);
      },
    },
  },
});
"##,
    );
}

pub fn pack(root: &Path, out: &str) -> Value {
    let result = run(root, &["plugin", "pack", "--out", out, "--json"]);
    assert!(result.status.success(), "{}", stderr(&result));
    serde_json::from_slice(&result.stdout).unwrap()
}
