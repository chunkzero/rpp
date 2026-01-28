# ARCHITECTURE.md Errors and Fixes

Errors identified in the original ARCHITECTURE.md document.

## Critical Errors

### 1. Channel Pattern Error (lines 362-366)

**Issue**: Document uses `mpsc::channel()` with `Arc<Mutex<mpsc::Receiver>>` to share receiver across workers.

**Problem**: `std::sync::mpsc::Receiver` is not designed for multiple consumers. Using a Mutex to share it defeats the purpose of the channel pattern.

**Fix**: Use `crossbeam_channel` (MPMC) as the current codebase already does:
```rust
let (job_tx, job_rx) = crossbeam_channel::unbounded();
let job_rx = Arc::new(job_rx); // crossbeam Receiver is Clone
```

### 2. Web Framework Confusion (line 540)

**Issue**: Document says "(viz/axum)" but these are different frameworks.

**Fix**: Use `viz` only - it's already a dependency in the CLI crate.

### 3. Old notify Crate API (line 573)

**Issue**: Uses `DebouncedEvent` which is from notify v4.x.

**Fix**: Current dependencies use notify v5+ which has:
```rust
// Old (v4)
DebouncedEvent::Write(path)

// New (v5)
Event { kind: EventKind::Modify(_), paths, .. }
```

### 4. DevServer Struct Missing Field (line 571)

**Issue**: Uses `self.file_watcher_rx` but struct definition doesn't include this field.

**Fix**: Add the field:
```rust
pub struct DevServer {
    // ...existing fields...
    change_rx: tokio::sync::mpsc::UnboundedReceiver<WatchEvent>,
}
```

### 5. Async/Sync Mixing (line 571)

**Issue**: Uses sync `recv()` call inside an async function.

**Fix**: Use tokio channels or spawn_blocking:
```rust
// Option A: Use tokio channels
let (watch_tx, mut watch_rx) = tokio::sync::mpsc::unbounded_channel();
while let Some(event) = watch_rx.recv().await { ... }

// Option B: Spawn blocking
tokio::task::spawn_blocking(move || { ... })
```

## Design Inconsistencies

### 6. Reference to Trait Object (line 73-74)

**Issue**: `Vec<&ProcessorPlugin>` - reference to unsized trait.

**Fix**: Use `Vec<Arc<dyn ProcessorPlugin>>` for thread safety and proper sizing.

### 7. Undefined Types

**Issue**: Several types referenced but never defined:
- `PluginRegistry`
- `LuaRuntime`
- `ProcessingResult`
- `WorkerHandle`

**Fix**: Define all types in their respective modules (see commit plans).

### 8. Undefined Functions in Cache Logic (lines 476, 484)

**Issue**:
- `get_plugin_version(&tx.processor)` - not defined
- `fingerprint_file(dep)` - not defined

**Fix**: Use `PluginRegistry::plugin_version()` and compute fingerprint inline:
```rust
// Instead of get_plugin_version
registry.plugin_version(&tx.processor)

// Instead of fingerprint_file
Fingerprint::compute(path)
```

### 9. Incomplete Placeholder Code (line 138-142)

**Issue**:
```rust
BuildResult {
    files_written: ...,
    files_skipped: ...,
    duration: ...,
}
```

**Fix**: Define complete struct:
```rust
pub struct BuildResult {
    pub files_processed: usize,
    pub files_cached: usize,
    pub files_generated: usize,
    pub files_cancelled: usize,
    pub duration: Duration,
}
```

### 10. mlua Send Feature

**Issue**: Document implies shared Lua state across threads.

**Fix**: Not needed - each worker loads plugins fresh, avoiding cross-thread Lua issues entirely.

## Thread Safety Issues

### 11. Plugin Trait Send + Sync

**Issue**: `Plugin: Send + Sync` but Lua plugins need special handling.

**Fix**: `LuaProcessor` uses `Mutex<Option<LuaRuntime>>` for lazy per-thread initialization:
```rust
pub struct LuaProcessor {
    // ...
    runtime: Mutex<Option<LuaRuntime>>,
}
```

## Current Code Issues (Not in ARCHITECTURE.md)

### Error Handling in Workers

**Current**: Silent drops with `let _ = handler.handle_event(...)`

**Fix**: Propagate errors through `ProcessingResult::Error`.

### Cache Not Persisted

**Current**: Cache loaded but never saved to disk.

**Fix**: Call `cache.save()` after successful build.
