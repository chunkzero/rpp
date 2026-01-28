# RPP 2.0 Architecture Plan

## Overview

A multi-phase build pipeline with sandboxed Lua plugins that maximizes parallelism while supporting stateful operations.

## Core Architecture

### Build Pipeline Flow

```
┌────────────────────────────────────────────────────────────────────────────┐
│                           BUILD PIPELINE                                    │
│                                                                             │
│  ┌─────────────┐   ┌──────────────┐   ┌──────────────┐   ┌──────────────┐  │
│  │  DISCOVERY  │──▶│   PROCESS    │──▶│  FINALIZE    │──▶│   OUTPUT     │  │
│  │             │   │  (Parallel)  │   │(Sequential)  │   │              │  │
│  └─────────────┘   └──────────────┘   └──────────────┘   └──────────────┘  │
│                                                                             │
│  • Walk files      • Run processors    • Run generators    • Write to      │
│  • Apply filters     in parallel         sequentially        output dir    │
│  • Build index     • Chain results     • Access all        • Update cache  │
│  • Check cache       (A→B→C)             processed files                    │
│                                                                             │
└────────────────────────────────────────────────────────────────────────────┘
```

### Three-Phase Execution

#### Phase 1: Discovery (Main Thread)

```rust
pub struct DiscoveryPhase;

impl DiscoveryPhase {
    pub fn run(&self, config: &BuildConfig) -> Result<FileIndex> {
        // 1. Walk directory respecting .rppignore
        // 2. Check cache for each file
        // 3. Build FileIndex of changed files
        // 4. Return only files needing processing
    }
}

pub struct FileIndex {
    entries: Vec<FileEntry>,
    // Files that haven't changed, will be copied from cache
    cached: Vec<PathBuf>,
}

pub struct FileEntry {
    pub source_path: PathBuf,
    pub relative_path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub content: Vec<u8>,
}
```

#### Phase 2: Process (Worker Pool)

```rust
pub struct ProcessPhase;

impl ProcessPhase {
    pub fn run(&self, index: FileIndex, plugins: &PluginRegistry) -> Result<ProcessedFiles> {
        // Spawn worker threads
        // Each worker gets a slice of files
        // Processors chain: A -> B -> C
        // If any processor cancels, skip remaining chain
    }
}

pub struct ProcessingJob {
    pub file: FileEntry,
    pub processors: Vec<&ProcessorPlugin>,  // Chain of processors for this file
}

pub struct ProcessedFile {
    pub source_path: PathBuf,
    pub output_path: PathBuf,
    pub content: Vec<u8>,
    pub transformations: Vec<Transformation>,  // Track what was done
}

pub struct Transformation {
    pub processor: String,
    pub version: String,
    pub input_hash: u64,
    pub output_hash: u64,
}
```

**Processor Chaining with Cancellation:**

```rust
impl Worker {
    fn process_file(&self, job: ProcessingJob) -> Result<ProcessedFile> {
        let mut context = ProcessingContext::new(&job.file);
        
        for processor in job.processors {
            match processor.process(&context)? {
                ProcessResult::Continue(new_content) => {
                    context.update_content(new_content);
                    context.record_transformation(processor.name());
                }
                ProcessResult::Cancel => {
                    // Stop processing this file, don't include in output
                    return Ok(ProcessedFile::cancelled(job.file.source_path));
                }
                ProcessResult::Skip => {
                    // Skip this processor, continue with next
                    continue;
                }
            }
        }
        
        Ok(context.into_processed_file())
    }
}
```

#### Phase 3: Finalize (Main Thread)

