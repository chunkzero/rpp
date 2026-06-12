# grayscale-wasm

An example [rpp](../../../README.md) WASM plugin (a `rpp:plugin@0.1.0` WASIp2
component) built on `wit-bindgen`.

## What it does

* Declares one processor, `grayscale`, matching `**/*.gray.png` with priority 0.
* For each matched file it decodes the PNG, converts it to 8-bit grayscale,
  re-encodes it, and **renames** the output by stripping the `.gray` infix
  (`logo.gray.png` becomes `logo.png`). This is reported as `Modified` with a
  new path.
* Exports a generator that calls the host `emit-file` to write
  `grayscale_report.json` listing how many files were converted.

## Building

This crate is **not** part of the rpp workspace (it has its own empty
`[workspace]` table) so it can target WebAssembly independently. Build the
component with:

```bash
cargo build --release --target wasm32-wasip2
```

The resulting component is at
`target/wasm32-wasip2/release/grayscale_wasm.wasm`. Rename or copy it to
`plugin.wasm` next to `plugin.toml` to use it as a plugin package:

```bash
cp target/wasm32-wasip2/release/grayscale_wasm.wasm plugin.wasm
```

> Requires the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`.

## Plugin package

`plugin.toml` declares `runtime = "wasm"` and `module = "plugin.wasm"`, so the
directory is a complete rpp plugin package once `plugin.wasm` is present.
