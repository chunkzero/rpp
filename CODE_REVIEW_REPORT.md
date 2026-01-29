# RPP Code Review Report

## Scope
- Reviewed Rust sources in `crates/rpp` and `crates/rpp-cli`, examples, and architecture docs.
- Focused on correctness, error handling, concurrency, performance, security, and API clarity.
- This report suggests fixes; it does not implement them.

## Key Strengths
- Clear multi-phase build pipeline (discovery → process → finalize) with sensible layering.
- Good sandbox boundary in `FileApi` (path traversal checks, absolute path rejection).
- Consistent plugin registry with pattern matching + priority ordering.
- Cache format includes transformation chain and plugin version checks.
- Lua runtime isolates `os`, `io`, `loadfile`, `dofile` and exposes a compact plugin API.

## High Priority Findings

### 1) Lua runtime safety + concurrency mismatch
**Files:** `crates/rpp/src/plugin/lua_processor.rs`, `crates/rpp/src/lua/runtime.rs`

**Issue:** `LuaProcessor` uses a single `LuaRuntime` behind a `Mutex<Option<...>>` and shares it across worker threads. `mlua::Lua` is not `Send` without extra guarantees, and the runtime holds Lua globals + registry keys. Even if `mlua` is configured with `send`, sharing a single runtime across multiple threads is a scaling bottleneck and risks deadlock during long-running scripts.

**Suggested fixes:**
- Prefer per-worker `LuaRuntime` instances, loaded once per thread, as described in `architecture/README.md` and `architecture/errors.md`.
- If a shared runtime is required, wrap it with a `Mutex` and explicitly document that Lua processing is serialized; consider a `tokio::sync::Semaphore` to enforce this.
- Add explicit tests verifying Lua processors run safely under parallel load.

### 2) Registry mutation panic in `BuildEngine`
**Files:** `crates/rpp/src/build/engine.rs`

**Issue:** `Arc::get_mut(&mut self.registry).expect(...)` panics if the registry was cloned earlier via `registry()`. This makes API usage fragile and can panic in normal use.

**Suggested fixes:**
- Replace with `Result` and return a `BuildError::Plugin` with a clear message.
- Alternatively, remove `registry()` or return an immutable handle that cannot be cloned, or move registration into `BuildEngineBuilder` only.

### 3) Missing config propagation into processors
**Files:** `crates/rpp/src/worker/pool.rs`, `crates/rpp/src/build/engine.rs`

**Issue:** `WorkerPool::process_job` creates an empty `toml::Value` for `ProcessingContext.config`. This ignores `BuildEngine.config`, so plugin configuration in `rpp.toml` is never passed to processors.

**Suggested fixes:**
- Thread `BuildEngine.config` down into `ProcessPhase::run`, `WorkerPool`, and `ProcessingJob`.
- Validate config schema once and reuse a parsed structure to avoid repeated parsing overhead.

### 4) Output zip creation loads whole files into memory
**Files:** `crates/rpp/src/build/output.rs`

**Issue:** `create_zip_archive` reads entire files into memory with `fs::read`, which can explode memory usage on large packs.

**Suggested fixes:**
- Stream file contents into the zip writer using `std::io::copy` and `File::open`.
- Consider using `zip::write::FileOptions` with a buffered reader per file.

### 5) Discovery phase double-reads file contents
**Files:** `crates/rpp/src/build/discovery.rs`

**Issue:** Files are read once for fingerprinting and again when loading content. This doubles I/O for non-cached files.

**Suggested fixes:**
- Reuse the read buffer when computing the fingerprint for files that need processing.
- If you want to avoid full reads for large files, compute hash from a streaming reader and only load content when required.

### 6) Dev server rebuild loop has no debounce
**Files:** `crates/rpp-cli/src/dev_server/watcher.rs`, `crates/rpp-cli/src/dev_server/mod.rs`

**Issue:** `notify` emits multiple events per file change, causing repeated builds and redundant reloads.

**Suggested fixes:**
- Add an event debounce window (e.g., 50–200ms) and coalesce changes.
- Track pending builds and collapse rapid changes into a single rebuild.

## Medium Priority Findings

### 7) Sandbox context is not actually used in Lua runtime
**Files:** `crates/rpp/src/lua/runtime.rs`, `crates/rpp/src/plugin/lua_processor.rs`, `crates/rpp/src/sandbox/mod.rs`

**Issue:** `SandboxContext` is created in `LuaProcessor` but not passed into the Lua API. The runtime only exposes `json/hash/log`, so file APIs and dependency tracking are unused.

**Suggested fixes:**
- Expose `FileApi` methods to Lua under `ctx.file` and wire calls through `SandboxContext`.
- Capture reads/writes for cache dependency tracking.
- Add LuaLS definitions for `ctx.file` APIs in `crates/rpp-cli/src/lua_defs.rs`.

### 8) Cache dependency tracking is incomplete
**Files:** `crates/rpp/src/build/cache.rs`, `crates/rpp/src/sandbox/mod.rs`

**Issue:** `CachedFile.dependencies` exists but is never populated. Cache invalidation does not incorporate file dependencies (e.g., JSON includes, file lookups).

**Suggested fixes:**
- Populate dependencies based on `SandboxContext.read_files()` and `written_files()` during processing.
- Use dependency fingerprints or recorded hashes to validate cache entries.

### 9) Process phase error handling loses context
**Files:** `crates/rpp/src/build/process.rs`

