# RPP Specification

This document defines RPP's configuration, plugin contracts, and build behavior.
Read it before architectural changes, and update the relevant contract when behavior
changes. Contributor tooling and code conventions live in `AGENTS.md`.

RPP is a build tool for Minecraft resource packs: it takes a source directory, runs it
through a plugin pipeline (TypeScript and WASM plugins), and produces an optimized output
directory and a distributable `.zip`.

## Goals

1. A pleasant, well-designed **TypeScript plugin system** (package-based, sandboxed, running on V8).
2. A **WASM (WASIp2 component) plugin system** via wasmtime for heavier plugins.
3. **Plugin distribution & discovery via a registry** (`chunkzero/rpp-registry`) with a lockfile.
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
  rpp/          # core library: config, plugin model, TypeScript plugin runtime, build engine, cache
  rpp-archive/  # plugin archive format (.rpp.tgz): deterministic packing, safe unpacking
  rpp-fetch/    # plugin resolution: registry client, path deps, cache, lockfile, search
  rpp-squash/   # pack optimization: json minify, png optimize, zip, packsquash-extern
  rpp-wasm/     # wasmtime WASIp2 component host + WIT definitions
  rpp-js/       # Rolldown bundling + sandboxed V8 runtime (deno_core) for TS plugins
  rpp-cli/      # the `rpp` binary
examples/
  pack/         # a real, complete example resource pack project (rpp.config.ts, rpp.json, src/, plugins/)
  plugins/      # standalone example plugin packages (TypeScript + wasm guest crate)
docs/           # SPEC.md (this file), plugin authoring guides
```

Dependency direction: `rpp-archive`, `rpp-fetch`, `rpp-squash`, `rpp-wasm`, `rpp-js` are
**standalone** (they do NOT depend on `rpp`); `rpp-fetch` uses `rpp-archive` to unpack
installs. `rpp` optionally depends on `rpp-wasm` (feature `wasm`) via an adapter module. `rpp-cli` depends on all of them.

Code style: follow `AGENTS.md` (thiserror in libraries, anyhow in CLI, tracing for
logs, builder patterns, rustfmt). Every crate must pass `cargo test`,
`cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`.

---

## 1. Project configuration: `rpp.config.ts` and `rpp.json`

A project is configured by `rpp.config.ts`, whose default export is the config object, and
`rpp.json`, which lists the plugin packages the project depends on (see §6). A project
that has `rpp.toml` and no `rpp.config.ts` is rejected with a pointer to
[`docs/MIGRATING.md`](MIGRATING.md); having both is an error. Directory discovery walks up
to the nearest `rpp.config.ts` (stopping at `rpp.toml`). User-global plugins are not
supported and `~/.rpp/plugins.toml` is ignored.

```ts
import { defineConfig, plugin } from "#rpp/config";
import jsonMinify from "#plugins/json-minify";

export default defineConfig({
  pack: {
    name: "my-pack", // used for zip filename; required
    description: "An example pack", // exposed to plugins as ctx.pack.description
    packFormat: 34, // optional; validated against src/pack.mcmeta if present
  },
  build: {
    source: "src", // pack source dir (contains pack.mcmeta, assets/)
    output: "dist", // output dir; zip goes to dist/<name>.zip
    workers: 0, // 0 = available_parallelism
    limits: { memoryLimitMb: 256, executionDeadlineSeconds: 60 }, // per plugin runtime / call
    wasm: { memoryLimitMb: 512, executionDeadlineSeconds: 60 }, // per component instance / call
    squash: {
      enabled: true,
      engine: "builtin", // "builtin" | "packsquash"
      json: true, // minify .json/.mcmeta in output
      png: "fast", // false | "off" | "fast" | "max" (oxipng levels)
      zip: true, // produce dist/<name>.zip
      strip: ["**/.DS_Store", "**/Thumbs.db", "**/*.psd", "**/*.xcf"],
      packsquashBinary: "packsquash", // used when engine = "packsquash"
      packsquashOptions: "packsquash.toml", // optional passthrough options file
    },
  },
  dev: { host: "127.0.0.1", port: 8080, open: false },
  // Ordered plugin list. Order = tie-break order for processor priority.
  plugins: [
    jsonMinify({ pretty: false }), // a plugin's config factory (its `config` module)
    plugin("my-local-tool", { level: 2 }), // a plugin without a config module; untyped options
    plugin("codegen", undefined, {
      security: "trusted", // "sandboxed" (default) | "trusted"
      permissions: {
        // host capabilities (trusted only)
        process: ["kotlinc"], // programs `process.run` may launch
        environment: ["JAVA_HOME"], // host variables visible to processes/components
        read: ["data"], // WASI read-only preopens (project-relative)
        write: ["generated"], // WASI writable preopens
        network: false, // WASI sockets
        clocks: false, // WASI clocks; otherwise fixed
        random: false, // host randomness
        stdio: false, // inherit stdout/stderr in components
      },
      outputs: { kotlin: "../server/src/main/kotlin/generated" }, // named roots for non-pack files
    }),
  ],
});
```

- Keys are camelCase. `plugins` is an array of `{ plugin, options?, security?, permissions?,
outputs? }`; `plugin` names an `rpp.json` dependency (`^[a-z0-9][a-z0-9_-]*$`). `plugin(name,
options?, access?)` builds an entry, and a plugin's config factory (a `definePluginConfig`
  default export, imported as `#plugins/<name>`) validates and normalizes its options first.