```rust
pub struct FinalizePhase;

impl FinalizePhase {
    pub fn run(&self, processed: ProcessedFiles, plugins: &PluginRegistry) -> Result<BuildResult> {
        // 1. Run generators (sequential, have access to all processed files)
        let generated = self.run_generators(&processed, plugins.generators())?;
        
        // 2. Run validators (sequential)
        self.run_validators(&processed, &generated, plugins.validators())?;
        
        // 3. Merge processed + generated
        // 4. Write to output directory
        // 5. Update cache
        
        Ok(BuildResult {
            files_written: ...,
            files_skipped: ...,
            duration: ...,
        })
    }
}

pub struct GeneratorContext<'a> {
    pub processed_files: &'a [ProcessedFile],
    pub config: &'a PluginConfig,
    // Sandboxed file operations
    pub file_api: FileApi,
}

pub trait GeneratorPlugin: Plugin {
    fn generate(&self, ctx: &GeneratorContext) -> Result<Vec<GeneratedFile>>;
}
```

## Plugin System

### Plugin Types

```rust
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn plugin_type(&self) -> PluginType;
}

pub enum PluginType {
    Processor(ProcessorDef),
    Generator(GeneratorDef),
    Filter(FilterDef),
    Validator(ValidatorDef),
}

pub struct ProcessorDef {
    /// File patterns this processor handles (glob syntax)
    pub patterns: Vec<String>,
    /// Run order - lower runs first
    pub priority: i32,
}

pub struct GeneratorDef {
    /// Run after all processors complete
    pub run_after_processors: bool,
}

pub struct FilterDef {
    /// Patterns for files this filter applies to
    pub patterns: Vec<String>,
}

pub struct ValidatorDef;
```

### Lua Plugin API

Each Lua plugin is a module that returns a table with metadata and handler functions:

```lua
-- plugins/json_minify.lua
return {
    -- Metadata
    name = "json_minify",
    version = "1.0.0",
    type = "processor",
    
    -- Processor configuration
    patterns = {"*.json", "**/lang/*.json"},
    priority = 100,
    
    -- Handler function
    process = function(ctx, input)
        -- ctx provides sandboxed API
        -- input contains file metadata and content
        
        local decoded = ctx.json_decode(input.content)
        local minified = ctx.json_encode(decoded, {compact = true})
        
        -- Return result
        return {
            action = "continue",  -- "continue", "cancel", or "skip"
            content = minified,
            -- Can optionally change output path
            -- path = "alternate/path.json"
        }
    end
}
```

```lua
-- plugins/texture_atlas.lua (Generator example)
return {
    name = "texture_atlas",
    version = "1.2.0",
    type = "generator",
    
    generate = function(ctx)
        local files = {}
        
        -- Access all processed files
        for _, file in ipairs(ctx.processed_files) do
            if file.path:match("%.png$") then
                table.insert(files, file)
            end
        end
        
        -- Generate atlas
        local atlas = ctx.generate_texture_atlas(files)
        
        -- Write output using sandboxed API
        ctx.write_file("assets/minecraft/textures/atlas.png", atlas)
        
        return {
            generated_files = {"assets/minecraft/textures/atlas.png"}
        }
    end
}
```

### Sandboxed Context API

```rust
pub struct PluginContext {
    // File operations (tracked)
    pub file_api: FileApi,
    
    // Data operations
    pub json: JsonApi,
    pub toml: TomlApi,
    
    // Hashing functions
    pub hash: HashApi,
    
    // Utility
    pub log: LogApi,
    
    // Plugin config from rpp.toml
    pub config: PluginConfig,
}

pub struct FileApi;

impl FileApi {
    /// Read a file from the source directory (read-only)
    pub fn read_source(&self, path: &str) -> Result<Vec<u8>>;
    
    /// Read a file from the output directory (read-only)
    pub fn read_output(&self, path: &str) -> Result<Vec<u8>>;
    
    /// Write a file to the output directory
    /// Automatically tracked for cache invalidation
    pub fn write_file(&self, path: &str, content: Vec<u8>) -> Result<()>;
    
    /// Delete a file from the output
    pub fn delete_file(&self, path: &str) -> Result<()>;
    
    /// Check if file exists
    pub fn exists(&self, path: &str) -> bool;
    
    /// Get file metadata
    pub fn metadata(&self, path: &str) -> Result<FileMetadata>;
}

pub struct HashApi;

impl HashApi {
    /// xxhash3_64 (fast, non-cryptographic)
    pub fn xxhash3(&self, data: &[u8]) -> u64;
    
    /// SHA-256 (cryptographic)
    pub fn sha256(&self, data: &[u8]) -> String;
    
    /// MD5 (for compatibility)
    pub fn md5(&self, data: &[u8]) -> String;
}
```

