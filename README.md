# rpp

`rpp` is a Rust toolchain for building Minecraft resource packs with
incremental Lua and WASIp2 component plugins.

The project is currently `0.1.0-alpha.0`. It provides sandboxed Lua 5.4
plugins, Wasmtime-hosted component plugins, content-addressed incremental
builds, GitHub plugin locking, deterministic release archives, and a watch
server with live-reload events.

## Quick Start

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
roots.

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

Working plugins live under [`examples/plugins`](examples/plugins).

## Development

```bash
just --list
just ci
cargo test -p rpp --features wasm
cargo clippy -p rpp --features wasm --all-targets -- -D warnings
just wasm-example
```

The WASM guest examples are intentionally outside the Cargo workspace.

## License

MIT
