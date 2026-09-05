# json-minify

A minimal, well-commented example **rpp** Lua processor plugin. It re-encodes
every `.json` and `.mcmeta` file in the pack, collapsing insignificant
whitespace so the shipped pack is as small as possible.

## What it demonstrates

- A single **processor** (`minify`) declared with a `files` glob list and a
  `priority`. Processors are pure — they see only `(ctx, file)` and have no
  filesystem access — which is what makes per-file incremental caching sound.
- Mutating `file.text` to change a file's contents. The runtime detects the
  mutation and reports the file as `Modified`.
- Reading plugin options through `ctx.options`.
- Defensive decoding with `pcall`: a malformed JSON file is logged via
  `ctx.log.warn` and passed through untouched instead of aborting the build.

## Options

| Option   | Type    | Default | Meaning                                              |
| -------- | ------- | ------- | ---------------------------------------------------- |
| `pretty` | boolean | `false` | When `true`, re-indent instead of minifying.         |

```toml
[[plugin]]
source = "path:../plugins/json-minify"
[plugin.options]
pretty = false
```

## Notes

`rpp build` also has a built-in JSON squash step (`[build.squash] json = true`),
so on a real pack this plugin is somewhat redundant with squash. It is included
because it is the smallest possible *realistic* processor and a good template
for writing your own.