## Worker Pool Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Main Thread                              │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │
│  │  Job Queue   │  │  Plugin      │  │  Result Collector    │   │
│  │  (mpsc)      │  │  Registry    │  │  (mpsc)              │   │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘   │
│         │                 │                      │               │
│         └─────────────────┴──────────────────────┘               │
│                           │                                      │
│                           ▼                                      │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Worker Threads (N)                     │   │
│  │                                                          │   │
│  │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐  │   │
│  │  │   Worker 1  │    │   Worker 2  │    │   Worker N  │  │   │
│  │  │             │    │             │    │             │  │   │
│  │  │ ┌─────────┐ │    │ ┌─────────┐ │    │ ┌─────────┐ │  │   │
│  │  │ │ Lua RT  │ │    │ │ Lua RT  │ │    │ │ Lua RT  │ │  │   │
│  │  │ │(loaded  │ │    │ │(loaded  │ │    │ │(loaded  │ │  │   │
│  │  │ │plugins) │ │    │ │plugins) │ │    │ │plugins) │ │  │   │
│  │  │ └─────────┘ │    │ └─────────┘ │    │ └─────────┘ │  │   │
│  │  └─────────────┘    └─────────────┘    └─────────────┘  │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

```rust
pub struct WorkerPool {
    workers: Vec<WorkerHandle>,
    job_sender: mpsc::Sender<ProcessingJob>,
    result_receiver: mpsc::Receiver<ProcessingResult>,
}

impl WorkerPool {
    pub fn new(
        num_workers: usize,
        plugins: Arc<PluginRegistry>,
    ) -> Self {
        let (job_tx, job_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        
        let job_rx = Arc::new(Mutex::new(job_rx));
        
        let workers: Vec<_> = (0..num_workers)
            .map(|id| {
                Worker::spawn(
                    id,
                    Arc::clone(&job_rx),
                    result_tx.clone(),
                    Arc::clone(&plugins),
                )
            })
            .collect();
        
        Self {
            workers,
            job_sender: job_tx,
            result_receiver: result_rx,
        }
    }
}

pub struct Worker {
    id: usize,
    lua: LuaRuntime,  // Each worker has its own Lua instance
}

impl Worker {
    fn spawn(
        id: usize,
        job_queue: Arc<Mutex<mpsc::Receiver<ProcessingJob>>>,
        result_sender: mpsc::Sender<ProcessingResult>,
        plugins: Arc<PluginRegistry>,
    ) -> WorkerHandle {
        thread::spawn(move || {
            // Initialize Lua runtime
            let mut worker = Self {
                id,
                lua: LuaRuntime::new(),
            };
            
            // Load all processor plugins into this Lua instance
            for plugin in plugins.processors() {
                worker.lua.load_plugin(plugin).unwrap();
            }
            
            // Process jobs
            loop {
                let job = match job_queue.lock().unwrap().recv() {
                    Ok(job) => job,
                    Err(_) => break,  // Channel closed
                };
                
                let result = worker.process(job);
                result_sender.send(result).unwrap();
            }
        })
    }
}
```

## Cache Strategy

