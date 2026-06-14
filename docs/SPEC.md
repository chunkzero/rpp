# RPP Rework Specification (v2)

This is the **authoritative spec** for the RPP rework. All implementation agents build
against this document. Where this spec conflicts with existing code or older docs
(`PLAN.md`, `ARCHITECTURE.md`, `architecture/`, `IMPLEMENTATION_SUMMARY.md`,
`CODE_REVIEW_REPORT.md`), **this spec wins** — the older docs are stale and will be
deleted at the end of the rework.

RPP is a build tool for Minecraft resource packs: it takes a source directory, runs it
through a plugin pipeline (Lua and WASM plugins), and produces an optimized output
directory and a distributable `.zip`.

## Goals

1. A pleasant, well-designed **Lua plugin system** (package-based, sandboxed, Lua 5.4).
2. A **WASM (WASIp2 component) plugin system** via wasmtime for heavier plugins.
3. **Plugin distribution & discovery via GitHub repos** with a lockfile.
4. **Incremental compilation**: content-addressed output cache; rebuilds touch only
   changed files.
5. **Pack squashing**: built-in lossless optimization (JSON minify, PNG optimization,
   deterministic optimized zip) plus optional delegation to an external PackSquash
   binary.
6. Real, working examples: an actual resource pack and actual plugins, built end-to-end
   by `rpp build`.

## Workspace layout & crate ownership

```
crates/
  rpp/          # core library: config, plugin model, lua runtime, build engine, cache
  rpp-fetch/    # plugin source resolution: github fetch, cache, lockfile, search
  rpp-squash/   # pack optimization: json minify, png optimize, zip, packsquash-extern
  rpp-wasm/     # wasmtime WASIp2 component host + WIT definitions
  rpp-cli/      # the `rpp` binary
examples/
  pack/         # a real, complete example resource pack project (rpp.toml, src/, plugins/)
  plugins/      # standalone example plugin packages (lua + wasm guest crate)
docs/           # SPEC.md (this file), plugin authoring guides
```

Dependency direction: `rpp-fetch`, `rpp-squash`, `rpp-wasm` are **standalone** (they do
NOT depend on `rpp`). `rpp` optionally depends on `rpp-wasm` (feature `wasm`) via an
adapter module. `rpp-cli` depends on all of them.

Code style: follow `AGENTS.md` (thiserror in libraries, anyhow in CLI, tracing for
logs, builder patterns, rustfmt). Every crate must pass `cargo test`,
`cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`.

---

## 1. Project configuration: `rpp.toml`

TOML is the **only** project config format (the jsonc config is removed).

```toml
[pack]
name = "my-pack"                 # used for zip filename; required
description = "An example pack"  # written into pack.mcmeta if generate_mcmeta = true
pack_format = 34                 # optional; validated against src/pack.mcmeta if present

[build]
source = "src"                   # pack source dir (contains pack.mcmeta, assets/)
output = "dist"                  # output dir; zip goes to dist/<name>.zip
workers = 0                      # 0 = available_parallelism

[build.squash]
enabled = true
engine = "builtin"               # "builtin" | "packsquash"
json = true                      # minify .json/.mcmeta in output
png = "fast"                     # false | "fast" | "max" (oxipng levels)
zip = true                       # produce dist/<name>.zip
strip = ["**/.DS_Store", "**/Thumbs.db", "**/*.psd", "**/*.xcf"]
packsquash_binary = "packsquash"        # used when engine = "packsquash"
packsquash_options = "packsquash.toml"  # optional passthrough options file

[dev]
host = "127.0.0.1"
port = 8080
open = false

# Ordered plugin list. Order = tie-break order for processor priority.
[[plugin]]
source = "path:plugins/json-minify"      # local plugin package dir
[plugin.options]                          # arbitrary; passed to the plugin
pretty = false

[[plugin]]
source = "github:example/rpp-plugins"    # remote repo
ref = "v1.2.0"                            # optional tag/branch/sha; default: default branch
subdir = "plugins/atlas"                  # optional path within the repo
```

