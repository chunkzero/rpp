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

Prompts and build status go to stderr; plugin list and search results go to stdout.
Redirected status output is plain text. Prompts use defaults when stdin or stderr
is redirected; use `init --yes` or `plugin add --project` / `--global` to skip them
in a terminal. Set `NO_COLOR=1` to disable color, and use `-v` / `-vv` or `RUST_LOG`
to control diagnostic and dev-server logs.

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

`mise.toml` pins Rust, kache, just, JDKs 21 and 25, Node (for oxfmt), and the repository
formatters. It installs rustfmt, Clippy, and the `wasm32-wasip2` target, and exposes
both JDKs to Gradle. The workspace's supported Rust baseline is 1.96.0. Keep the manifest baseline
and mise toolchain pin aligned when updating Rust, and run `just verify-wasm`
before adopting a new toolchain: guest imports must remain compatible with the sandbox. With mise activated in your shell,
you can run `just` and `cargo` directly.

[Kache](https://github.com/kunobi-ninja/kache) is enabled through mise's
`RUSTC_WRAPPER` environment variable, including builds of the standalone WASM guests.
It shares compiled artifacts between checkouts through its per-user cache. Keep
that cache on the same filesystem as the checkouts to benefit from copy-on-write
restores. Use `mise exec -- kache stats` to inspect it, or set `KACHE_DISABLED=1`
for a build that bypasses caching. Cargo outside the mise environment uses your
normal wrapper configuration.

Kache does not remove existing `target/` directories when enabled. Preview stale
targets with `mise exec -- kache clean --tracked --stale 14d --dry-run` before
cleaning them, and run `mise exec -- kache gc` to reclaim unused cache entries.
Cache size limits apply to the shared store; outputs retained by targets can keep
disk space in use. GitHub Actions persists kache's store separately for each build
job, with a 2 GiB retention budget per job.

Use `just check-crate <crate>`, `just lint-crate <crate>`, and
`just test-crate <crate> <test-filter>` while iterating. Check and lint recipes accept
Cargo feature flags. `just check-features` checks the core library independently in
its core-only, default Lua, and Lua + WASM + tracing configurations.

`just fmt` formats Rust (including standalone WASM guests), Java, Gradle Kotlin
scripts, Lua, configuration, documentation, and the justfile. `just fmt-check`
checks the same files. Scoped recipes are `fmt-rust`, `fmt-jvm`, `fmt-lua`, and
`fmt-config`; use `fmt-rust --check` or the other recipes' `-check` variants to
check without writing. Generated files, lockfiles, and pack data used by examples
are excluded from configuration formatting. Lua formatting uses Lua 5.4 syntax.

Shared VS Code and Zed settings select these formatters. For VS Code, install the
recommended extensions and launch it with `mise exec -- code .` so its formatter
extensions find the pinned binaries. Zed's external formatters invoke mise directly.
Both editors should open the repository root. On Windows, the just recipes require
Git Bash; the dedicated Windows core checks also run directly through Cargo.

Run `just verify-wasm` for component host and CLI integration tests. It fails when
the guest target is missing, instead of allowing those tests to skip. Guest crates
have their own committed lockfiles outside the workspace.

Run `just verify-jvm` to build the CLI, test the JVM client against an actual dev
server, and compile the Minestom example. `just jvm <tasks>` runs the Gradle wrapper
with the configured JDKs. The verification recipe reruns the test task so a previous
run without `RPP_BIN` cannot silently skip the process integration test.

Run `just ci` before publishing substantial changes. GitHub Actions runs the same
format, lint, feature, workspace test, and JVM checks in separate jobs on standard
GitHub runners, including WASM prerequisites. `just verify-rust` runs only the Rust checks.
Verification commands use `--locked`; update lockfiles deliberately when changing
dependencies.

## License

MIT

For Minecraft server live pack updates, see the [JVM client and Minestom/Spigot examples](integrations/jvm/README.md).
