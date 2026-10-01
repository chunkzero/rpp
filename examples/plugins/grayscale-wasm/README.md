# grayscale-wasm

A processor plugin whose work is done by a WASIp2 component: every texture PNG
is decoded, converted to luma, and re-encoded by `grayscale.wasm`.

The component is a Rust crate under [`guest/`](guest). Build it once (needs
`rustup target add wasm32-wasip2`), then point a project at this directory:

```bash
just example-wasm            # builds guest/ and copies grayscale.wasm here
```

`rpp.json`:

```json
{ "dependencies": { "grayscale-wasm": "path:../plugins/grayscale-wasm" } }
```

`rpp.config.ts` lists `plugin("grayscale-wasm")` in `plugins`.

`grayscale.wasm` is not checked in; the build fails with a clear error if it
is missing. The package declares it in `rpp.json` as `"components": { "grayscale":
"grayscale.wasm" }`. The WIT world is a single function:

```wit
export grayscale: func(input: list<u8>) -> result<list<u8>, string>;
```

`list<u8>` maps to a `Uint8Array` in both directions, and a `result` unwraps to its
`ok` value or throws, so the plugin is just
`file.bytes = components.load("grayscale").exports.grayscale(file.bytes)`.