**Issue:** When errors are present, only the first error is returned and the file path is discarded (`errors.into_iter().next().unwrap()` returns the error only). This hides the path or multiple errors.

**Suggested fixes:**
- Return the first `(path, error)` in a wrapper error with context.
- Consider aggregating errors into a single `BuildError::Worker` with a joined summary.

### 10) File watcher mixes config detection rules
**Files:** `crates/rpp-cli/src/dev_server/watcher.rs`

**Issue:** The watcher treats both `rpp.toml` and `rpp.jsonc` as config changes, but the codebase only loads `rpp.jsonc` in `rpp::RppConfig` and CLI. This can cause confusing reloads when `rpp.toml` is used (or absent).

**Suggested fixes:**
- Align on a single config filename or support both consistently.
- If both are supported, prefer one and log which config is active.

### 11) Duplicate or orphaned server implementation
**Files:** `crates/rpp-cli/src/server/mod.rs`, `crates/rpp-cli/src/server/sse.rs`

**Issue:** There is an older `viz`-based server module that is not wired into CLI. `server/sse.rs` is empty. This is dead code or leftover scaffolding.

**Suggested fixes:**
- Remove unused `server/` module or finish it and standardize on one framework.
- Ensure dependencies are only those used by the chosen server implementation.

### 12) Plugin metadata parsing executes Lua code without sandboxing
**Files:** `crates/rpp-cli/src/cli/build.rs`

**Issue:** `parse_plugin_metadata` runs the Lua file in a fresh `mlua::Lua` without sandboxing (no `os`/`io` removal). This is a potential security risk.

**Suggested fixes:**
- Use `LuaRuntime` sandboxing or a restricted Lua environment for metadata parsing.
- Alternatively, parse metadata with a dedicated lightweight parser (e.g., `toml`/`json` header comment) instead of executing the file.

## Low Priority Findings

### 13) Public API documentation gaps in CLI crate
**Files:** `crates/rpp-cli/src/config.rs`, `crates/rpp-cli/src/template.rs`, `crates/rpp-cli/src/dev_server/*.rs`

**Issue:** Public structs and enums lack `///` docs, which clashes with the project’s stated documentation standard.

**Suggested fixes:**
- Add doc comments to public structs/enums and key functions.
- Consider `#![deny(missing_docs)]` for CLI crate once docs are in place.

### 14) Regex initialization panics on invalid patterns
**Files:** `crates/rpp/src/util/regex.rs`

**Issue:** `Regex::new(...).unwrap()` will panic on invalid regex literals at runtime (unlikely but possible). These are static literals so it is probably fine, but `unwrap()` is still a panic path.

**Suggested fixes:**
- Use `expect("...valid regex...")` or a compile-time regex crate if preferred.

### 15) Config handling confusion in CLI
**Files:** `crates/rpp-cli/src/config.rs`, `crates/rpp-cli/src/cli/*`

**Issue:** CLI config structs (`Config`, `RppSection`, `ServerSection`) are unused by the current CLI commands, which rely on `rpp::RppConfig` (`rpp.jsonc`). This duplicates concepts and risks drift.

**Suggested fixes:**
- Remove unused CLI config module or integrate it fully.
- Document intended future use if you want to keep it.

## Architecture/Doc Drift

### 16) Documentation mismatches vs. current implementation
**Files:** `ARCHITECTURE.md`, `architecture/README.md`, `IMPLEMENTATION_SUMMARY.md`

**Issue:** Architecture docs reference `viz` in some places and `axum` in others; current implementation uses `axum`. `IMPLEMENTATION_SUMMARY.md` lists a `serve` command, but the CLI uses `dev`. This can confuse contributors.

**Suggested fixes:**
- Update docs to reflect `axum` and `rpp dev`.
- Ensure docs mention `rpp.jsonc` as the primary config format.

## Suggested Fix Roadmap

### Phase 1: Correctness + Safety
- Make plugin registration non-panicking (`BuildEngine`).
- Thread config into processor context.
- Fix Lua runtime concurrency (per-worker runtimes).
- Sandbox metadata parsing for Lua plugins.

### Phase 2: Performance + Dev UX
- Stream zip creation and avoid double-read in discovery.
- Add debounce to file watcher and coalesce rebuilds.

### Phase 3: API + Docs
- Expose `ctx.file` sandbox in Lua and update LuaLS definitions.
- Populate cache dependencies.
- Remove or finalize legacy `server/` module and document CLI config source.
- Add missing docs in `rpp-cli`.

## Targeted Fix Examples

### Example: Stream zip writes
**File:** `crates/rpp/src/build/output.rs`

```rust
let mut file = File::open(&path)?;
zip.start_file(name_str, *options)?;
std::io::copy(&mut file, zip)?;
```

### Example: Thread config into worker pool
**Files:** `crates/rpp/src/build/process.rs`, `crates/rpp/src/worker/pool.rs`

```rust
pub struct ProcessingJob {
    pub file: FileEntry,
    pub processors: Vec<Arc<dyn ProcessorPlugin>>,
    pub config: toml::Value,
}
```

## Notes
- No unsafe blocks found in the Rust sources.
- Most `unwrap()`/`panic!` use is in tests; production panics are primarily in Lua processor and registry mutation.
- The `ResourcePackProcessor` type in `crates/rpp/src/rpp.rs` is currently unused and empty.
