# mcmeta-validate

An **rpp** TypeScript _generator_ plugin that validates a resource pack's
texture animation `.mcmeta` files and fails the build if anything is malformed.

## What it demonstrates

- A **generator** (`generate(ctx)`), which runs once after all per-file
  processing and has access to the whole pack: `ctx.files(glob)`,
  `ctx.readSourceText(path)`, and `ctx.pack`.
- Whole-pack validation that a pure processor cannot do: every problem across
  the source tree is reported in one failure.
- Failing a build by throwing; rpp attributes the message to this plugin.
- Plugin-local modules: the validation rules live in `src/rules.ts` and are
  imported by the entry like any other TypeScript module.
- Incremental friendliness: validation reads files through `ctx.readSourceText`,
  so its read-set is recorded and it only re-runs when an input `.mcmeta`
  actually changes.

## What it checks

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
  pack: { name: "my-pack", format: 34 },
  plugins: [plugin("mcmeta-validate")],
});
```