```rust
pub struct BuildCache {
    /// File fingerprints for incremental builds
    files: HashMap<PathBuf, CachedFile>,
    
    /// When the cache was created
    created_at: SystemTime,
    
    /// Cache format version
    version: u32,
}

pub struct CachedFile {
    /// Source file fingerprint
    fingerprint: FileFingerprint,
    
    /// Output file path (relative to output dir)
    output_path: PathBuf,
    
    /// Content hash (xxhash3)
    content_hash: u64,
    
    /// Transformations applied (processor name + version)
    transformations: Vec<TransformationRecord>,
    
    /// Dependencies on other files (for cache invalidation)
    dependencies: Vec<PathBuf>,
}

pub struct FileFingerprint {
    mtime: u64,
    size: u64,
    hash: u64,  // xxhash3 of content
}

impl BuildCache {
    /// Check if we can use cached version
    pub fn is_valid(&self, path: &Path, current: &FileFingerprint) -> bool {
        let Some(cached) = self.files.get(path) else {
            return false;
        };
        
        // Check if file changed
        if cached.fingerprint != *current {
            return false;
        }
        
        // Check if any processors changed versions
        for tx in &cached.transformations {
            let plugin_version = get_plugin_version(&tx.processor);
            if plugin_version != tx.version {
                return false;
            }
        }
        
        // Check if dependencies changed
        for dep in &cached.dependencies {
            let dep_current = fingerprint_file(dep);
            let dep_cached = self.files.get(dep);
            
            match dep_cached {
                Some(c) if c.fingerprint == dep_current => continue,
                _ => return false,
            }
        }
        
        true
    }
}
```

Cache invalidation rules:
1. **Source file changes** (mtime, size, or content hash)
2. **Processor version changes** (plugin updated)
3. **Dependency changes** (generator depends on processed file)
4. **Config changes** (rpp.toml modified)

## Dev Server with Hot Reload

```
┌──────────────────────────────────────────────────────────────────┐
│                         Dev Server                                │
│                                                                   │
│  ┌──────────────┐                                                │
│  │ File Watcher │◀── Watches: source files, plugin files, config │
│  │   (notify)   │                                                │
│  └──────┬───────┘                                                │
│         │                                                         │
│         │ File changed                                            │
│         ▼                                                         │
│  ┌──────────────┐                                                │
│  │  Change      │── Determine what changed:                      │
│  │  Handler     │   - Source file → incremental rebuild          │
│  │              │   - Plugin file → reload plugin + rebuild      │
│  │              │   - Config → full rebuild                      │
│  └──────┬───────┘                                                │
│         │                                                         │
│         │ Trigger rebuild                                         │
│         ▼                                                         │
│  ┌──────────────┐                                                │
│  │   Build      │── Uses cache for unchanged files               │
│  │   Engine     │                                                │
│  └──────┬───────┘                                                │
│         │                                                         │
│         │ Changes detected                                        │
│         ▼                                                         │
│  ┌──────────────┐                                                │
│  │  SSE Channel │── Broadcast to all connected browsers          │
│  │              │   {type: "reload", files: [...]}               │
│  └──────────────┘                                                │
│                                                                   │
│  ┌──────────────┐                                                │
│  │ HTTP Server  │── Serves static files from output directory    │
│  │  (viz/axum)  │                                                │
│  └──────────────┘                                                │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

```rust
pub struct DevServer {
    build_engine: BuildEngine,
    file_watcher: notify::RecommendedWatcher,
    clients: Arc<Mutex<Vec<SseClient>>>,
    plugin_hot_reload: bool,
}

