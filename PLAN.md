# RPP Architecture Improvement Plan

Based on thorough analysis of the RPP (Resource Pack Processor) codebase, this document outlines critical architectural improvements needed.

## Critical Issues

### 1. Unimplemented Core Features

| Component | Location | Issue | Impact |
|-----------|----------|-------|--------|
| LuaProcessor | `lua/handler.rs` | Completely empty - all methods are `todo!()` | Lua plugins cannot integrate with compile system |
| BuildCommand | `cli/build.rs` | Only logs "Building", doesn't invoke PackCompiler | Build command does nothing |
| Cache Persistence | `compile/mod.rs` | Cache loaded but never saved | Incremental builds broken across runs |

### 2. Error Handling Gaps

- **Worker threads silently drop all errors** (`compile/worker.rs:74-76`)
  - Uses `let _ =` to ignore Result types
  - File read failures, event handler errors are invisible
  
- **Plugin unloading undefined behavior** (`lua/manager.rs:161-164`)
  - Warns about UB but doesn't prevent it
  - No cleanup mechanism for loaded plugins

### 3. Design Mismatches

- **Thread-per-handler vs Lua single-threadedness**
  - Worker pool creates EventHandlers per-thread
  - Lua is inherently single-threaded
  - Architecture may not work as designed

- **Unused abstractions**
  - `FileProcessContext` trait defined but never used
  - `ResourcePackProcessor` struct is empty
  - Suggests unclear or incomplete architecture

## Architectural Improvements

### 1. Separate Plugin Runtime from Compile System

**Current:** Lua plugins integrate directly with compile event system via EventHandler trait

**Proposed:** 
- Dedicated plugin runtime in separate thread/process
- Message-passing architecture between compiler and plugins
- Solves thread-safety issues with Lua

**Benefits:**
- Lua can run single-threaded as designed
- Compiler doesn't block on plugin execution
- Better isolation and error handling

### 2. Add Proper Dependency Injection

**Current:** `EventHandlerProvider` creates handlers per-thread

**Proposed:**
```rust
// Plugin registry pattern
pub struct PluginRegistry {
    handlers: Vec<Box<dyn EventHandler>>,
}

impl PluginRegistry {
    pub fn register(&mut self, handler: Box<dyn EventHandler>) {
        self.handlers.push(handler);
    }
    
    pub fn dispatch(&self, event: &BuildEvent) -> Result<()> {
        for handler in &self.handlers {
            handler.handle(event)?;
        }
        Ok(())
    }
}
```

**Benefits:**
- Clearer ownership model
- Easier to test
- Better error propagation

### 3. Implement Missing Glue Code

Priority implementation order:

1. **Connect BuildCommand → PackCompiler**
   - Parse config file
   - Initialize compiler with correct paths
   - Invoke build and handle results

2. **Implement LuaProcessor**
   - Bridge EventHandler trait to Lua functions
   - Load and execute Lua plugins
   - Handle Lua errors gracefully

3. **Add Cache Persistence**
   - Save cache after successful build
   - Handle cache corruption gracefully
   - Consider atomic writes

### 4. Add Test Infrastructure

**Current state:** Zero tests exist

**Needed:**
- Unit tests for cache logic
- Worker pool tests
- Plugin loading tests
- Integration tests for full build pipeline
- Mock file system for testing

### 5. Simplify or Remove Unused Abstractions

**Actions:**
- [ ] Implement `FileProcessContext` or remove it
- [ ] Clarify `ResourcePackProcessor` vs `PackCompiler` roles
- [ ] Remove duplicate imports in `lua/manager.rs`
- [ ] Clean up unused traits and types

## Implementation Phases

### Phase 1: Critical Fixes (Immediate)
- [ ] Fix BuildCommand to invoke PackCompiler
- [ ] Add cache persistence
- [ ] Add error propagation in worker threads
- [ ] Remove/fix undefined behavior in plugin unloading

### Phase 2: Core Features (Short-term)
- [ ] Implement LuaProcessor bridge
- [ ] Add basic test coverage
- [ ] Implement file operations in BuildContext
- [ ] Complete dev server implementation

### Phase 3: Architecture Improvements (Medium-term)
- [ ] Separate plugin runtime
- [ ] Refactor to plugin registry pattern
- [ ] Add comprehensive error handling
- [ ] Performance optimization

### Phase 4: Polish (Long-term)
- [ ] Complete plugin install/remove commands
- [ ] Add more examples
- [ ] Documentation
- [ ] Benchmarking

## Code Quality Issues

### Minor Issues Found

1. **Duplicate imports** (`lua/manager.rs:1-3`)
   ```rust
   use std::{collections::HashMap, path::PathBuf};
   use std::collections::HashMap;  // Duplicate
   ```

2. **Mix of anyhow and thiserror**
   - CLI uses anyhow
   - Library uses thiserror
   - Config has TODO to replace anyhow

3. **Print statements instead of logging**
   - `lua/manager.rs` uses `println!`
   - Should use `tracing` crate (already a dependency)

4. **Empty implementations**
   - `server/sse.rs` is empty
   - `lua/core/lib/` is empty
   - `rpp.rs` contains empty struct

## Positive Patterns to Maintain

1. **Builder Pattern** - Used consistently throughout
2. **Feature Flags** - Proper Cargo feature usage
3. **Error Types** - thiserror used well
4. **Workspace Structure** - Clean separation of concerns
5. **Event-Driven Design** - Good abstraction for plugins
6. **Parallel Processing** - Worker pool design is sound

## Notes

- Project is at version 0.1.0-alpha.0
- Approximately 2,035 lines of Rust code
- Uses mlua with vendored LuaJIT
- Cache uses bincode with big-endian encoding
- Supports `.rppignore` files for custom ignore patterns
