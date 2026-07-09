# Lua Plugin Authoring

Lua plugins are directories containing `plugin.toml`, an entry script, and any
plugin-local modules.

```toml
[plugin]
id = "my-plugin"
version = "1.0.0"
entry = "init.lua"
```

The entry must be a normalized relative path inside the package.

## Processors

```lua
local rpp = require("rpp")
local plugin = rpp.plugin()

plugin:processor("compact", {
    files = { "**/*.json", "**/*.mcmeta" },
    priority = 50,
}, function(ctx, file)
    file.text = rpp.json.encode(rpp.json.decode(file.text))
end)

return plugin
```

Processors may mutate `file.path`, `file.bytes`/`file.text`, or call
`file:drop()`. Paths must be normalized relative forward-slash paths.

Processors must be deterministic functions of the file, options, and pack
metadata. Do not accumulate module-level state, use time/randomness, or depend
on worker ordering: incremental builds may skip a processor entirely.
Sandboxed Lua therefore omits `os` and `math.random` by default. Trusted
`clocks`/`random` grants expose them and make that plugin non-replayable.

`ctx` exposes `options`, `pack`, and `log`.

## Generators

```lua
plugin:generator("index", function(ctx)
    local index = {}
    for _, path in ipairs(ctx:files("assets/*/textures/**/*.png")) do
        index[#index + 1] = { path = path, hash = rpp.hash.xxh3(ctx:read(path)) }
    end
    ctx:emit("texture_index.json", rpp.json.encode(index))
end)
```

Generator methods are `files`, `source_files`, `read`, `read_source`,
`load_source`, `emit`, `emit_output`, and `remove`. Reads observe an immutable
snapshot taken before that generator starts, so a generator does not read back
its own mutations. Later generators observe earlier generators' final output.

`files(glob)` enumerates processed outputs, while `source_files(glob)` enumerates
raw source files. Both return deterministic, sorted paths and record the list as
an incremental dependency. `read_source` and `load_source` record the contents of
the exact source path they read.

`emit_output(root, path, contents)` writes to a named project-configured output
root. Declared roots remain available in the default sandboxed mode:

```toml
[[plugin]]
source = "path:plugins/my-plugin"

[plugin.outputs]
kotlin = "../server/src/main/kotlin/generated"
```

## Components, Hooks, And Modules

Plugins can ship named WASIp2 components:

```toml
[component.compiler]
module = "compiler.wasm"
```

```lua
local compiler = require("rpp.component").load("compiler")
local result = compiler:call("compile", "input")
```

Processor component calls receive a fresh guest instance per file. This keeps
guest globals and deterministic WASI random streams independent of worker/file
ordering. Sequential generator and hook calls may reuse their component instance.

See [`WASM_PLUGINS.md`](WASM_PLUGINS.md) for component details.

`plugin:on_start(fn)` and `plugin:on_finish(fn)` register lifecycle hooks.
`require("helpers")` resolves `helpers.lua` or `helpers/init.lua` within the
plugin package.

Builtins are `rpp.json`, `rpp.toml`, `rpp.hash`, `rpp.path`, `rpp.log`,
`rpp.str`, `rpp.component`, and `rpp.process`. Sandboxed plugins do not get
`io`, `os`, `math.random`, `debug`, dynamic loading, C module loading,
unrestricted filesystem access, or process execution. Trusted/native modes can
grant more capability through project config. Lua states have memory and
execution limits. Tracebacks name plugin-local and source modules with stable
package-relative paths.

See [`examples/plugins`](../examples/plugins) for complete plugins.
