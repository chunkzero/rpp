//! TypeScript plugins on V8 through `rpp-js` (spec §4).
//!
//! A plugin whose `rpp.json` `entry` ends in `.ts`, `.mts`, `.js` or `.mjs` is bundled with
//! three virtual modules:
//!
//! - `#rpp`: the SDK ([`SDK_FILES`]), whose public API is `sdk/index.ts`.
//! - `rpp:runtime`: `sdk/runtime.ts`, the dispatcher below.
//! - `rpp:entry`: `import plugin from "./<entry>"; import { register } from "rpp:runtime";
//!   register(plugin); export * from "rpp:runtime";`
//!
//! A manifest with `discover` patterns also gets `#rpp/config`, a `#plugins/<id>` package for
//! its config module, `#plugin` (the entry) and `rpp:discovered` (the matched files' namespace
//! objects); the bundle root is the source dir. `rpp:entry` then calls
//! `register(plugin, discovered)`. Processors cannot read `ctx.discovered()`.
//!
//! Bundles are cached under `<cache dir>/bundles` and reused while the request and every
//! input file are unchanged.
//!
//! The bundle exports these functions, called through [`rpp_js::Runtime::call`]:
//!
//! | export | args | bytes | returns |
//! |---|---|---|---|
//! | `describe` | `null` | – | `{ processors: [{ name, files: string[], priority }], generator, onStart, onFinish }` |
//! | `init` | `{ plugin, options, pack: { name, description?, format? } }` | – | `null` |
//! | `process` | `{ processor, path }` | contents | final contents (`Uint8Array`) |
//! | `generate` | `null` | – | `null` |
//! | `onStart` | `null` | – | `null` |
//! | `onFinish` | `{ processed, cached, generated, dropped }` | – | `null` |
//!
//! `process` reports the final path and drop flag with the `file` host call before
//! returning. Host calls (`__rpp.call(name, value, bytes?)`):
//!
//! | name | value | bytes | reply |
//! |---|---|---|---|
//! | `file` | `{ path, dropped }` | – | `null` |
//! | `files` / `source_files` | `{ glob? }` | – | `string[]` |
//! | `read` / `read_source` | `{ path }` | – | `{ found }` + bytes |
//! | `emit` | `{ path }` | contents | `null` |
//! | `remove` | `{ path }` | – | `null` |
//! | `emit_output` | `{ root, path }` | contents | `null` |
//! | `toml.parse` | `{ text }` | – | value |
//! | `toml.stringify` | `{ value }` | – | string |
//! | `hash` | `{ algorithm: "xxh3" \| "sha256" \| "md5" \| "crc32" }` | data | string, or number for crc32 |
//! | `glob.match` | `{ pattern, path }` | – | boolean |
//! | `process.run` | `{ program, args, cwd?, env, timeout_ms? }` | stdin | `{ status, stderr }` + stdout |
//! | `component.load` | `{ name }` | – | `{ handle, functions: [{ path, params: [[name, type]], results: [type] }] }` |
//! | `component.call` | `{ handle, path, args }` | byte lists, by `[offset, len]` | `{ results }` + byte lists, or `{ failure: { kind: "trap" \| "timeout", message } }` |
//!
//! Generator host calls fail outside `generate`; `process.run` additionally requires
//! process permissions. Component instances live until the call's host is dropped. Paths are validated as relative pack paths.
//!
//! Bundle evaluation and `init` always run on a fixed clock seeded by the plugin id;
//! calls use real time and randomness only when both
//! `permissions.clocks` and `permissions.random` are granted, otherwise a fixed clock.
//!
//! Each worker thread owns one [`rpp_js::Engine`]. A [`PluginInstance`] keeps one
//! runtime for processors (module state persists between files) and loads a fresh
//! runtime for each `generate`, `onStart` and `onFinish` call.
//!
//! [`PluginInstance`]: crate::model::PluginInstance

mod bundle_cache;
#[cfg(feature = "wasm")]
mod component;
mod config;
mod discover;
mod factory;
mod host;
mod instance;

pub use config::{evaluate_config, ConfigPackage, EvaluatedConfig, CONFIG_FILE};
pub use factory::{JsPluginFactory, JsPluginLimits};

/// The embedded SDK, as `(relative path, contents)`. `rpp codegen` writes these
/// under `.rpp/sdk/`.
pub const SDK_FILES: &[(&str, &str)] = &[
    ("index.ts", include_str!("sdk/index.ts")),
    ("config.ts", include_str!("sdk/config.ts")),
];

/// Whether an `rpp.json` entry selects the JavaScript runtime.
pub fn is_js_entry(entry: &str) -> bool {
    [".ts", ".mts", ".js", ".mjs"]
        .iter()
        .any(|ext| entry.ends_with(ext))
}