impl DevServer {
    pub async fn run(&mut self, config: ServerConfig) -> Result<()> {
        // Watch source directory
        self.file_watcher.watch(&config.source_dir, RecursiveMode::Recursive)?;
        
        // Watch plugin directory
        if self.plugin_hot_reload {
            self.file_watcher.watch(&config.plugin_dir, RecursiveMode::Recursive)?;
        }
        
        // Start HTTP server
        let app = Router::new()
            .route("/", get(serve_index))
            .route("/*path", get(serve_static))
            .route("/events", get(sse_handler));
        
        // Handle file change events
        while let Ok(event) = self.file_watcher_rx.recv() {
            match event {
                DebouncedEvent::Write(path) | DebouncedEvent::Create(path) => {
                    if self.is_plugin_file(&path) {
                        // Hot reload plugin
                        self.reload_plugin(&path).await?;
                        // Trigger rebuild
                        self.trigger_rebuild().await?;
                    } else {
                        // Incremental rebuild
                        self.rebuild_file(&path).await?;
                    }
                }
                _ => {}
            }
        }
        
        Ok(())
    }
    
    async fn reload_plugin(&self, path: &Path) -> Result<()> {
        // 1. Unload old plugin
        // 2. Load new plugin code
        // 3. Validate plugin
        // 4. Update plugin registry
        // 5. Broadcast reload to all workers
        
        info!("Hot reloading plugin: {}", path.display());
        
        self.build_engine.reload_plugin(path).await?;
        self.broadcast(Event::PluginReloaded { path: path.to_path_buf() }).await?;
        
        Ok(())
    }
}
```

## Transformation Tracking

Every file operation is tracked for debugging and caching:

```rust
pub struct TransformationLog {
    entries: Vec<TransformationEntry>,
}

pub struct TransformationEntry {
    pub timestamp: SystemTime,
    pub file: PathBuf,
    pub operation: Operation,
    pub plugin: String,
    pub details: String,
}

pub enum Operation {
    Read,
    Write,
    Delete,
    Process { from_hash: u64, to_hash: u64 },
    Generate,
    Skip,
    Cancel,
}

// Example log output:
// [2024-01-28T10:30:00Z] assets/minecraft/models/item/diamond_sword.json
//   ├─ Read (source)
//   ├─ Process [json_minify@v1.0.0] 2,456 bytes → 1,892 bytes
//   ├─ Process [optimize_names@v2.1.0] 1,892 bytes → 1,734 bytes
//   └─ Write (output) 1,734 bytes
//
// [2024-01-28T10:30:01Z] assets/minecraft/textures/atlas.png
//   └─ Generate [texture_atlas@v1.2.0] from 47 textures → 1 atlas
```

## Implementation Phases

### Phase 1: Core Foundation
- [ ] New module structure (discovery, process, finalize)
- [ ] Basic worker pool
- [ ] Sandboxed Lua runtime
- [ ] File API (read/write/delete)
- [ ] Simple processor plugins
- [ ] Basic caching

### Phase 2: Plugin System
- [ ] All plugin types (processor, generator, filter, validator)
- [ ] Processor chaining
- [ ] Cancellation support
- [ ] Transformation tracking
- [ ] JSON/TOML APIs
- [ ] Hashing functions

### Phase 3: Dev Server
- [ ] HTTP server with static file serving
- [ ] SSE for live reload
- [ ] File watching
- [ ] Incremental rebuilds
- [ ] Plugin hot reload

### Phase 4: Polish
- [ ] Error handling and reporting
- [ ] Logging and diagnostics
- [ ] Performance optimization
- [ ] Cache compression
- [ ] Parallel generator execution (if possible)

## Migration from Current Code

| Current | New | Notes |
|---------|-----|-------|
| `compile/` | `build/` | Rename to reflect broader scope |
| `PackCompiler` | `BuildEngine` | Orchestrates all phases |
| `WorkerPool` | `WorkerPool` | Keep but refactor for new context system |
| `EventHandler` | `ProcessorPlugin` | Different trait design |
| `RppLua` | `LuaRuntime` | Sandboxed with context |
| `PluginLoader` | `PluginRegistry` | Manages all plugin types |
| `lua/handler.rs` | `sandbox/` | New sandboxed API implementation |
| `server/` | `dev_server/` | Rename and expand |
| `Cache` | `BuildCache` | Enhanced with transformation tracking |

## Key Files to Create/Modify

```
crates/rpp/src/
├── build/
│   ├── mod.rs           # BuildEngine
│   ├── discovery.rs     # File walking and indexing
│   ├── process.rs       # Worker pool and processing
│   ├── finalize.rs      # Generators and validators
│   ├── cache.rs         # Caching logic
│   └── result.rs        # BuildResult, ProcessingResult
├── plugin/
│   ├── mod.rs           # Plugin trait, PluginRegistry
│   ├── processor.rs     # ProcessorPlugin trait
│   ├── generator.rs     # GeneratorPlugin trait
│   ├── filter.rs        # FilterPlugin trait
│   ├── validator.rs     # ValidatorPlugin trait
│   └── loader.rs        # Lua plugin loading
├── sandbox/
│   ├── mod.rs           # PluginContext
│   ├── file_api.rs      # Sandboxed file operations
│   ├── hash_api.rs      # Hash functions
│   ├── json_api.rs      # JSON encode/decode
│   └── log_api.rs       # Logging
├── worker/
│   ├── mod.rs           # WorkerPool
│   ├── worker.rs        # Worker implementation
│   └── runtime.rs       # LuaRuntime per worker
└── lib.rs               # Public exports