- `#rpp/config` provides `defineConfig`, `plugin`, `definePluginConfig` and the config types.
- Keys inside `options` and `outputs` are kept verbatim; `null` values are invalid.
- The Lua-era keys (`build.lua`, `permissions.lua`, `security: "native"`, `id`, `source`, `ref`,
  `subdir`) are rejected with a pointer to the migration guide. Errors blame `rpp.config.ts`.

Any granted capability other than `outputs` makes the plugin non-deterministic
from RPP's point of view and disables cache replay for it.

Source `pack.mcmeta` format validation runs on each build, rather than configuration
loading, so configuration-only commands such as `clean` work with malformed sources.

## 2. Plugin manifest: `rpp.json`

A **plugin is a directory** ("plugin package") containing `rpp.json`. A directory with
`plugin.toml` and no `rpp.json` is a Lua plugin and is rejected with a pointer to the
migration guide, as is an `entry` ending in `.lua`.

```json
{
  "name": "window",
  "version": "0.1.0",
  "description": "…",
  "rpp": ">=0.2",
  "entry": "src/plugin.ts",
  "config": "src/config.ts",
  "components": { "compiler": "window.wasm" },
  "discover": { "windows": "*/window/**/window.ts" },
  "overrides": ["assets/*/textures/**"]
}
```

- `name` follows the plugin id grammar (`^[a-z0-9][a-z0-9_-]*$`); `version` is semver; `rpp`
  is a semver range checked during dependency resolution.
- `entry` defaults to `src/plugin.ts` and must be a `.ts`, `.mts`, `.js` or `.mjs` file;
  `entry`, `config` (the config-factory module) and component paths are relative.
- `components` maps component names to WASIp2 binaries callable with `components.load` (§5).
- `overrides` lists pack-path globs the plugin's generator may emit over or remove even when
  another source or plugin owns them. Entries are validated like other pack paths and globs.
- `discover` maps names (plugin id grammar) to one glob each, relative to the pack source
  directory (`build.source`). Matching files are bundled with the plugin, so adding one needs
  no configuration change, and `ctx.discovered(name)` returns `{ path, namespace?, module }`
  for each, sorted by path (an undeclared name throws `TypeError`). `namespace` is the
  segment matched by the pattern's first whole `*` segment when all earlier segments are
  literal, and must match `^[a-z0-9_.-]+$`. Authoring files import the plugin's `config`
  module as `#plugins/<name>`. Discovered files and the source files they import are
  authoring inputs: they are excluded from processors, `sourceFiles()` and pack output, as is
  every `.ts`, `.mts` or `.cts` file under the source directory (TypeScript is never pack content).
- `dependencies` is accepted and ignored; other unknown keys are rejected.

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
    /// Hash of everything per-file processors depend on; defaults to `cache_key()`.
    fn processor_key(&self) -> u64 { self.cache_key() }
    fn processors(&self) -> &[ProcessorDef];
    fn has_generator(&self) -> bool;
    /// Instantiate for one worker thread (V8 runtimes are per-worker).
    fn instantiate(&self) -> Result<Box<dyn PluginInstance>, Error>;
}

