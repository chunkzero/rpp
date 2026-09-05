# grayscale-wasm

A processor plugin whose work is done by a WASIp2 component: every texture PNG
is decoded, converted to luma, and re-encoded by `grayscale.wasm`.

The component is a Rust crate under [`guest/`](guest). Build it once (needs
`rustup target add wasm32-wasip2`), then point a project at this directory:

```bash
just example-wasm            # builds guest/ and copies grayscale.wasm here
```

```toml
[[plugin]]
source = "path:../plugins/grayscale-wasm"
```

`grayscale.wasm` is not checked in; the build fails with a clear error if it
is missing. The WIT world is a single function:

```wit
export grayscale: func(input: list<u8>) -> result<list<u8>, string>;
```

`list<u8>` maps to a Lua string in both directions, so the Lua side is just
`file.bytes = grayscale:call("grayscale", file.bytes).ok`.
