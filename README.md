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

With [mise](https://mise.jdx.dev), on Linux, macOS, and Windows x64:

```toml
[tools]
"github:chunkzero/rpp" = "latest"
```

Nightlies install through the [mise-chunkzero](https://github.com/chunkzero/mise-chunkzero) plugin:

```toml
[plugins]
chunkzero = "https://github.com/chunkzero/mise-chunkzero"

[tools]
"chunkzero:rpp-nightly" = { version = "latest", prerelease = true }
```

Without mise, on Linux x64/arm64 and macOS Intel/Apple Silicon (use a published version):

```bash
version=RELEASE_VERSION
curl -fsSLO "https://github.com/chunkzero/rpp/releases/download/v$version/install.sh"
sh install.sh "$version"
```

This installs into `~/.local/share/rpp/<version>` and links `~/.local/bin/rpp`. Set
`RPP_INSTALL_DIR` to use another prefix.

On other platforms, build from source with Rust 1.96 or newer:

```bash
cargo install --locked --git https://github.com/chunkzero/rpp rpp-cli
```

## Releases

The workspace version in `Cargo.toml` is the upcoming release. Daily builds of `main`
publish nightlies as `X.Y.Z-nightly.<UTC commit time>.g<12-character commit>`, e.g.
`0.1.0-nightly.20261004062300.ge282f11816cd`; a commit always gets the same version, and
unchanged commits are skipped. A manual `Release` dispatch on `main` with `channel=release`
publishes the workspace version itself. Releases are immutable and never deleted, so
pinned versions and lockfiles keep working.

Each release has `rpp-<version>-<platform>.tar.gz` archives with `.sha256` files for
linux-x64, linux-arm64, darwin-x64, darwin-arm64, and windows-x64. An archive holds one
directory with the `rpp` executable, its bundled TypeScript compiler, and `release.json`
recording the source commit. `rpp --version` reports the full version. The
[JVM dev client](integrations/jvm/README.md) is published to Maven at the same version
before the GitHub release, and the release is then added to the mise registry.

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
  pack: { name: "my-pack", description: "My resource pack", format: 34 },
  build: { source: "src", output: "dist" },
  plugins: [jsonMinify({ pretty: false })],
});
```

rpp generates `pack.mcmeta` from `pack`, so `src/` holds only pack content. `format` also takes
an inclusive `{ min, max }` range, and `pack` accepts `overlays`, `filter`, and `language`.

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
