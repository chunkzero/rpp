# hash-rename

An **rpp** TypeScript plugin that fingerprints asset files by content hash
("cache busting") and records the renames in a map.

## What it demonstrates

- A **generator** that renames files (`gem.png` to `gem.1a2b3c4d.png`) and emits
  `rename_map.json`, using the SDK's `hash.xxh3` and `path.*`.
- `overrides` in `rpp.json`: the generator may replace or remove files owned by
  sources and other plugins.
- A config module whose `normalize` fills in the default `files` option.

## Why renaming happens in the generator

The generator reads the processed output snapshot, hashes those final bytes,
emits each fingerprinted path, and removes the original. This matters when an
earlier processor transforms a file: the map always names the file that is
actually written. Generator reads and mutations are recorded, so warm builds
replay the same rename without relying on state shared across worker threads.

## Options

| Option  | Type          | Default                                 | Meaning                     |
| ------- | ------------- | --------------------------------------- | --------------------------- |
| `files` | list of globs | `["assets/*/textures/custom/**/*.png"]` | Which files to fingerprint. |

The default deliberately targets an author-owned `custom/` texture subtree:
renaming a vanilla texture would break the fixed name a model or blockstate
points at, but `custom/` assets are referenced only through the rename map.

```ts
import hashRename from "#plugins/hash-rename";

export default defineConfig({
  pack: { name: "my-pack" },
  plugins: [hashRename({ files: ["assets/*/textures/custom/**/*.png"] })],
});
```

## Output

`rename_map.json`, for example:

```json
{
  "assets/minecraft/textures/custom/gem.png": "assets/minecraft/textures/custom/gem.1a2b3c4d.png"
}
```
