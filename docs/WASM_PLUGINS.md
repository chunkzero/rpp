# WASM Plugin Authoring

WASM plugins are WASIp2 components implementing the
`rpp:plugin@0.1.0/rpp-plugin` world in
[`crates/rpp-wasm/wit/plugin.wit`](../crates/rpp-wasm/wit/plugin.wit).

```toml
[plugin]
id = "my-plugin"
version = "1.0.0"
runtime = "wasm"
module = "plugin.wasm"
```

The component's `get-info` id and version must exactly match `plugin.toml`.
Processor names must be non-empty and unique.

The guest exports `configure`, `get-info`, `process`, and `generate`. Processors
return `unchanged`, `modified(file-data)`, or `dropped`. Modified and generated
paths are confined normalized pack paths.

During `generate`, the host can list/read processed files, read an exact raw
source path, emit/remove output files, and log messages.

## Rust Guest

```rust
wit_bindgen::generate!({
    world: "rpp-plugin",
    path: "../../../crates/rpp-wasm/wit",
});
```

Build guests independently from the rpp workspace:

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/my_plugin.wasm plugin.wasm
```

The host provides no filesystem preopens, network access, or environment
variables. Calls have epoch deadlines and store resource limits.

See [`examples/plugins/grayscale-wasm`](../examples/plugins/grayscale-wasm), or
run `just wasm-example` from the repository root.