`source` grammar:
- `path:<relative-or-absolute-dir>` — local plugin package directory.
- `github:<owner>/<repo>` — GitHub repository (optionally with `ref` and `subdir` keys).

The CLI also accepts a bare plugin package directory for `rpp plugin add` and
normalizes it to a `path:` source.

### User-level plugins

`rpp plugin add` prompts whether to install into the current project or globally
for the current user. `--project` and `--global` select the scope non-interactively.
Global plugin entries live in `~/.rpp/plugins.toml`, with GitHub pins in
`~/.rpp/plugins.lock`. Directory sources are copied into `~/.rpp/plugins/<id>` so
they can be used from unrelated projects without retaining a relative source path.

Global plugins are loaded before project plugins. A project plugin with the same
plugin id overrides the global plugin.

## 2. Plugin manifest: `plugin.toml`

A **plugin is a directory** ("plugin package") containing `plugin.toml`:

```toml
[plugin]
id = "json-minify"          # ^[a-z0-9][a-z0-9_-]*$ ; unique within a project
version = "1.2.0"           # semver
description = "Minifies JSON files"
authors = ["someone"]
runtime = "lua"             # "lua" | "wasm"
entry = "init.lua"          # lua: entry script relative to plugin root (default "init.lua")
# module = "plugin.wasm"    # wasm: component file relative to plugin root (required for wasm)
```

## 3. Core plugin model (in `crates/rpp`)

The build pipeline is runtime-agnostic. Core types (names are normative; signatures may
be refined as long as semantics hold):

```rust
/// A file flowing through the pipeline. Paths are relative, forward-slash.
pub struct PackFile { pub path: String, pub contents: Vec<u8> }

pub enum ProcessOutcome { Unchanged, Modified, Dropped }

pub struct ProcessorDef { pub name: String, pub patterns: Vec<String>, pub priority: i32 }

/// Static, shareable description + instantiation. One per configured plugin.
pub trait PluginFactory: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    /// Hash covering plugin code AND its options — feeds cache invalidation.
    fn cache_key(&self) -> u64;
    fn processors(&self) -> &[ProcessorDef];
    fn has_generator(&self) -> bool;
    /// Instantiate for one worker thread (Lua states are per-worker).
    fn instantiate(&self) -> Result<Box<dyn PluginInstance>, Error>;
}

/// A live instance bound to one thread.
pub trait PluginInstance: Send {
    /// Run one named processor over a file (mutates file in place).
    fn process(&mut self, processor: &str, file: &mut PackFile) -> Result<ProcessOutcome, Error>;
    /// Run the generator phase (sequential, after all processing).
    fn generate(&mut self, ctx: &mut dyn GeneratorHost) -> Result<(), Error>;
    fn on_build_start(&mut self) -> Result<(), Error>;
    fn on_build_finish(&mut self, stats: &BuildStats) -> Result<(), Error>;
}

/// What generators may do — implemented by the build engine.
pub trait GeneratorHost {
    fn list_files(&mut self, glob: Option<&str>) -> Vec<String>;   // recorded as dep
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;        // processed file; recorded as dep
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>>;      // raw source file; recorded as dep
    fn emit(&mut self, path: &str, contents: Vec<u8>);             // add/overwrite output file
    fn remove(&mut self, path: &str);                              // drop an output file
}
```

Notes:
- Per-plugin **options** (from `rpp.toml [plugin.options]`) are provided to the factory
  at construction (as `toml::Value`/JSON) and exposed through the runtime context.
  `GeneratorHost` itself has no `options()` method. Options are part of `cache_key()`.
- **Processors are pure**: input = (file path, contents, options). They get NO
  filesystem access. This is what makes per-file incremental caching sound.
- **Generators** access files only through `GeneratorHost`, which records a read-set
  for incremental invalidation.
- A generator's reads observe an immutable snapshot taken before that generator starts.
  It does not read back its own mutations. Later generators observe earlier generators'
  final output.
- Generators enumerate processed output files only. Raw source files can be read by
  exact path but cannot be enumerated.