/// A live instance bound to one thread.
pub trait PluginInstance {
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
    fn list_source_files(&mut self, glob: Option<&str>) -> Vec<String>; // recorded as dep
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;        // processed file; recorded as dep
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>>;      // raw source file; recorded as dep
    fn emit(&mut self, path: &str, contents: Vec<u8>);             // add/overwrite output file
    fn remove(&mut self, path: &str);                              // drop an output file
    fn emit_output(&mut self, root: &str, path: &str, contents: Vec<u8>); // declared external root
}
```

Notes:

- Per-plugin **options** (from `plugins[].options` in `rpp.config.ts`) are provided to the factory
  at construction (as JSON-compatible values) and exposed through the runtime context.
  `GeneratorHost` itself has no `options()` method. Options are part of `cache_key()`.
- **Processors are pure**: input = (file path, contents, options). They get NO
  filesystem access. This is what makes per-file incremental caching sound.
- **Generators** access files only through `GeneratorHost`, which records a read-set
  for incremental invalidation.
- A generator's reads observe an immutable snapshot taken before that generator starts.
  It does not read back its own mutations. Later generators observe earlier generators'
  final output.
- Generators enumerate processed output files with `list_files` and raw source files
  with `list_source_files`. Both list reads are recorded for invalidation.
- Processor chain for a file: all matching processors across all plugins, sorted by
  `priority` ascending, ties broken by plugin order in `rpp.config.ts`, then by declaration
  order within a plugin. A `Dropped` outcome stops the chain and excludes the file.
- Plugin errors abort the build with a message attributing plugin id + processor +
  file path.

## 4. TypeScript plugin system

Plugins are TypeScript (or JavaScript) modules bundled by Rolldown and run in a sandboxed V8
isolate (`crates/rpp-js`, wired into `rpp` by the `js` feature). The entry module default-exports
a plugin built with the SDK, imported as `#rpp` (written to `.rpp/sdk/index.ts` by `rpp codegen`,
so it always matches the running rpp):

```ts
import { definePlugin, hash, path } from "#rpp";

export default definePlugin<{ pretty?: boolean }>({
  // Processors: parallel, per matching file. Pure: (ctx, file) only.
  processors: {
    minify: {
      files: ["**/*.json", "**/*.mcmeta"], // glob or glob list; required
      priority: 50, // optional, default 0, lower runs first
      run(ctx, file) {
        file.text = JSON.stringify(JSON.parse(file.text)); // assignment marks the file modified
      },
    },
  },
  // Generator: sequential, after processing, sees all files.
  generate(ctx) {
    for (const file of ctx.files("assets/*/textures/**/*.png")) {
      const bytes = ctx.read(file)!;
      // ...
    }
    ctx.emit("assets/minecraft/atlases/blocks.json", JSON.stringify(atlas));
  },
  onStart(ctx) {},
  onFinish(ctx, stats) {},
});
```

### Semantics

- **file** in processors: `file.path` (get/set, relative forward-slash string; setting it
  renames the output), `file.bytes` (`Uint8Array`) and `file.text` (UTF-8 view of `bytes`), both
  get/set, and `file.drop()` (exclude from output, stops the chain). The runtime tracks whether
  mutation occurred to report Unchanged/Modified/Dropped.
- **ctx** (all handlers): `ctx.plugin` (the plugin name), `ctx.options` (plugin options), and
  `ctx.pack` (`{ name, description?, format? }`).
- **ctx** (generator, `onStart`, `onFinish`): additionally `ctx.discovered(name)` (§2).
  Processors do not get it.
