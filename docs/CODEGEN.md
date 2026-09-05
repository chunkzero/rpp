# Codegen Outputs

Status: **implemented**.

Some plugins produce artifacts whose consumer is not the resource pack. Window,
for example, compiles Lua UI sources into pack font assets and also generates
typed Kotlin views for the server runtime. Those generated sources should not
be squashed into the resource-pack zip, but they do need to participate in the
same build and invalidation story.

## Declaring Output Roots

Plugins may declare named project-relative output roots in `rpp.toml`:

```toml
[[plugin]]
id = "window"

[plugin.options]
kotlin_package = "dev.example.generated"

[plugin.outputs]
kotlin = "../server/src/main/kotlin/dev/example/generated"
```

Declared output roots are safe in sandboxed mode: the project chooses each root,
and the plugin can only emit normalized paths beneath it. `trusted` is needed only
when granting ambient capabilities such as environment, filesystem, or process
access; `native` remains the unrestricted local-tooling mode.

The `id = "window"` form references a plugin installed globally with
`rpp plugin add <path-or-source> --global`; the project still owns the options
and output roots.

## Emitting Artifacts

Generator Lua can write to a root with:

```lua
ctx:emit_output("kotlin", "ShopView.kt", contents)
```

The engine records external-output mutations alongside normal generator
mutations. External outputs are copied (never hard-linked) from the
content-addressed object store after the pack output sync finishes. Each file is
published through a same-directory temporary file and atomic replacement, so an
update never intentionally leaves a missing or partially written generated source.
A durable ownership manifest outside the incremental cache removes stale files
after normal rebuilds, `--no-cache`, plugin/config changes, and `rpp clean`.

External outputs are excluded from squash and zip. They are still driven by the
generator read-set, so changing a source read by the generator reruns codegen,
while unrelated pack-only cache hits do not rewrite generated artifacts.
Files not recorded in the ownership manifest are never removed. Two plugins that
resolve to the same external path fail with an attributed collision diagnostic.

## Component Bindgen

For Lua plugins that call WASM components, `rpp component bindgen` can generate
a Lua wrapper from the component export schema:

```bash
rpp component bindgen compiler.wasm --name compiler --out compiler.lua
```

The wrapper uses `rpp.component.load("<name>")`, calls typed component exports,
and includes LuaLS-style annotations for the generated functions.

## Integration Tests

The `rpp-cli` crate is also a library. Plugin repositories can drive builds from
their own tests without loading user-global plugins:

```rust
let mut project = rpp_cli::project::Project::discover_isolated("example/pack")?;
project.config.build.workers = 1;
let result = project.build_engine()?.build()?;
assert_eq!(result.generated, 1);
assert!(result.changes.external.written.is_empty());
```

`project.clean_artifacts()` removes the output, the cache, and RPP-owned external
files. Deleting only `.rpp/cache` matches `rpp build --no-cache`: the separate
ownership manifest survives so stale generated sources are still cleaned up.
