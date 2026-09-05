# rpp

`rpp` is a Rust toolchain for building Minecraft resource packs with
incremental Lua and WASIp2 component plugins.

The project is currently `0.1.0-alpha.0`. It provides sandboxed Lua 5.4
plugins, Wasmtime-hosted component plugins, content-addressed incremental
builds, GitHub plugin locking, deterministic release archives, and a watch
server with live-reload events.

## Quick Start

First complete the [development setup](#development) to install the pinned tools.
The commands below assume mise is activated; otherwise prefix them with `mise exec --`.

```bash
cargo build --release
target/release/rpp init my-pack --yes
cd my-pack
../target/release/rpp build
```

To build the checked-in example:

```bash
cd examples/pack
cargo run -p rpp-cli -- build
cargo run -p rpp-cli -- build
```

The second build should report `processed 0`. Loose pack files remain the
engine's unsquashed output; optimization is applied to the release zip only.

## Configuration

```toml
[pack]
name = "my-pack"
description = "My resource pack"
pack_format = 34

[build]
source = "src"
output = "dist"

[build.squash]
enabled = true
engine = "builtin"
json = true
png = "fast"
zip = true

[[plugin]]
source = "path:plugins/example"
```

Build paths must be separate, project-relative directories. Plugin-produced
paths are normalized relative pack paths and cannot escape the source or output
roots. Plugins may also emit into named external roots explicitly declared by the
project; RPP atomically replaces owned artifacts, removes only its own stale
generated files, and preserves handwritten neighbors.

## Commands

```text
rpp init [dir]
rpp build [--no-cache] [--no-squash] [--jobs N]
rpp dev
rpp clean
rpp plugin add|remove|list|update|search

# Add from GitHub or a plugin package directory; choose project or global scope
rpp plugin add ../window
rpp plugin add github:owner/repo --global
```

`rpp dev` serves loose output. Builtin squash and PackSquash are release archive
operations and do not run in dev mode.

## Plugin Authoring

- [Lua plugin guide](docs/LUA_PLUGINS.md)
- [WASM plugin guide](docs/WASM_PLUGINS.md)
- [Authoritative specification](docs/SPEC.md)

Working plugins live under [`examples/plugins`](examples/plugins), including a
WASIp2 component plugin (`just example-wasm` builds its guest crate).

## Development

Install [mise](https://mise.jdx.dev/getting-started.html) and a C compiler and native
linker (for example, `build-essential` on Ubuntu or Xcode Command Line Tools on
macOS). RPP's `mlua` dependency builds bundled Lua 5.4 from C source, and the final
Rust executable needs a native linker.

From the repository root:

```bash
mise trust
mise install
mise exec -- just --list
mise exec -- just check-crate rpp
```

`mise.toml` pins Rust and just and installs rustfmt, Clippy, and the `wasm32-wasip2`
target. The workspace's supported Rust baseline is 1.96.0. Keep the manifest baseline
and mise toolchain pin aligned when updating Rust, and run `just verify-wasm`
before adopting a new toolchain: guest imports must remain compatible with the sandbox. With mise activated in your shell,
you can run `just` and `cargo` directly.

Use `just check-crate <crate>`, `just lint-crate <crate>`, and
`just test-crate <crate> <test-filter>` while iterating. Check and lint recipes accept
Cargo feature flags. `just check-features` checks the core library independently in
its core-only, default Lua, and Lua + WASM + tracing configurations.

Run `just verify-wasm` for component host and CLI integration tests. It fails when
the guest target is missing, instead of allowing those tests to skip. Guest crates
have their own committed lockfiles outside the workspace.

Run `just ci` before publishing substantial changes. GitHub Actions runs the same
format, lint, feature, and workspace test checks, including WASM prerequisites.
Verification commands use `--locked`; update lockfiles deliberately when changing
dependencies.

## License

MIT

For Minecraft server live pack updates, see the [JVM client and Minestom/Spigot examples](integrations/jvm/README.md).
