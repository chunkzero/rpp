# WASM Components In Plugins

rpp plugins are Lua packages. A plugin may also ship named WASIp2 components
and call them from Lua with `rpp.component`.

```toml
[plugin]
id = "my-plugin"
version = "1.0.0"
entry = "init.lua"

[component.compiler]
module = "compiler.wasm"
```

```lua
local component = require("rpp.component").load("compiler")

local result = component:call("compile", "assets/example/input.json")
if result.err ~= nil then
    error(result.err)
end
```

Use `rpp component bindgen` to generate a small Lua wrapper from a component
binary:

```bash
rpp component bindgen compiler.wasm --name compiler --out compiler.lua
```

The generated wrapper calls `rpp.component.load("<name>")`, exposes stable Lua
functions for each exported component function, and includes structural
LuaCATS annotations derived recursively from records, lists, options, variants,
and results.

## Component Shape

Components may export ordinary WIT functions directly from their world:

```wit
package example:compiler;

world compiler {
    record output-file {
        path: string,
        contents: list<u8>,
    }

    export compile: func(input: string) -> result<list<output-file>, string>;
}
```

Lua values are converted from the component type signature. `list<u8>` accepts
and returns Lua strings; records are Lua tables; `result<T, E>` is represented
as `{ ok = value }` or `{ err = value }`. Type/range errors identify the
component export and parameter that failed conversion.

## WASI And Trust

By default, components run without filesystem preopens, network access, passed
environment variables, or process execution. Standard Rust WASI imports such as
closed stdio, environment access with no variables, terminal probing, exit, and
random seed are linkable so ordinary Rust components instantiate. Without a
random grant, random interfaces receive deterministic streams so cached and cold
builds agree; `permissions.random = true` opts into host randomness and disables
replay for that plugin.

Calls from processor callbacks instantiate a fresh guest for each file. This
prevents guest globals or deterministic random-stream position from depending on
worker scheduling. Calls from a sequential generator or hook reuse the handle's
instance, which permits multi-call compiler workflows without cross-file state.

Component binaries participate in the Lua plugin cache key. RPP also caches
Wasmtime compilation by component content in memory and in `.rpp/cache/wasmtime`.
Per-instance memory and per-call time limits come from `[build.wasm]
memory_limit_mb` and `execution_deadline_seconds` in `rpp.toml`.

[`examples/plugins/grayscale-wasm`](../examples/plugins/grayscale-wasm) is a
complete processor plugin with a Rust guest crate.

Project config may grant broader access only outside sandboxed mode:

```toml
[[plugin]]
source = "path:plugins/my-plugin"
security = "trusted"

[plugin.permissions]
read = ["data"]
write = ["generated"]
environment = ["MY_ENV_VAR"]
process = ["my-tool"]
```

`security = "native"` is the unsafe mode: Lua gains native standard-library
access and `rpp.process.run` may launch any program. Use it only for trusted
local tooling.
