# rpp

Build Minecraft resource packs with plugins written in TypeScript.

rpp takes a pack source directory, runs it through your plugins, and produces a
loose pack for development and an optimized zip for release. Plugins are small
TypeScript modules: they transform files, generate new ones, and can call into
WebAssembly components when TypeScript isn't the right tool.

- **Incremental.** Builds are content-addressed. Unchanged files and plugins are
  replayed from cache, so a rebuild with nothing to do finishes almost instantly.
- **Sandboxed.** Plugins run in V8 isolates without filesystem, network, clock, or
  randomness access unless the project grants it.
- **Typed.** `rpp codegen` writes types for the SDK, your plugins' options, and any
  WASIp2 components, and `rpp check` type-checks the project with a bundled compiler.
- **Live.** `rpp dev` rebuilds on change and serves the pack to Minecraft servers
  through the [JVM dev client](integrations/jvm/README.md).
- **Shareable.** Plugins are versioned packages in
  [rpp-registry](https://github.com/chunkzero/rpp-registry), pinned by `rpp.lock`.

rpp is alpha software (`0.1.0-alpha.0`). Expect breaking changes.

## Install

Linux x64:

```bash
curl -fsSLO https://github.com/chunkzero/rpp/releases/download/v0.1.0-alpha.0/install.sh
sh install.sh 0.1.0-alpha.0
```

This installs into `~/.local/share/rpp/<version>` and links `~/.local/bin/rpp`. Set
`RPP_INSTALL_DIR` to use another prefix.

On other platforms, build from source with Rust 1.96 or newer:

```bash
cargo install --locked --git https://github.com/chunkzero/rpp rpp-cli
```

## Getting started

```bash
rpp init my-pack
cd my-pack
rpp build      # writes dist/ and dist/my-pack.zip
rpp dev        # rebuilds on change and serves the pack
```

A project has two files at its root. `rpp.config.ts` describes the pack and the
plugins it runs:

```ts
import { defineConfig } from "#rpp/config";
import jsonMinify from "#plugins/json-minify";

export default defineConfig({
  pack: { name: "my-pack", description: "My resource pack", packFormat: 34 },
  build: { source: "src", output: "dist" },
  plugins: [jsonMinify({ pretty: false })],
});
```

`rpp.json` lists the plugin packages the project depends on, either as registry
version ranges or local directories:

```json
{ "dependencies": { "json-minify": "^1.0.0", "my-plugin": "path:plugins/my-plugin" } }
```

## Writing a plugin

A plugin is a directory with an `rpp.json` manifest and a TypeScript entry point:

```ts
import { definePlugin } from "#rpp";

export default definePlugin<{ greeting?: string }>({
  // Processors run on every matching file.
  processors: {
    shout: {
      files: ["assets/*/lang/*.json"],
      run(ctx, file) {
        file.text = file.text.toUpperCase();
      },
    },
  },
  // Generators run once, after processing, and can emit new files.
  generate(ctx) {
    ctx.emit("credits.txt", `${ctx.options.greeting ?? "built"} by rpp`);
  },
});
```

Learn more:

- [Example pack and plugins](examples), including a WebAssembly component plugin
- [Publishing a plugin to the registry](docs/publishing.md)
- [WebAssembly component plugins](docs/WASM_PLUGINS.md)
- [Generated types](docs/CODEGEN.md)
- [Migrating from `rpp.toml` and Lua plugins](docs/MIGRATING.md)
- [Specification](docs/SPEC.md)

## Commands

| Command                                        | Description                                              |
| ---------------------------------------------- | -------------------------------------------------------- |
| `rpp init [dir]`                               | Create a project                                         |
| `rpp build`                                    | Build the pack (`--no-cache`, `--no-squash`, `--jobs N`) |
| `rpp dev`                                      | Rebuild on change and serve the pack                     |
| `rpp clean`                                    | Remove build output and caches                           |
| `rpp add <name>[@range]`, `rpp add path:<dir>` | Add a plugin dependency                                  |
| `rpp remove <name>`                            | Remove a plugin dependency                               |
| `rpp update [name...]`                         | Update locked plugin versions                            |
| `rpp search <query>`                           | Search the registry                                      |
| `rpp codegen`                                  | Write TypeScript types for the project                   |
| `rpp check`                                    | Type-check the project                                   |
| `rpp plugin pack [dir]`                        | Package a plugin for publishing                          |

Set `NO_COLOR=1` to disable color, and use `-v`, `-vv`, or `RUST_LOG` for more logging.

## Contributing

Development tools are pinned with [mise](https://mise.jdx.dev). You also need a C
compiler and linker (`build-essential` on Ubuntu, Xcode Command Line Tools on macOS).

```bash
mise trust && mise install
mise exec -- just --list   # available tasks
mise exec -- just ci       # everything CI runs
```

Commits and pull request titles follow [Conventional Commits](https://www.conventionalcommits.org).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