- Processor chain for a file: all matching processors across all plugins, sorted by
  `priority` ascending, ties broken by plugin order in `rpp.toml`, then by declaration
  order within a plugin. A `Dropped` outcome stops the chain and excludes the file.
- Plugin errors abort the build with a message attributing plugin id + processor +
  file path.

## 4. Lua plugin system v2 (the centerpiece)

Engine: `mlua` with **vendored Lua 5.4** (`features = ["lua54", "vendored", "serde", "send"]`)
— replaces LuaJIT (5.4 gives working memory limits and a cleaner sandbox story).

### Authoring API

`init.lua` receives a preloaded `rpp` module via `require("rpp")` and returns a plugin
built with it:

```lua
local rpp = require("rpp")
local plugin = rpp.plugin()

-- Processor: parallel, per matching file. Pure: (ctx, file) only.
plugin:processor("minify", {
    files = { "**/*.json", "**/*.mcmeta" },  -- required, glob list
    priority = 50,                            -- optional, default 0, lower runs first
}, function(ctx, file)
    local data = rpp.json.decode(file.text)
    file.text = rpp.json.encode(data)         -- mutation is what counts; no return needed
end)

-- Generator: sequential, after processing, sees all files.
plugin:generator("atlas", function(ctx)
    for _, path in ipairs(ctx:files("assets/*/textures/**/*.png")) do
        local bytes = ctx:read(path)
        -- ...
    end
    ctx:emit("assets/minecraft/atlases/blocks.json", rpp.json.encode(t))
end)

plugin:on_start(function(ctx) end)
plugin:on_finish(function(ctx, stats) end)

return plugin
```

### Semantics

- **file** (userdata) in processors: `file.path` (get/set, relative forward-slash
  string; setting it renames the output), `file.bytes` / `file.text` (get/set, both are
  Lua strings; two names for readability), `file:drop()` (exclude from output, stops
  the chain). The runtime tracks whether mutation occurred to report
  Unchanged/Modified/Dropped.
- **ctx** (processors): `ctx.options` (plugin options as Lua table), `ctx.pack`
  (`{ name, description, format }`), `ctx.log` (`debug|info|warn|error` functions).
- **ctx** (generators): everything above plus `ctx:files(glob?) -> {string}`,
  `ctx:read(path) -> string|nil` (processed output), `ctx:read_source(path) -> string|nil`,
  `ctx:emit(path, contents)`, `ctx:remove(path)`.
- Raising a Lua `error()` fails the build with plugin/processor/file attribution.

### Builtin modules (preloaded, available via `require`)

- `rpp` — root: `rpp.plugin()`, plus re-exports of the submodules below.
- `rpp.json` — `decode(str) -> value`, `encode(value, opts?) -> str` where
  `opts = { pretty = false }`; preserves JSON semantics via serde.
- `rpp.toml` — `decode(str)`, `encode(value)`.
- `rpp.hash` — `xxh3(str) -> hex string`, `sha256(str) -> hex`, `md5(str) -> hex`,
  `crc32(str) -> integer`.
- `rpp.path` — `join(...)`, `dirname(p)`, `basename(p)`, `ext(p)`, `with_ext(p, e)`,
  `match(glob, p) -> bool`.
- `rpp.log` — same functions as `ctx.log` (for module-level logging).
- `rpp.str` — `starts_with`, `ends_with`, `split(s, sep)`, `trim(s)`.

### Module resolution & sandbox

- `require(name)`: builtin `rpp*` modules; otherwise resolved **within the plugin
  package directory** (`name.lua` or `name/init.lua`, dots map to `/`). Nothing else.
  No C modules, no `package.cpath`.
- Available stdlib: `string`, `table`, `math`, `utf8`, `select`, `pairs`, `ipairs`,
  `next`, `tonumber`, `tostring`, `type`, `pcall`, `xpcall`, `error`, `assert`,
  `setmetatable`/`getmetatable`/`rawget`/`rawset`/`rawequal`/`rawlen`,
  `os.clock`/`os.time`/`os.date` only. **No** `io`, no `os.*` beyond those, no
  `load`/`loadstring`/`dofile`/`loadfile`, no `debug`, no `collectgarbage` (stub ok),
  no global `print` (map it to `rpp.log.info`).
