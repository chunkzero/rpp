# rpp-example-pack

A small but **complete and real** Minecraft resource pack, built end-to-end by
[rpp](../../README.md). It exists to show what an actual rpp project looks like:
a `rpp.toml`, hand-authored `src/` assets, and three local Lua plugins wired into
the build.

## Layout

```
examples/pack/
├── rpp.toml                 # project config: pack metadata, squash, plugins
├── src/
│   ├── pack.mcmeta          # pack_format 34 + description
│   ├── .rppignore           # excludes *.txt (design notes) from the build
│   ├── notes/design.txt     # ignored — never ships
│   └── assets/minecraft/
│       ├── blockstates/rpp_bricks.json
│       ├── lang/en_us.json
│       ├── sounds.json
│       ├── models/
│       │   ├── block/rpp_bricks.json     # deliberately whitespace-heavy
│       │   └── item/diamond_sword.json, magic_gem.json
│       └── textures/
│           ├── block/rpp_bricks.png
│           ├── block/ember.png (+ ember.png.mcmeta animation)
│           ├── custom/gem.png            # fingerprinted by hash-rename
│           └── item/diamond_sword.png
└── tools/gen_textures.py    # regenerates the PNGs (stdlib only; see below)
```

The textures are real 16×16 (and one 16×64 animated) RGBA PNGs. They are
checked in; `tools/gen_textures.py` regenerates them deterministically using only
the Python standard library (no Pillow required).

## What the build does

The pipeline in `rpp.toml` runs three local plugins (in
[`../plugins/`](../plugins)) and then the built-in squash pass:

1. **json-minify** — re-encodes every `.json`/`.mcmeta` compactly. Watch the
   whitespace-heavy `models/block/rpp_bricks.json` collapse to one line.
2. **mcmeta-validate** — validates `pack.mcmeta` (including that its
   `pack_format` matches `rpp.toml`) and the `ember.png.mcmeta` animation. Fails
   the build if anything is wrong.
3. **hash-rename** — fingerprints `custom/gem.png` to
   `custom/gem.<hash>.png` and emits `rename_map.json` so references can be
   rewritten. It is scoped to `custom/` so vanilla texture names (which models
   and blockstates reference by fixed name) are left untouched.
4. **squash** (built in) — minifies JSON, optimizes the PNGs with oxipng, strips
   junk, and writes a deterministic `dist/rpp-example-pack.zip`.

To also try the WASM-backed processor, run `just example-wasm` once and add
`source = "path:../plugins/grayscale-wasm"` as another `[[plugin]]`.

> Note: `magic_gem.json` references `minecraft:custom/gem`. After hash-rename the
> texture file is `gem.<hash>.png`, so a real pack would also rewrite that
> reference using `rename_map.json` — left as an exercise / a job for another
> plugin. The example keeps the two concerns separate on purpose.

## Building it

From this directory:

```bash
cargo run -p rpp-cli -- build
```

Or, if a `just rpp` recipe is configured, run it with this directory as the
working directory. Output lands in `dist/`:

```
dist/
├── assets/...                       # processed + squashed pack contents
├── rename_map.json                  # emitted by hash-rename
└── rpp-example-pack.zip             # deterministic distributable
```

## Regenerating textures

```bash
python3 tools/gen_textures.py
```

This rewrites the PNGs under `src/assets/minecraft/textures`. The script is
deterministic, so committed binaries stay stable across runs.

## Verified by

`crates/rpp/tests/example_plugins.rs` drives the rpp `Engine` directly against
this pack and the three plugins, asserting the JSON is minified, validation
passes, files are hash-renamed with a consistent `rename_map.json`, the
`.rppignore`d notes are absent, and the incremental cache behaves (a clean
second build, and a one-file edit reprocessing only that file).
