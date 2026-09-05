# hash-rename

An example **rpp** Lua plugin that fingerprints asset files by content hash —
"cache busting" — and records the renames in a map.

## What it demonstrates

- A **generator** that renames files (`gem.png` → `gem.1a2b3c4d.png`) and emits
  `rename_map.json`, using `rpp.hash.xxh3` and `rpp.path.*`.
- **Plugin-local `require`**: `naming.lua` holds the one true naming function,
  shared by the generator and any tooling that needs the naming convention.
- Honouring a `files` option (a list of globs) to scope which files are renamed.

## Why renaming happens in the generator

The generator reads the processed output snapshot, hashes those final bytes,
emits each fingerprinted path, and removes the original. This matters when an
earlier processor transforms a file: the map always names the file that is
actually written. Generator reads and mutations are recorded, so warm builds
replay the same rename without relying on state shared across worker threads.

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
