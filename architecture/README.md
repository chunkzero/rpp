# RPP 2.0 Architecture Implementation Plan

A multi-phase build pipeline with sandboxed Lua plugins that maximizes parallelism while supporting stateful operations.

## Design Decisions

| Decision | Choice |
|----------|--------|
| Event Model | New processor model (`ProcessResult::Continue/Cancel/Skip`) |
| Plugin Scope | Processor + Generator only (Filter/Validator later) |
| Output Strategy | Separate output directory (`./dist/` or configurable) |
| Lua Threading | Each worker loads plugins fresh |
| Migration | Clean break - remove old `EventHandler` API |
| Lua API Style | Nested tables (`ctx.json.encode()`, `ctx.hash.xxhash3()`) |

## Commit Plan

| Commit | Description | Files |
|--------|-------------|-------|
| 1 | [Core Types Foundation](./01-core-types.md) | `build/`, `plugin/` types and traits |
| 2 | [Sandbox API](./02-sandbox-api.md) | `sandbox/` file, hash, json, log APIs |
| 3 | [Plugin Registry](./03-plugin-registry.md) | Plugin matching and ordering |
| 4 | [Discovery Phase](./04-discovery-phase.md) | File walking, fingerprinting, cache check |
| 5 | [Worker Pool](./05-worker-pool.md) | Parallel processing, job/result channels |
| 6 | [Finalize Phase](./06-finalize-phase.md) | Generators, output writing |
| 7 | [Build Engine](./07-build-engine.md) | Main orchestrator, builder pattern |
| 8 | [Lua Runtime](./08-lua-runtime.md) | Lua plugin loading and execution |
| 9 | [Dev Server](./09-dev-server.md) | HTTP server, file watching, SSE |
| 10 | [CLI Integration](./10-cli-integration.md) | Wire up CLI, remove old code |

## Architecture Errors Fixed

See [errors.md](./errors.md) for the list of errors identified in the original ARCHITECTURE.md.

## Module Structure

```
crates/rpp/src/
├── build/
│   ├── mod.rs           # BuildEngine
│   ├── types.rs         # FileEntry, ProcessedFile, BuildResult
│   ├── error.rs         # BuildError
│   ├── discovery.rs     # DiscoveryPhase
│   ├── process.rs       # ProcessPhase
│   ├── finalize.rs      # FinalizePhase
│   ├── output.rs        # OutputWriter
│   ├── cache.rs         # BuildCache
│   └── engine.rs        # BuildEngine, BuildEngineBuilder
├── plugin/
│   ├── mod.rs           # Plugin trait
│   ├── processor.rs     # ProcessorPlugin, ProcessResult
│   ├── generator.rs     # GeneratorPlugin
│   ├── registry.rs      # PluginRegistry
│   └── lua_processor.rs # LuaProcessor
├── sandbox/
│   ├── mod.rs           # SandboxContext
│   ├── file_api.rs      # FileApi
│   ├── hash_api.rs      # HashApi
│   ├── json_api.rs      # JsonApi
│   └── log_api.rs       # LogApi
├── worker/
│   ├── mod.rs           # WorkerPool exports
│   ├── pool.rs          # WorkerPool implementation
│   └── job.rs           # ProcessingJob, ProcessingResult
├── lua/
│   └── runtime.rs       # LuaRuntime
└── lib.rs               # Public exports

crates/rpp-cli/src/
├── dev_server/
│   ├── mod.rs           # DevServer
│   ├── watcher.rs       # FileWatcher
│   └── sse.rs           # SSE broadcaster
└── cli/
    └── build.rs         # Updated BuildCommand
```

## Verification

```bash
# After each commit, verify with:
cargo check -p rpp
cargo test -p rpp

# Full integration test after commit 10:
cargo run -p rpp-cli -- build ./examples/sample_pack
```

## Dependencies

```toml
# crates/rpp/Cargo.toml additions
thiserror = "2.0"
sha2 = "0.10"
md5 = "0.7"
glob = "0.3"

# crates/rpp-cli/Cargo.toml additions
mime_guess = "2.0"
```