- **ctx** (generator): additionally `files(glob?)`, `sourceFiles(glob?)`, `read(path)` and
  `readText(path)` (processed output), `readSource(path)` and `readSourceText(path)` (raw source),
  `emit(path, contents)` (add or overwrite), `remove(path)`, and `emitOutput(root, path, contents)`
  (write into a root declared in the plugin's `outputs`). Contents are `Uint8Array | string`.
  Reads are tracked for incremental rebuilds.
- Throwing fails the build with plugin/processor/file attribution. `onFinish` receives the
  build counts (`processed`, `cached`, `generated`, `dropped`).

### SDK modules

- `hash` — `xxh3(data) -> hex string`, `sha256(data) -> hex`, `md5(data) -> hex`,
  `crc32(data) -> number`; `data` is a `Uint8Array` or string.
- `path` — `join(...)`, `dirname(p)`, `basename(p)`, `ext(p)`, `withExt(p, e)`,
  `match(glob, p) -> boolean`.
- `toml` — `parse(text)`, `stringify(value)`.
- `components` — `load(name)` for components declared in `rpp.json`; see §5.
- `process` — `run({ program, args?, env?, stdin?, cwd?, timeoutMs? }) -> { status, stdout,
stderr }`. Requires `security: "trusted"` with the program in `permissions.process` and is
  only callable from the generator and hooks. The shorter of `timeoutMs` and the remaining
  execution deadline bounds stdin writing, process execution, and stdout/stderr draining,
  including inherited descendant pipes. RPP closes its pipes and terminates remaining members
  of the invocation's process group (Unix) or job object (Windows) on completion or error.
  Descendants that deliberately leave that group or job are outside this cleanup boundary. Each
  captured output stream is limited to 16 MiB; excess bytes are drained and discarded.
- JSON uses the language's own `JSON.parse` and `JSON.stringify`; object keys keep insertion order.

### Module resolution & sandbox

- Imports resolve within the plugin package, to `#rpp`, `#rpp/config`, `#plugins/<name>` for
  packages with a config module, and to npm dependencies inlined from `node_modules`. `node:`
  imports are rejected. There is no filesystem, network, clock or randomness access: the
  isolate gets deterministic host calls only, and trusted clock/random grants disable cache
  replay for that plugin.
- Each plugin gets its own isolate; plugins cannot see each other's globals. Memory and per-call
  time limits per runtime come from `build.limits`.

### Execution model

- One runtime **per plugin, per worker thread**. This keeps memory limits, module state and
  component handles isolated between plugins while still giving every worker an independent
  instance.
- Loading = read `rpp.json`, bundle `entry` (and discovered modules), run it in the sandbox,
  and collect the default export's processors/generator/hooks. Processor defs (patterns/priority)
  are extracted at load time on the main thread (a validation load), then re-instantiated per
  worker via `PluginFactory::instantiate`.
- Generators and lifecycle hooks run on a single dedicated instance (main thread).
- A TypeScript plugin has two keys. `processor_key` (chains, `compile_processors`) is xxh3 over
  the rpp version, manifest, component binaries, canonical options, host-access policy and the
  content of every bundled file outside the source directory (the plugin package).
  `cache_key` (generators) adds the bundled code, which contains the discovered module list and
  the source helpers they import. Adding, removing or editing a discovered module therefore
  reruns generators but reuses cached processor results. Processors do not get
  `ctx.discovered()` (it throws), and top-level side effects of discovered modules are not
  tracked for processors.
- TypeScript bundles are cached at `.rpp/cache/bundles/<xxh3-hex of plugin id>.bin` (bincode),
  keyed by the bundler request (rpp version, root, virtual modules including the discovered
  list, packages) and the content hash of every file the bundle read. A corrupt or stale entry
  is a miss. A new file that changes import resolution without touching a recorded input needs
  `rpp clean`.

## 5. WASM component system (`crates/rpp-wasm`)

Host for **WASIp2 components** using `wasmtime` (component model +
`wasmtime-wasi`). rpp plugins remain TypeScript packages; they load named components
declared in `rpp.json`.

### Host crate API (normative shape)

```rust
pub struct WasmEngine { /* wasmtime Engine, shared */ }
pub struct CompiledComponent { /* Component + schema, Send+Sync, cheap to instantiate */ }
pub struct WasmInstance { /* Store + bindings */ }

impl WasmEngine { pub fn new(limits: Limits, cache_dir: Option<&Path>) -> Result<Self>; pub fn load(&self, wasm_path: &Path) -> Result<CompiledComponent>; }
impl CompiledComponent { pub fn schema(&self) -> &Schema; pub fn instantiate(&self, permissions: Permissions) -> Result<WasmInstance>; }
impl WasmInstance {
    pub fn call(&mut self, export_path: &str, args: &[Value]) -> Result<Vec<Value>>;
}
```

- WASM is available as named WASIp2 components loaded by plugins with
  `components.load(name)`. Components export ordinary WIT functions; rpp
  validates imports against plugin capabilities and provides WASI without
  filesystem preopens, network, passed env, or process execution by default.
- `rpp codegen` generates `.rpp/generated/<name>.d.ts` from a built component's export schema,
  typing `components.load(name)`.
- Component compilation uses an in-memory content-digest map and a persistent
  project-local Wasmtime cache. Replacing a component binary invalidates the plugin
  cache key even when its path and TypeScript are unchanged.
- Permissionless WASI random imports receive deterministic streams. Granting
  `permissions.random = true` enables host randomness and disables build replay for
  that plugin.
- Component calls made from processors use a fresh instance per file so guest globals
  and deterministic random-stream position cannot couple output to worker scheduling.
  Generator and hook calls may reuse an instance for a sequential component workflow.
- Resource limits: memory cap via `StoreLimits` and epoch interruption.

### TypeScript component value representation

Values are converted from the component type signature (the full table is in
[`WASM_PLUGINS.md`](WASM_PLUGINS.md)). `list<u8>` is a `Uint8Array`, `s64`/`u64` are `bigint`,
records use camelCase fields, variants are `{ tag, val }`, and an `option<T>` is `T` or
`undefined`, except that an option nested in an option uses `{ tag: "some" | "none", val }`.
A `result` return unwraps to its `ok` payload and throws `ComponentError` carrying the `err`
payload. Traps and exceeded deadlines throw `ComponentTrapError` and `ComponentTimeoutError`
and make the handle unusable.

## 6. Plugin resolution & discovery (`crates/rpp-fetch`)

Standalone, blocking (`ureq`), no async. Plugins come from the registry or from local
directories; GitHub sources (`github:`), the GitHub codeload fetcher and `rpp.lock` versions 1
and 2 were removed (a `github:` dependency or an old lockfile is rejected with a pointer to
[`docs/MIGRATING.md`](MIGRATING.md)). Its public API is `Error`, `Incompatibility`, `Result`,
`HttpConfig`, `DEFAULT_REGISTRY_BASE`, `USER_AGENT` and the `registry` module. Requests send no
credentials. Tests must not hit the network; the HTTP layer takes a base URL so tests run
against a local mock.

### Registry dependencies (`rpp.json`)

Projects declare the plugins they use in `rpp.json` and configure them in `rpp.config.ts`.

```json
{
  "dependencies": {
    "window": "^0.1.0",
    "local-tools": "path:../tools"
  }
}
```

- Keys are plugin names (`^[a-z0-9][a-z0-9_-]*$`, at most 64 characters) and must equal
  the `name` in the package's own `rpp.json`.
- A value is a semver range against the registry, or `path:<dir>` (relative to the
  project root). A bare version (`0.1.4`) means exactly that version; `*` is rejected.
- The registry is the git repository `chunkzero/rpp-registry`, read over HTTP from
  `https://raw.githubusercontent.com/chunkzero/rpp-registry/main` (`RPP_REGISTRY`
  overrides it). `plugins/<name>.json` lists a plugin's `repository`, `description` and
  `versions`, each with `version`, `url` (a release archive), `sha256`, the supported
  `rpp` range, and `yanked`. `index.json` lists every plugin's `name`, `description`,
  `repository` and `latest` version for search.
- Resolution picks the newest non-yanked version matching the range whose `rpp` range
  accepts the running rpp version, ignoring rpp's pre-release suffix.
- Archives are plugin archives (§9) of at most 128 MiB, including `rpp.json`. They are
  verified against `sha256`, unpacked by `rpp-archive` into a temporary sibling directory
  that is renamed to `<cache>/registry/<name>/<version>-<hash>/`
  (`<cache>` is `RPP_CACHE_DIR` or `~/.cache/rpp`), and their manifest must declare the
  requested name and version.
- `rpp.lock` version 3 pins each registry dependency's `name`, `requested` spec,
  `version`, `rpp` range, `url` and `sha256`. A pin is reused while its `requested`
  spec is unchanged; reused pins with a cached archive make no network requests.
  Yanked versions still install when pinned. `path:` dependencies are never locked.
- `rpp.lock` lives next to `rpp.json` and is managed by the CLI; a version 1 or 2 lock is
  rejected with the instruction to delete it and rebuild.
- `rpp add <name>[@range] | path:<dir>` adds a dependency (a bare name records
  `^<selected version>`), `rpp remove <name>` removes one, `rpp update [name...]`
  re-selects pinned versions within their ranges, and `rpp search <query>` searches
  `index.json`.

## 7. Incremental compilation (cache v3, in `crates/rpp`)

Layout: `.rpp/cache/manifest.bin` (bincode) + `.rpp/cache/objects/<xxh3-hex>` (CAS of
output contents).

Manifest:

- `global_key`: xxh3 of (rpp version, canonicalized full `rpp.config.ts` build-relevant
  sections). Plugin keys are not part of it: a plugin change invalidates only the chains
  containing its processors (`processor_key`) and its own generator (`cache_key`).
- Per source file: `{ fingerprint: {mtime_ns, size, xxh3}, chain_key: u64, outputs: Vec<{ path, object: u64 }> }`
  (`outputs` empty = dropped). `chain_key` = xxh3 over the ordered `(plugin processor_key, processor name)`
  chain that applies to this file.
- Per generator: `{ plugin cache_key, read_set: Vec<{ kind: List|SourceList|File|Source, key: String, hash: u64 }>, outputs: Vec<{ path, object }> }`.

Build flow:

1. Discovery walks `source` (respect `.rppignore` via `ignore` crate), fingerprints
   files (mtime+size fast path; hash on mismatch).
2. A file is **clean** iff global_key matches, fingerprint matches, and chain_key
   matches → outputs materialized from CAS (reflink/hardlink if possible, else copy).
   Dirty files go to the worker pool.
3. Generators re-run iff their read-set replays to different hashes (or global_key
   changed). Their reads during the run are recorded for next time.
   Every pack path has an owner: the source file whose processors produced it, or the
   generator plugin that last emitted it. A generator may emit or remove a path only if it is
   unowned, owned by that plugin, or matched by the plugin's `overrides` (ownership then moves
   to it or is cleared). Anything else is a build error naming the owner. Cache replay applies
   recorded emits/removes through the same check against the current state.
