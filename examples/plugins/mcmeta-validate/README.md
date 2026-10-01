# mcmeta-validate

An **rpp** TypeScript _generator_ plugin that validates a resource pack's
`.mcmeta` files and fails the build if anything is malformed.

## What it demonstrates

- A **generator** (`generate(ctx)`), which runs once after all per-file
  processing and has access to the whole pack: `ctx.files(glob)`,
  `ctx.readSourceText(path)`, and `ctx.pack`.
- Cross-project validation that a pure processor cannot do: checking that
  `pack.mcmeta`'s `pack_format` matches the `packFormat` pinned in
  `rpp.config.ts` (surfaced as `ctx.pack.format`).
- Failing a build by throwing; rpp attributes the message to this plugin.
- Plugin-local modules: the validation rules live in `src/rules.ts` and are
  imported by the entry like any other TypeScript module.
- Incremental friendliness: validation reads files through `ctx.readSourceText`,
  so its read-set is recorded and it only re-runs when an input `.mcmeta`
  actually changes.

## What it checks

- `pack.mcmeta`: has a `pack` object; `pack_format` is a positive integer and
  (when pinned) matches `rpp.config.ts`; `description` is a string or text component.
- Every `*.png.mcmeta`: has an `animation` object; `frametime` is a positive
  integer; `interpolate` is a boolean; `frames` entries are valid frame indices
  or `{ index, time }` records.

This plugin emits no output files; it is a pure correctness gate.

## Usage

`rpp.json`:

```json
{ "dependencies": { "mcmeta-validate": "path:../plugins/mcmeta-validate" } }
```

`rpp.config.ts`:

```ts
import { defineConfig, plugin } from "#rpp/config";

export default defineConfig({
  pack: { name: "my-pack", packFormat: 34 },
  plugins: [plugin("mcmeta-validate")],
});
```
