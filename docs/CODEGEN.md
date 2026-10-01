# Codegen Outputs

Status: **implemented**.

Some plugins produce artifacts whose consumer is not the resource pack. Window,
for example, compiles UI sources into pack font assets and also generates
typed Kotlin views for the server runtime. Those generated sources should not
be squashed into the resource-pack zip, but they do need to participate in the
same build and invalidation story.

## Declaring Output Roots

Projects declare named project-relative output roots per plugin in `rpp.config.ts`:

```ts
import { defineConfig } from "#rpp/config";
import window from "#plugins/window";

export default defineConfig({
  pack: { name: "my-pack" },
  plugins: [
    window(
      { kotlinPackage: "dev.example.generated" },
      { outputs: { kotlin: "../server/src/main/kotlin/dev/example/generated" } },
    ),
  ],
});
```

Declared output roots are safe in sandboxed mode: the project chooses each root,
and the plugin can only emit normalized paths beneath it. `trusted` is needed only
when granting ambient capabilities such as environment, filesystem, or process
access.

## Emitting Artifacts

A generator can write to a root with:

```ts
ctx.emitOutput("kotlin", "ShopView.kt", contents);
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

## Component Types

For plugins that call WASM components, `rpp codegen` writes TypeScript declarations
from each built component's export schema to `.rpp/generated/<name>.d.ts`, so
`components.load("<name>")` is typed. See [WASM_PLUGINS.md](WASM_PLUGINS.md).

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