4. Output dir is synced exactly: stale files removed (the engine owns `output`).
5. CAS objects garbage-collected when unreferenced by the new manifest.
6. Corrupt/old-version manifest → silently treated as empty (full rebuild).

Declared external outputs use a separate `.rpp/external-outputs.bin` ownership
manifest, so stale generated files can still be removed after cache deletion,
configuration/plugin changes, and `rpp clean`. Only paths recorded as RPP-owned are
removed; unrelated files beside generated artifacts are preserved. An existing unowned file
at an emitted path is adopted when its bytes already match; otherwise the build fails with
`refusing to overwrite unowned file`. Replacements are
published atomically from sibling temporary files and are never hard-linked to the
immutable CAS. External-output collisions are build errors with both plugin ids in the
diagnostic. Before pack synchronization, RPP validates external destinations and
stages all external writes, then persists ownership of both previous and planned
outputs. Publication removes stale paths and atomically replaces staged files; only
after success does ownership shrink to the new generation. If publication fails,
retry or clean uses the retained ownership to account for partially published files.
Staging or initial ownership persistence failures leave published files unchanged.
This is recoverable publication, not a transaction across filesystems.

Build and clean validate filesystem destinations before mutation. Pack output,
bookkeeping paths, and external roots inside the project must not pass through
symlinks below the project root; existing ancestors are checked even when the final
directory does not exist yet. `build.source` is read-only and may be a symlink.
External roots outside the project, such as `../server/generated`, are resolved
through their existing ancestors (symlinked parents are allowed) and must not
resolve into source, pack output, or `.rpp`, nor contain them. External emitted and
previously owned paths are checked below their roots before writing or removing
files, including during clean after configuration changes. Symlinked pack output
ancestors are rejected; individual pack output file symlinks are replaced safely.
These checks assume directories are not concurrently replaced by another process.

