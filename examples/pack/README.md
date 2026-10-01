# RPP example pack

A resource pack project showing per-file processors, ordered generators,
TypeScript item definitions found with `discover`, incremental dependencies, and
declared external outputs. The baseline uses sandboxed TypeScript plugins and
checked-in PNGs; no guest compilation is needed.

## Build and develop

From `examples/pack`:

```bash
cargo run -p rpp-cli -- build
cargo run -p rpp-cli -- build       # unchanged inputs reuse cached work
cargo run -p rpp-cli -- dev         # watch sources and rebuild; Ctrl-C to stop
cargo run -p rpp-cli -- clean       # remove build artifacts and owned catalog files
```

The example retains pack format 34. Its diamond sword texture replaces a vanilla
asset. The custom models demonstrate asset generation; adding a model does not
register a new Minecraft item or automatically assign it to an in-game item.

## Layout

```text
rpp.config.ts                    Pack metadata, limits, plugin options, output roots
rpp.json                         Plugin dependencies (path packages)
plugins/catalog/                 Pack-local TypeScript plugin
src/items/ember_gem.ts           Authoring input; never included in the pack
src/pack.mcmeta                  Validated against rpp.config.ts
src/assets/minecraft/            Models, language, static and animated textures
src/.rppignore                   Excludes design notes
src/notes/design.txt             Ignored input
tools/gen_textures.py            Deterministic PNG generation using Python stdlib
```

The reusable processors and generators live in [`../plugins`](../plugins).

## Pipeline

All file processors run before generators. Generators run in registration order;
each sees the outputs of earlier generators.

1. **json-minify** compacts JSON and metadata with a per-file processor.
2. **mcmeta-validate** checks source pack metadata and animation metadata,
   demonstrating tracked source reads and a plugin-local `rules.ts` module.
3. **hash-rename** fingerprints `textures/custom/` outputs and emits
   `rename_map.json`. Vanilla texture paths stay fixed.
4. **catalog**, the pack-local plugin, declares `discover: { items: "items/*.ts" }`
   in its `rpp.json`. Discovered modules are authoring inputs that never reach the
   pack, and `ctx.discovered("items")` returns them to its generator. The generator
   reads the rename map, fixes texture references in existing models, and emits
   models and English translations from item definitions. It exports an item
   catalog with `emitOutput` to the `catalog` root declared in `rpp.config.ts`.
5. **Built-in squash** minifies JSON and optimizes PNGs for the deterministic
   release ZIP. Loose output stays at the pre-squash stage, so texture names hash
   those processed bytes, not the ZIP's optimized PNG bytes.

`build.limits` sets memory and execution limits for TypeScript plugins. The catalog output root works in
the default sandbox without filesystem or process grants.

## Generated output

```text
dist/
  pack.mcmeta
  assets/minecraft/models/item/magic_gem.json   Updated hashed texture reference
  assets/minecraft/textures/custom/gem.<hash>.png
  assets/rpp/models/item/ember_gem.json         Generated model
  assets/rpp/lang/en_us.json                   Generated translations
  rename_map.json
  rpp-example-pack.zip
generated/catalog/items.json                  External artifact, outside the ZIP
```

The external catalog links each item ID to its model, translation key, and resolved
texture. A server or another tool can consume it without unpacking the resource
pack. RPP tracks ownership and cleans up its generated files while preserving
unowned files in that directory.

## Try a change

Edit `src/items/ember_gem.ts` to change its display name, or add
`src/items/frost_gem.ts`:

```ts
import type { Item } from "#plugins/example-catalog";

export default { name: "Frost Gem", texture: "minecraft:custom/gem" } satisfies Item;
```

Rebuild to generate the new model, translation, and catalog entry. Definitions
must export `name` and `texture` strings; filenames use lowercase letters,
digits, underscores, or hyphens. Textures must exist in this example pack.
Deleting a definition removes its generated model and catalog entry on rebuild.
Discovered modules are bundled with the plugin, so additions, edits, and deletions
invalidate its generator. An unchanged build replays cached results.

Change `namespace` in the catalog plugin options to relocate generated models
and translations. Change the declared `catalog` output root to send the external
artifact to another directory. `rpp check` type-checks the project with TypeScript 7+.

## Optional WASM processor

From the repository root, install the guest target and build the component:

```bash
rustup target add wasm32-wasip2
just example-wasm
```

Add `"grayscale-wasm": "path:../plugins/grayscale-wasm"` to the dependencies in
`rpp.json`, then append `plugin("grayscale-wasm")` to `plugins` in `rpp.config.ts`.

The processor converts PNGs to grayscale through a WASIp2 component. Even when
listed last, it runs before every generator, so hash-rename fingerprints the
converted bytes and the catalog resolves the resulting texture names. See the
[WASM example](../plugins/grayscale-wasm/README.md) for the guest implementation.

To regenerate the original checked-in textures, run
`python3 tools/gen_textures.py` from this directory.

## Verification

`cargo test -p rpp-cli --test examples` builds a temporary copy using this
configuration. It checks minified output, repaired references, generated assets,
the external catalog, ignored files, an identical cached rebuild, and validation
failures. It also runs the WASM example over a real PNG when the `wasm32-wasip2`
target is installed.