- Each plugin gets its own environment table (`_ENV`); plugins cannot see each other's
  globals. Memory limit per Lua state (configurable, default 256 MB).

### Execution model

- One Lua state **per worker thread**; all Lua plugins for the project are loaded into
  each worker's state at pool startup (each in its own env).
- Loading = parse `plugin.toml`, run `entry` in the sandbox, collect the returned
  plugin builder's processors/generators/hooks. Processor defs (patterns/priority) are
  extracted at load time on the main thread (a validation load), then re-instantiated
  per worker via `PluginFactory::instantiate`.
- Generators and lifecycle hooks run on a single dedicated instance (main thread).
- `cache_key` for a Lua plugin: xxh3 over all `*.lua` files in the package (sorted by
  path) + `plugin.toml` + canonicalized options.

## 5. WASM plugin system (`crates/rpp-wasm`)

Host for **WASIp2 components** using `wasmtime` (component model + `wasmtime-wasi`).
Standalone crate; `rpp` wraps it behind feature `wasm` with an adapter implementing
`PluginFactory`/`PluginInstance`.

### WIT (lives at `crates/rpp-wasm/wit/plugin.wit`)

```wit
package rpp:plugin@0.1.0;

interface host {
    enum log-level { debug, info, warn, error }
    log: func(level: log-level, message: string);
    /// Generator-phase host functions (empty/no-op outside generate()):
    list-files: func(pattern: option<string>) -> list<string>;
    read-file: func(path: string) -> option<list<u8>>;
    read-source: func(path: string) -> option<list<u8>>;
    emit-file: func(path: string, contents: list<u8>);
    remove-file: func(path: string);
}

interface guest {
    record processor-def { name: string, patterns: list<string>, priority: s32 }
    record plugin-info { id: string, version: string, processors: list<processor-def>, has-generator: bool }
    record file-data { path: string, contents: list<u8> }
    variant process-result { unchanged, modified(file-data), dropped }
    /// Called once after instantiation; options-json is the plugin options as JSON.
    configure: func(options-json: string);
    get-info: func() -> plugin-info;
    process: func(processor: string, file: file-data) -> result<process-result, string>;
    generate: func() -> result<_, string>;
}

world rpp-plugin {
    import host;
    export guest;
}
```

### Host crate API (normative shape)

```rust
pub struct WasmEngine { /* wasmtime Engine, shared */ }
pub struct CompiledPlugin { /* Component + metadata, Send+Sync, cheap to instantiate */ }
pub struct WasmInstance { /* Store + bindings */ }

impl WasmEngine { pub fn new() -> Result<Self>; pub fn load(&self, wasm_path: &Path) -> Result<CompiledPlugin>; }
impl CompiledPlugin {
    pub fn info(&self) -> &PluginInfo;                     // from a validation instantiation
    pub fn instantiate(&self, options_json: &str, host: impl HostCallbacks + Send + 'static) -> Result<WasmInstance>;
}
impl WasmInstance {
    pub fn process(&mut self, processor: &str, path: &str, contents: &[u8]) -> Result<ProcessResult>;
    pub fn generate(&mut self) -> Result<()>;
}
pub trait HostCallbacks { /* mirrors `host` interface; engine wires GeneratorHost here */ }
```

- WASI context: no filesystem preopens, no network, no env — stdout/stderr inherited
  for debugging. Epoch-based interruption with a configurable deadline (default 60s per
  call).
- Resource limits: memory cap via `StoreLimits` (default 512 MB).
- Includes an **example guest plugin** in `examples/plugins/grayscale-wasm/`: a Rust
  crate (NOT a workspace member; own `Cargo.toml`) using `wit-bindgen`, targeting
  `wasm32-wasip2`, implementing a simple processor (e.g. converts `**/*.gray.png`
  textures to grayscale, or a CSV→JSON transformer — something visibly real).