`BuildResult` reports processed/cached/generated/dropped counts + duration; the engine
exposes what changed (paths written/removed) so dev-server can broadcast minimal
reloads and squash can run incrementally. `generated` counts generator executions,
not individual emitted files. External written/removed paths are reported separately.

## 8. Squash (`crates/rpp-squash`)

```rust
pub struct SquashOptions { pub json: bool, pub png: PngLevel, pub strip: Vec<String>, /* … */ }
pub enum PngLevel { Off, Fast, Max }
pub struct SquashReport { pub files_optimized: usize, pub files_stripped: usize, pub bytes_before: u64, pub bytes_after: u64, pub warnings: Vec<String> }

/// Optimize files in-place in a release staging directory, honoring options.
pub fn squash_dir(dir: &Path, opts: &SquashOptions) -> Result<SquashReport>;
/// Write a deterministic zip of `dir` (sorted entries, fixed timestamps, deflate)
/// atomically: staged in a temp file beside `zip_path`, mode 0644, then renamed.
pub fn write_zip(dir: &Path, zip_path: &Path) -> Result<()>;
/// The same deterministic zip bytes, built in memory.
pub fn zip_to_vec(dir: &Path) -> Result<Vec<u8>>;
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

- `rpp init [dir]` — scaffold a TypeScript project: `rpp.config.ts`, `rpp.json`,
  `plugins/hello/{rpp.json,src/plugin.ts}`, `pack.mcmeta` and `.gitignore`.
- `rpp build [--no-cache] [--no-squash] [--jobs N]` — full pipeline:
  resolve plugins (lockfile-aware) → build (incremental) → squash → zip.
  Console output: per-phase timing, cache hit counts, squash savings.
- `rpp dev` — watch + incremental rebuild + static file server + SSE (`/events`)
  live-reload events listing changed paths. Plugin file changes reload that plugin and
  invalidate accordingly; `rpp.config.ts` and `rpp.json` changes do a full reload.
- `rpp clean` — remove output + cache.
- `rpp codegen` — write the TypeScript SDK (`.rpp/sdk/`) and `.rpp/tsconfig.json`, and a
  root `tsconfig.json` if missing, in the nearest directory with `rpp.config.ts` or `rpp.json`, and a
  `.rpp/generated/<name>.d.ts` for each built component a plugin manifest declares.
  `build` and `dev` do this best-effort.
- `rpp check` — `codegen`, then run `tsc -p tsconfig.json --noEmit`. The compiler is
  `RPP_TSC`, else `toolchain/typescript/7.0.2/tsc` beside the `rpp` executable (bundled in
  release archives), else `tsc` on PATH (TypeScript 7+). A non-zero exit fails the command.
- `rpp add <name>[@range] | path:<dir>`, `rpp remove <name>`, `rpp update [name...]` and
  `rpp search <query>` manage the `rpp.json` dependencies and `rpp.lock` (§6). The Lua-era
  `rpp plugin add|remove|list|update|search` and `rpp component` commands are gone.

- `rpp plugin pack [dir] [--out <dir>] [--json]` — bundles a plugin with `rpp.json` into
  `<name>-<version>.rpp.tgz` plus `<file>.sha256` (`<hex>  <file>`), written to `--out`
  (default: the plugin directory). `--json` prints `{name, version, rpp, description, file,
sha256}` on stdout. `rpp.json` must set `rpp`. The archive holds the manifest with
  `entry: "dist/plugin.js"`, `config: "dist/config.js"` and `dependencies` removed;
  `dist/*.js` with `.js.map` files (shared code in `dist/chunk-<hash>.js`); every
  `components` module; and, for a TypeScript config, `types/**.d.ts` from isolated
  declarations plus `dist/config.d.ts`. npm dependencies are inlined from `node_modules`
  (including hoisted ones outside the plugin directory, resolved through `module` then
  `main`); `#rpp` and `#rpp/*` stay imports; `node:` imports are rejected. Source maps
  list the original files (`../src/plugin.ts`, `../node_modules/dep/index.js`), and a
  loaded file's `//# sourceMappingURL=` map is chained, so installed plugins report stacks
  at their original sources. Config modules must support isolated declarations and their
  public types must not reference dependency types. Each archive is checked by unpacking
  it and bundling its entries as an installing rpp does.

### Plugin archive format (`crates/rpp-archive`)

A plugin archive (`.rpp.tgz`) is a gzipped tar of regular files at the archive root.
`rpp_archive::pack` writes entries sorted by path with mtime 0, owner 0 and mode 0644, so
equal files give byte-identical archives, and rejects paths that are not normalized
`/`-separated relative paths. `rpp_archive::unpack`, used by installs (§6) and by
`rpp plugin pack`'s check, extracts only regular files and directories beneath its
destination and rejects absolute or `..` paths, symlinks, hard links and other special
entries. Both enforce the same limits: at most 20,000 entries, 64 MiB per file and
512 MiB in total.

Prompts and command status use cliclack on stderr. Redirected stderr and `TERM=dumb`
receive plain status lines; command results such as search hits stay on stdout. Prompts
require both stdin and stderr to be terminals. Otherwise, `init` accepts defaults; `--yes`
bypasses its prompts. Cancelling a prompt
exits with status 130. `NO_COLOR` disables color.

Diagnostics and ongoing dev-server activity use tracing on stderr, controlled by
`-v` / `-vv` and `RUST_LOG`.

### Dev-server pack update protocol

`GET /events` is an SSE stream with JSON data and 15-second keepalive comments.
Connection and broadcast-lag snapshots use the named SSE event `pack`:

```text
event: pack
data: {"type":"pack","pack":{"url":"/packs/<sha1>.zip","sha1":"<40 lowercase hex characters>","size":1234}}
```

Successful rebuilds with changed pack outputs send unnamed reload events:

```json
{"type":"reload","changed":["assets/minecraft/lang/en_us.json"],"pack":{"url":"/packs/<sha1>.zip","sha1":"<40 lowercase hex characters>","size":1234}}
```

`reload` always has a nonempty `changed` array preserving the changed-path contract.
Snapshots have no `changed` field and use a named SSE event so browser
`onmessage` handlers do not reload on connection. Browser pack consumers use
`addEventListener("pack", ...)` as well as handling pack metadata on reloads.
When present, `pack` describes the latest available archive when an event is
consumed; queued pack metadata may be coalesced to that snapshot. Reloads without
pack metadata retain that shape. Broadcast lag yields a pack snapshot.
Clients deduplicate pack offers by SHA-1, including after reconnects. There is no
event ID/replay history; disconnected clients catch up to the latest pack.

After every successful build, dev creates a deterministic, unsquashed ZIP outside
the engine-owned output and computes its SHA-1 (the Minecraft download hash).
Release squash and ZIP settings do not disable this archive. The bytes and metadata
are published together only after archive creation succeeds. Identical bytes do
not produce a new pack update. External-output-only changes do not update the pack.
The initial build must succeed before HTTP starts.

`GET /packs/<sha1>.zip` returns the current archive with `application/zip`.
The URL is root-relative; callers may resolve it against a player-reachable base
address. Only the latest archive is retained in memory; unknown or superseded
hashes return 404. A response already started holds its immutable bytes even if a
rebuild publishes a newer pack. Loose files remain available via the static
fallback; `/events` and `/packs/:file` are reserved.

Failed rebuilds or archive creation leave the last successful archive available
and send `{"type":"build_error","message":"..."}`, with no pack update.
Unchanged successful rebuilds emit no event unless archive publication recovers
from a prior failure, in which case a named `pack` event announces the recovered
archive without a reload. Build errors are live notifications,
not persistent history, and startup failures terminate the CLI.

The Java 21+ client in `integrations/jvm` connects to this protocol. It exposes
URL/hash/size metadata and connection, protocol, build, and callback failures,
reconnects after interruptions, and leaves player scheduling and resource-pack
responses to platform integrations. Its README contains a runnable Minestom
example and Spigot caller integration.

## 10. Examples (must actually work)

- `examples/pack/` — a complete project: `rpp.config.ts` and `rpp.json` (local example plugins
  as `path:` dependencies + squash enabled), a pack-local `catalog` plugin that uses `discover`
  for item definitions, `src/pack.mcmeta`, real `assets/minecraft/...` content (a few
  models, blockstates, lang files, textures — small hand-made PNGs are fine, generated
  by a checked-in script or tiny valid PNGs committed directly).
- `examples/plugins/json-minify/` (processor), `examples/plugins/mcmeta-validate/`
  (generator that validates pack.mcmeta + all `*.mcmeta` against pack_format),
  `examples/plugins/hash-rename/` (generator renaming processed output via content
  hash), `examples/plugins/grayscale-wasm/` (processor backed by a
  WASIp2 component built from a Rust guest crate; `just example-wasm`). Each is an `rpp.json`
  package with `src/plugin.ts`.
- `crates/rpp-cli/tests/examples.rs` builds `examples/pack` through the CLI and asserts real
  outputs (minified JSON, zip contents, incremental no-op second build), and runs the
  grayscale component over a real PNG.

Plugin projects can depend on the `rpp-cli` library in integration tests:
`rpp_cli::project::Project::discover` loads the project containing a directory,
and `build_engine()` returns the engine whose `build()` reports structured
results (counts plus written/removed paths, including external outputs).
