# hash-rename

An example **rpp** Lua plugin that fingerprints asset files by content hash —
"cache busting" — and records the renames in a map.

## What it demonstrates

- A single plugin registering **both a processor and a generator**.
- A **processor** that renames files by mutating `file.path`
  (`gem.png` → `gem.1a2b3c4d.png`), using `rpp.hash.xxh3`, `rpp.path.*`.
- A **generator** that emits `rename_map.json` mapping every original path to
  its hashed path.
- **Plugin-local `require`**: `naming.lua` holds the one true naming function,
  shared by both the processor and generator so they cannot disagree.
- Honouring a `files` option (a list of globs) to scope which files are renamed.

## Why the generator derives the map from the output

Processors are **pure and parallel**: they run on a pool of worker threads, each
with its own Lua state, and the engine skips them entirely for files served from
the incremental cache. A module-level table populated inside the processor would
therefore be unreliable — each worker sees only the fragment it processed, and
cached files contribute nothing.

So the map is **not** accumulated in the processor. Instead the generator
rebuilds it from the build output. The generator host can only enumerate output
files (`ctx:files`), and by the generator phase those files have already been
renamed — so `ctx:files` returns the hashed names. The generator recovers each
original name from its hashed name with `naming.original_path`, the exact
inverse of `naming.hashed_path` (the hash infix is a fixed, recognizable
8-hex-char segment). This is deterministic regardless of worker scheduling or
caching, and the `ctx:files` read is recorded so `rename_map.json` is rebuilt
only when the in-scope output set changes.

## Options

| Option  | Type            | Default                                  | Meaning                                   |
| ------- | --------------- | ---------------------------------------- | ----------------------------------------- |
| `files` | list of globs   | `["assets/*/textures/custom/**/*.png"]`  | Which files to fingerprint.               |

The default deliberately targets an author-owned `custom/` texture subtree:
renaming a vanilla texture would break the fixed name a model or blockstate
points at, but `custom/` assets are referenced only through the rename map.

```toml
[[plugin]]
source = "path:../plugins/hash-rename"
[plugin.options]
files = ["assets/*/textures/custom/**/*.png"]
```

## Output

`rename_map.json`, for example:

```json
{
  "assets/minecraft/textures/custom/gem.png": "assets/minecraft/textures/custom/gem.1a2b3c4d.png"
}
```
