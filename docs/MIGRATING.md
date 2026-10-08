# Migrating from `rpp.toml` and Lua plugins

RPP projects are configured with TypeScript and plugins are TypeScript packages. `rpp.toml`,
`plugin.toml` and Lua plugins are no longer read. A project with `rpp.toml` and no
`rpp.config.ts` fails with a pointer to this guide.

## `rpp.toml` to `rpp.config.ts`

Keys become camelCase and sections become objects:

| `rpp.toml`                         | `rpp.config.ts`                                                |
| ---------------------------------- | -------------------------------------------------------------- |
| `[pack] name, description`         | `pack: { name, description }`                                  |
| `pack_format`                      | `pack.format` (a number or `{ min, max }`)                     |
| `[build] source, output, workers`  | `build: { source, output, workers }`                           |
| `[build.lua]`                      | `build.limits` (`memoryLimitMb`, `executionDeadlineSeconds`)   |
| `[build.wasm]`                     | `build.wasm`                                                   |
| `[build.squash] packsquash_binary` | `build.squash.packsquashBinary` (likewise `packsquashOptions`) |
| `[dev]`                            | `dev: { host, port, open }`                                    |
| `[[plugin]]`                       | one entry in `plugins: [...]`, in the same order               |
| `source = "path:../x"`             | a `path:` dependency in `rpp.json` (below)                     |
| `[plugin.options]`                 | the options argument, `plugin("name", { ... })`                |
| `security`, `[plugin.permissions]` | the access argument: `{ security, permissions }`               |
| `[plugin.outputs]`                 | `{ outputs: { root: "dir" } }`                                 |

`security = "native"`, `permissions.lua`, `id`, `ref` and `subdir` have no equivalent and are
rejected. Plugin permissions need `security: "trusted"`.

```ts
import { defineConfig, plugin } from "rpp:config";

export default defineConfig({
  pack: { name: "my-pack", format: 34 },
  plugins: [plugin("minify", { pretty: false }), plugin("codegen", undefined, { outputs: { kotlin: "gen" } })],
});
```

rpp generates `pack.mcmeta` from `pack`. Move the fields of `src/pack.mcmeta` into it
(`description`, `format`, `overlays`, `filter`, `language`) and delete the file; a source
`pack.mcmeta` fails the build. `pack.packFormat` is rejected in favor of `pack.format`.

A plugin that ships a config module is configured through its factory instead, with typed and
validated options: `import minify from "plugin:minify"` then `minify({ pretty: false })`.

## Plugin sources to `rpp.json`

List every plugin the project uses in `rpp.json`, keyed by the plugin's name:

```json
{ "dependencies": { "minify": "^1.2.0", "local-tools": "path:plugins/local-tools" } }
```

- `path:<dir>` stays a path dependency, relative to the project root.
- `github:owner/repo` is gone: publish the plugin to the registry (see [publishing.md](publishing.md))
  or use `path:`.
- User-global plugins (`~/.rpp/plugins.toml`) are ignored.
- Delete `rpp.lock` (versions 1 and 2 are rejected) and run `rpp build` to write a new one.

## `plugin.toml` to `rpp.json`

| `plugin.toml`                     | plugin `rpp.json`                                                |
| --------------------------------- | ---------------------------------------------------------------- |
| `[plugin] id`                     | `name`                                                           |
| `version`, `description`          | unchanged; `authors` is dropped                                  |
| `entry = "init.lua"`              | `entry: "src/plugin.ts"` (a `.ts`, `.mts`, `.js` or `.mjs` file) |
| `overrides = [...]`               | `overrides: [...]`                                               |
| `[component.x] module = "x.wasm"` | `components: { "x": "x.wasm" }`                                  |

New optional keys: `rpp` (supported rpp range, required to publish), `config` (the options
factory module) and `discover` (globs of authoring modules, replacing `ctx:source_files` plus
`ctx:load_source`).

## Lua API to the TypeScript SDK

Import from `rpp` and default-export `definePlugin({ ... })`.

| Lua                                               | TypeScript                                                     |
| ------------------------------------------------- | -------------------------------------------------------------- |
| `plugin:processor(name, { files, priority }, fn)` | `processors: { name: { files, priority, run(ctx, file) {} } }` |
| `plugin:generator(name, fn)`                      | `generate(ctx) {}`                                             |
| `plugin:on_start(fn)`, `on_finish(fn)`            | `onStart(ctx)`, `onFinish(ctx, stats)`                         |
| `file.text`, `file.bytes`, `file:drop()`          | `file.text`, `file.bytes` (`Uint8Array`), `file.drop()`        |
| `ctx:files`, `ctx:source_files`                   | `ctx.files`, `ctx.sourceFiles`                                 |
| `ctx:read`, `ctx:read_source`                     | `ctx.read`/`readText`, `ctx.readSource`/`readSourceText`       |
| `ctx:emit`, `ctx:remove`, `ctx:emit_output`       | `ctx.emit`, `ctx.remove`, `ctx.emitOutput`                     |
| `ctx:load_source(path)`                           | a `discover` pattern and `ctx.discovered(name)`                |
| `ctx.options`, `ctx.pack`                         | `ctx.options`, `ctx.pack` (`format` as before)                 |
| `rpp.json.encode/decode`                          | `JSON.stringify`/`JSON.parse`                                  |
| `rpp.hash.*`, `rpp.path.*` (`with_ext`)           | `hash.*`, `path.*` (`withExt`)                                 |
| `rpp.process.run{...}`                            | `process.run({ ... })` (`timeoutMs`)                           |
| `rpp.component.load(n):call(f, ...)`              | `components.load(n).exports.f(...)`                            |
| `rpp.str.*`, `ctx.log.*`, `error(msg)`            | built-in string methods; `throw new Error(msg)`                |

Component values change shape: `list<u8>` is a `Uint8Array`, an `option` is the value or
`undefined`, and a `result` returns its `ok` value or throws. See [WASM_PLUGINS.md](WASM_PLUGINS.md).
JSON objects now keep insertion order instead of being sorted by key.

## Removed commands

| Removed                                           | Use instead                                               |
| ------------------------------------------------- | --------------------------------------------------------- |
| `rpp plugin add` / `remove` / `update` / `search` | `rpp add` / `rpp remove` / `rpp update` / `rpp search`    |
| `rpp plugin list`                                 | read `rpp.json`                                           |
| `rpp component bindgen`                           | `rpp codegen` writes component types to `.rpp/generated/` |
| LuaLS definitions in `.rpp/api`                   | `rpp codegen` and `rpp check` (TypeScript 7+)             |

`rpp plugin pack` is unchanged. See [`examples/`](../examples) for converted projects.
