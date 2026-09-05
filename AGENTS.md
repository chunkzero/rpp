# AGENTS.md

Guidelines for agents working in the RPP (Resource Pack Processor) repository.

## Project Overview

RPP is a Rust toolchain for building Minecraft resource packs. The design document is
`docs/SPEC.md`; read it before architectural changes. Workspace:

- `crates/rpp`: core library. `config` (`rpp.toml`), `manifest` (`plugin.toml`), `model`
  (runtime-agnostic plugin traits), `lua` (sandboxed Lua 5.4 runtime and builtins),
  `engine` (incremental build: discovery, file phase, generators, output sync), `cache`
  (bincode manifest + content-addressed object store under `.rpp/cache/`).
- `crates/rpp-fetch`: plugin source resolution (GitHub + local), `rpp.lock`, plugin search.
- `crates/rpp-squash`: JSON minify, PNG optimization (oxipng), deterministic zip, external
  PackSquash.
- `crates/rpp-wasm`: WASIp2 component host on wasmtime.
- `crates/rpp-cli`: the `rpp` binary (init, build, dev server, plugin management,
  component bindgen).
- `examples/`: an example pack project and example plugin packages.

Dependency direction: `rpp-fetch`, `rpp-squash`, `rpp-wasm` do not depend on `rpp`.
`rpp` optionally depends on `rpp-wasm` (feature `wasm`). `rpp-cli` depends on all of them.

## Commands

```bash
just --list                 # task runner; `just ci` = fmt-check + lint + test
cargo check --workspace --all-targets
cargo test -p rpp           # prefer package-scoped commands when iterating
cargo test -p rpp --features wasm
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p rpp-cli -- <args>
```

WASM tests build guest fixture crates under `crates/*/tests/fixtures/` (not workspace
members) with `--target wasm32-wasip2`. They skip when that target is not installed
(`rustup target add wasm32-wasip2`).

## Code Style

- Standard `cargo fmt`; edition 2021; 100 column lines.
- Imports in three groups separated by blank lines: std, external crates, `crate::`/`super::`.
- Libraries define errors with `thiserror` (main type `Error`, alias
  `pub type Result<T> = std::result::Result<T, Error>`). The CLI uses `anyhow` with
  `.context()` on I/O.
- Logging via `tracing`; the CLI's `ui` module handles user-facing output.
- `pub(crate)` for internal items; feature gates use `dep:` optional dependencies.
- Shared dependency versions live in `[workspace.dependencies]` in the root `Cargo.toml`.
- Document public APIs with `///`. Use `//` sparingly and only for behavior, not history.

## Notes

- `mlua` uses vendored **Lua 5.4** (features `lua54, vendored, serde, send`), not LuaJIT.
- The incremental cache lives at `.rpp/cache/`; external generated-file ownership at
  `.rpp/external-outputs.bin`.
- Source trees honor `.rppignore`.
- `crates/rpp` features: `lua` (default), `wasm`, `tracing`.