crates/rpp-cli/src/
├── dev_server/
│   ├── mod.rs           # DevServer
│   ├── watcher.rs       # File watching
│   └── sse.rs           # SSE broadcasting
└── cli/
    └── build.rs         # Updated to use new BuildEngine
```

## Testing Strategy

```rust
// Unit tests for each component
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_processor_chaining() {
        let worker = TestWorker::new();
        let file = test_file!(r#"{"key": "value"}"#);
        
        let processors = vec![
            test_processor!(json_minify),
            test_processor!(optimize_names),
        ];
        
        let result = worker.process(file, processors);
        
        assert_eq!(result.transformations.len(), 2);
        assert!(result.content.len() < file.content.len());
    }
    
    #[test]
    fn test_cancellation() {
        let worker = TestWorker::new();
        let file = test_file!("skip_this.txt");
        
        let processors = vec![
            test_processor!(canceller),
            test_processor!(should_not_run),
        ];
        
        let result = worker.process(file, processors);
        
        assert!(result.is_cancelled());
        assert_eq!(result.transformations.len(), 1); // Only first ran
    }
    
    #[test]
    fn test_sandboxed_file_api() {
        let ctx = PluginContext::test();
        
        // Can write to output
        ctx.file_api.write_file("test.txt", b"hello").unwrap();
        
        // Cannot write outside output dir
        let result = ctx.file_api.write_file("../outside.txt", b"bad");
        assert!(result.is_err());
    }
}
```

## Performance Considerations

1. **Memory**: Each worker has its own Lua runtime (~5-10MB each)
   - Limit worker count based on available memory
   - Consider worker pools for large projects

2. **Parallelism**: 
   - Processors run in parallel across files
   - Generators run sequentially (access all files)
   - I/O operations can be batched

3. **Cache**:
   - Use xxhash3 for fast hashing
   - Cache file fingerprints in memory during build
   - Persist cache to disk after build

4. **Hot Reload**:
   - Don't rebuild full plugin registry on every change
   - Only reload changed plugins
   - Validate plugin before swapping

## Open Questions

1. **Generator Parallelism**: Can we run independent generators in parallel?
2. **Streaming**: Should large files be processed as streams instead of loading entirely into memory?
3. **Plugin Distribution**: How will users share/distribute plugins?
4. **Debugging**: How can users debug their Lua plugins?

---

**Ready for review!** This architecture addresses all requirements:
- ✅ Sandboxed Lua with file API
- ✅ Processor chaining with cancellation
- ✅ Hashing functions available
- ✅ Generators access all processed files
- ✅ Hot reload for dev server
- ✅ Transformation tracking throughout