## 6. Plugin fetch & discovery (`crates/rpp-fetch`)

Standalone, blocking (`ureq`), no async.

### Resolution

```rust
pub enum PluginSource { Path { dir: PathBuf }, GitHub { owner: String, repo: String, ref_: Option<String>, subdir: Option<String> } }
impl PluginSource { pub fn parse(s: &str, ref_: Option<&str>, subdir: Option<&str>) -> Result<Self>; }

pub struct Resolver { /* cache_root, http agent, optional token (GITHUB_TOKEN env) */ }
impl Resolver {
    /// Resolve to a local directory containing plugin.toml.
    /// GitHub: resolve ref -> commit sha (REST API), download codeload tarball,
    /// extract into <cache_root>/github/<owner>/<repo>/<sha>/, return dir (+ subdir).
    pub fn resolve(&self, source: &PluginSource, locked: Option<&LockedPlugin>) -> Result<ResolvedPlugin>;
}

pub struct ResolvedPlugin { pub root: PathBuf, pub pinned: Option<Pin> }  // Pin { ref_, commit }
```

- Cache root default: `~/.cache/rpp/plugins` (use `dirs` or honor `RPP_CACHE_DIR`).
- If a lockfile pin exists and is already in cache → **no network at all**.
- `ureq` with JSON; send `User-Agent: rpp`; use `GITHUB_TOKEN` if set.

### Lockfile `rpp.lock` (TOML, lives next to rpp.toml; managed by CLI)

```toml
version = 1
[[plugin]]
source = "github:example/rpp-plugins"
ref = "v1.2.0"          # what was requested (or default branch name)
commit = "<full sha>"
subdir = "plugins/atlas"
```

`rpp-fetch` provides `Lockfile::load/save`, lookup by source string, and update logic.
Path sources are never locked.

### Discovery / search

`rpp plugin search <query>` → GitHub repository search restricted to topic
**`rpp-plugin`** (`https://api.github.com/search/repositories?q=topic:rpp-plugin+<query>`),
returning name/full_name/description/stars. Provide
`pub fn search(query: &str) -> Result<Vec<RepoHit>>` with the HTTP layer factored so
tests can run against a local mock (trait or base-URL injection). **Tests must not hit
the network** — use local fixtures.

## 7. Incremental compilation (cache v2, in `crates/rpp`)

Layout: `.rpp/cache/manifest.bin` (bincode) + `.rpp/cache/objects/<xxh3-hex>` (CAS of
output contents).

Manifest:
- `global_key`: xxh3 of (rpp version, canonicalized full `rpp.toml` build-relevant
  sections, ordered list of plugin `cache_key`s).
- Per source file: `{ fingerprint: {mtime_ns, size, xxh3}, chain_key: u64, outputs: Vec<{ path, object: u64 }> }`
  (`outputs` empty = dropped). `chain_key` = xxh3 over the ordered `(plugin cache_key, processor name)`
  chain that applies to this file.
- Per generator: `{ plugin cache_key, read_set: Vec<{ kind: List|File|Source, key: String, hash: u64 }>, outputs: Vec<{ path, object }> }`.

Build flow:
1. Discovery walks `source` (respect `.rppignore` via `ignore` crate), fingerprints
   files (mtime+size fast path; hash on mismatch).
2. A file is **clean** iff global_key matches, fingerprint matches, and chain_key
   matches → outputs materialized from CAS (reflink/hardlink if possible, else copy).
   Dirty files go to the worker pool.
3. Generators re-run iff their read-set replays to different hashes (or global_key
   changed). Their reads during the run are recorded for next time.
4. Output dir is synced exactly: stale files removed (the engine owns `output`).
5. CAS objects garbage-collected when unreferenced by the new manifest.
6. Corrupt/old-version manifest → silently treated as empty (full rebuild).

