# RPP (Resource Pack Processor)

Rust tooling for building Minecraft resource packs with incremental Lua and WASIp2 plugins.

## General guidelines

- Do not edit AGENTS.md or CLAUDE.md unless explicitly asked. Keep their guidance consistent.
- Read `docs/SPEC.md` before architectural changes; it defines the project contracts.
- Keep solutions simple and public APIs narrow. Avoid speculative abstractions.
- Preserve unrelated changes and ask before destructive actions outside the requested scope.
- `rpp-fetch`, `rpp-squash`, `rpp-wasm`, and `rpp-js` are standalone and do not depend on `rpp`.
  `rpp` optionally depends on `rpp-wasm`; `rpp-cli` composes the workspace crates.

## Tooling

- Run `mise install` after tool pins change. Use `mise exec -- <command>` when mise
  is not activated in the shell. `mise.toml` pins Rust, its components/targets, and just.
- Run `just --list` for tasks. Prefer scoped checks during development:
  - `just check-crate rpp` (accepts Cargo feature flags).
  - `just lint-crate rpp` (accepts Cargo feature flags).
  - `just test-crate rpp <test-filter>`.
- Run `just fmt` to format and `just fmt-check` to check formatting.
- `just check-features` checks core-only, default Lua, and Lua + WASM + tracing builds.
- `just verify-wasm` exercises WASM host and CLI integrations and requires `wasm32-wasip2`.
  Guest fixture crates are outside the workspace; individual tests may skip without that target.
- Run `just ci` before publishing substantial changes. Avoid full suites or rebuilds
  between individual edits. CI is the source of truth; do not rely on pre-commit hooks.
- Keep Cargo.lock files committed; verification uses `--locked`.
- Check for existing dev servers before starting one, and stop resources you start.

## Code style

- Rust edition 2021, standard rustfmt, 100 columns.
- Group imports as std, external crates, then crate/super, separated by blank lines.
- Libraries use `thiserror` with `Error` and `Result<T>`; the CLI uses `anyhow` with
  context on I/O errors. Use `tracing` for logging and the CLI's `ui` module for display.
- Use `pub(crate)` for internals, `///` for public APIs, and `dep:` for optional dependencies.
- Keep shared dependency versions in `[workspace.dependencies]`.
- Write focused behavior tests. Keep comments sparse and about current behavior.
- Lua plugins use vendored Lua 5.4, not LuaJIT.

## Git

- Use Conventional Commits for commits and PR titles.
- When merging a PR, use squash merge.
- Do not revert unrelated changes.
- Summarize changes, validation results, and known limitations when handing work back.

## Glossary

- **Processor:** a plugin operation applied to each matching file in the file phase.
- **Generator:** a sequential plugin operation that reads files and emits outputs after processing.
- **Component:** a WASIp2 module exposed to Lua through declared, typed WIT exports.
- **Cache replay:** reuse of prior outputs when tracked inputs and plugin keys still match.
- **External output:** a generated file in a declared root outside the pack output directory;
  RPP tracks ownership separately so cleanup preserves handwritten neighbors.
