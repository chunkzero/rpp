# json-minify

A minimal **rpp** TypeScript processor plugin. It re-encodes every `.json` and
`.mcmeta` file in the pack, collapsing insignificant whitespace so the shipped
pack is as small as possible.

## What it demonstrates

- A single **processor** (`minify`) declared with `files` globs and a
  `priority`. Processors are pure: they see only `(ctx, file)` and have no
  filesystem access, which is what makes per-file incremental caching sound.
- Assigning `file.text` to change a file's contents. Assignment marks the file as modified.
- Typed options through a config module (`src/config.ts`, declared as `config` in
  `rpp.json`) built with `definePluginConfig`.
- Defensive decoding: a malformed JSON file is passed through untouched instead
  of aborting the build.

## Options

| Option   | Type    | Default | Meaning                                      |
| -------- | ------- | ------- | -------------------------------------------- |
| `pretty` | boolean | `false` | When `true`, re-indent instead of minifying. |

`rpp.json`:

```json
{ "dependencies": { "json-minify": "path:../plugins/json-minify" } }
```

`rpp.config.ts`:

```ts
import jsonMinify from "#plugins/json-minify";

export default defineConfig({
  pack: { name: "my-pack" },
  plugins: [jsonMinify({ pretty: false })],
});
```

## Notes

`rpp build` also has a built-in JSON squash step (`build.squash.json`), so on a
real pack this plugin is somewhat redundant with squash. It is included because
it is the smallest realistic processor and a good template for your own.
