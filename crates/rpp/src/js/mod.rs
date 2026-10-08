//! TypeScript plugins on V8 through `rpp-js` (spec §4).
//!
//! A plugin whose `rpp.json` `entry` ends in `.ts`, `.mts`, `.js` or `.mjs` is bundled with
//! three virtual modules:
//!
//! - `rpp`: the SDK, `sdk/index.ts` followed by `sdk/components.ts`. [`SDK_FILES`] also
//!   carries `sdk/bridge.d.ts`, the `__rpp` type declaration the sources reference; bundles
//!   never include it.
//! - `rpp:internal/runtime`: `sdk/runtime.ts`, the dispatcher below.
//! - `rpp:internal/entry`: `import plugin from "./<entry>"; import { register } from "rpp:internal/runtime";
//!   register(plugin); export * from "rpp:internal/runtime";`
//!
//! Every bundle also gets `rpp:jsx` and `rpp:jsx/jsx-runtime` (`sdk/jsx.ts`), the import
//! source that `.tsx` files compile against.
//!
//! Every bundle also gets `rpp:config` and the plugin's own `plugin:<id>` and
//! `plugin:<id>/<subpath>` packages for its config module and exports.
//!
//! A manifest with `discover` patterns also gets `rpp:internal/plugin`
//! (the entry) and `rpp:internal/discovered` (the matched files' namespace objects); the
//! bundle root is the source dir. `rpp:internal/entry` then calls
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
//! | `init` | `{ plugin, options, pack: { name, description, format: { min, max } } }` | – | `null` |
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

mod access;
mod bundle;
mod bundle_cache;
#[cfg(feature = "wasm")]
mod component;
mod config;
mod discover;
mod factory;
mod hash;
mod host;
mod instance;
mod keys;
mod log;
mod process;
mod toml_json;

use std::time::Duration;

use crate::config::LimitsConfig;

pub use config::{evaluate_config, ConfigPackage, EvaluatedConfig, CONFIG_FILE};
pub use factory::{JsPluginFactory, JsPluginSpec};

/// The plugin SDK imported as `rpp`: `sdk/index.ts` with `sdk/components.ts` appended.
pub const SDK_INDEX: &str = concat!(
    include_str!("sdk/index.ts"),
    "\n",
    include_str!("sdk/components.ts")
);
/// `sdk/config.ts`, the config SDK imported as `rpp:config`.
pub const SDK_CONFIG: &str = include_str!("sdk/config.ts");
/// `sdk/jsx.ts`, the JSX runtime imported as `rpp:jsx` and `rpp:jsx/jsx-runtime`.
pub const SDK_JSX: &str = include_str!("sdk/jsx.ts");
/// The JSX import source of `.tsx` files.
pub const JSX_IMPORT_SOURCE: &str = "rpp:jsx";
/// `sdk/bridge.d.ts`, the `__rpp` declaration the SDK sources reference. Not a bundled module.
const SDK_BRIDGE: &str = include_str!("sdk/bridge.d.ts");

/// The embedded SDK, as `(relative path, contents)`. `rpp codegen` writes these
/// under `.rpp/sdk/`.
pub const SDK_FILES: &[(&str, &str)] = &[
    ("index.ts", SDK_INDEX),
    ("config.ts", SDK_CONFIG),
    ("jsx.ts", SDK_JSX),
    ("bridge.d.ts", SDK_BRIDGE),
];

/// The runtime limits `build.limits` sets for each plugin runtime and call.
/// The `rpp:jsx` and `rpp:jsx/jsx-runtime` virtual modules.
pub fn jsx_modules() -> [(String, String); 2] {
    [
        (JSX_IMPORT_SOURCE.to_string(), SDK_JSX.to_string()),
        (
            format!("{JSX_IMPORT_SOURCE}/jsx-runtime"),
            SDK_JSX.to_string(),
        ),
    ]
}

/// `message` with a pointer to the current specifiers when it reports an unresolved `#rpp`
/// or `#plugins/` import.
pub(crate) fn hint_renamed_specifiers(message: String) -> String {
    if message.contains("cannot import `#rpp") || message.contains("cannot import `#plugins/") {
        format!(
            "{message}\nthe SDK is imported as `rpp`, `rpp:config` and `rpp:jsx`, and plugin \
             modules as `plugin:<name>`"
        )
    } else {
        message
    }
}

fn runtime_limits(limits: &LimitsConfig) -> rpp_js::Limits {
    rpp_js::Limits {
        heap_bytes: limits.memory_limit_mb as usize * 1024 * 1024,
        time: Duration::from_secs(limits.execution_deadline_seconds),
    }
}
