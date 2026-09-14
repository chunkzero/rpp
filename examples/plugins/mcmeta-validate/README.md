# mcmeta-validate

An example **rpp** Lua _generator_ plugin that validates a resource pack's
`.mcmeta` files and fails the build if anything is malformed.

## What it demonstrates

- A **generator** (`plugin:generator(...)`), which runs once after all
  per-file processing and has access to the whole pack via the `ctx` host API:
  `ctx:files(glob)`, `ctx:read_source(path)`, and `ctx.pack`.
- Cross-project validation that a pure processor cannot do — checking that
  `pack.mcmeta`'s `pack_format` matches the `pack_format` pinned in `rpp.toml`
  (surfaced as `ctx.pack.format`).
- Failing a build with `error()`; rpp attributes the message to this plugin.
- **Plugin-local `require`**: the validation rules live in `rules.lua` next to
  `init.lua` and are loaded with `require("rules")`. The sandbox resolves the
  name inside this plugin package only.
- Incremental friendliness: validation reads files through `ctx:read_source`,
  so its read-set is recorded and it only re-runs when an input `.mcmeta`
  actually changes.

## What it checks

- `pack.mcmeta`: has a `pack` object; `pack_format` is a positive integer and
  (when pinned) matches `rpp.toml`; `description` is a string or text component.
- Every `*.png.mcmeta`: has an `animation` object; `frametime` is a positive
  integer; `interpolate` is a boolean; `frames` entries are valid frame indices
  or `{ index, time }` records.

This plugin emits no output files; it is a pure correctness gate.

## Usage

```toml
[[plugin]]
source = "path:../plugins/mcmeta-validate"
```

No options.