`BuildResult` reports processed/cached/generated/dropped counts + duration; the engine
exposes what changed (paths written/removed) so dev-server can broadcast minimal
reloads and squash can run incrementally.

## 8. Squash (`crates/rpp-squash`)

```rust
pub struct SquashOptions { pub json: bool, pub png: PngLevel, pub strip: Vec<String>, /* … */ }
pub enum PngLevel { Off, Fast, Max }
pub struct SquashReport { pub files_optimized: usize, pub bytes_before: u64, pub bytes_after: u64, /* per-file details */ }

/// Optimize files in-place in a release staging directory, honoring options.
pub fn squash_dir(dir: &Path, opts: &SquashOptions) -> Result<SquashReport>;
/// Optimize a single file's bytes (used for incremental squash; keyed by caller).
pub fn squash_file(path: &str, contents: Vec<u8>, opts: &SquashOptions) -> Result<Option<Vec<u8>>>;
/// Write a deterministic zip of `dir` (sorted entries, fixed timestamps, deflate).
pub fn write_zip(dir: &Path, zip_path: &Path, opts: &ZipOptions) -> Result<()>;
/// engine = "packsquash": invoke external binary with a generated/passthrough options file.
pub fn run_packsquash(binary: &str, pack_dir: &Path, zip_path: &Path, options_file: Option<&Path>) -> Result<()>;
```

- JSON minify: `.json` + `.mcmeta` via serde_json (parse→compact). Invalid JSON = build
  **warning**, file passed through (some packs ship quirky JSON).
- PNG: `oxipng` library — Fast = preset 2, Max = preset 6 + `Zopfli` off (keep build
  times sane); always strip safe metadata chunks.
- Zip: `zip` crate, deflate, entries sorted by path, fixed DOS timestamp (1980-01-01),
  no extra fields → byte-reproducible builds. `pack.mcmeta` first in the archive.
- `strip` patterns delete matching files from release staging before zipping.
- Squashing is a release-archive operation. The engine-owned loose output remains
  unsquashed; builtin squash operates on a temporary staging copy. PackSquash likewise
  produces a release archive and is not run by `rpp dev`.

## 9. CLI (`crates/rpp-cli`, binary name `rpp`)

- `rpp init [dir]` — interactive scaffold: rpp.toml, src/pack.mcmeta, sample plugin,
  `.rpp/` gitignore, LuaLS definition files (`.rpp/api/*.lua`) for editor completion.
- `rpp build [--no-cache] [--no-squash] [--jobs N]` — full pipeline:
  resolve plugins (lockfile-aware) → build (incremental) → squash → zip.
  Pretty console output (indicatif/console): per-phase timing, cache hit counts,
  squash savings.
- `rpp dev` — watch + incremental rebuild + static file server + SSE (`/events`)
  live-reload events listing changed paths. Plugin file changes reload that plugin and
  invalidate accordingly; rpp.toml changes do a full reload.
- `rpp clean` — remove output + cache.
- `rpp plugin add <source> [--ref r] [--subdir d] [--project|--global]` /
  `remove <id> [--global]` / `list [--global]` /
  `update [id] [--global]` / `search <query>` — manages `[[plugin]]` entries
  (toml_edit, preserve formatting) and the corresponding lockfile.

## 10. Examples (must actually work)

- `examples/pack/` — a complete project: `rpp.toml` (uses local example plugins +
  squash enabled), `src/pack.mcmeta`, real `assets/minecraft/...` content (a few
  models, blockstates, lang files, textures — small hand-made PNGs are fine, generated
  by a checked-in script or tiny valid PNGs committed directly).
- `examples/plugins/json-minify/` (processor), `examples/plugins/mcmeta-validate/`
  (generator that validates pack.mcmeta + all `*.mcmeta` against pack_format),
  `examples/plugins/hash-rename/` (processor renaming via content hash, demonstrating
  `file.path` mutation), `examples/plugins/grayscale-wasm/` (WASM guest).
- Integration tests in the workspace build `examples/pack` end-to-end and assert real
  outputs (minified JSON, zip contents, incremental no-op second build).
