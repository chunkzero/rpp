# Lua Plugin Authoring

Lua plugins are directories containing `plugin.toml`, an entry script, and any
plugin-local modules.

```toml
[plugin]
id = "my-plugin"
version = "1.0.0"
runtime = "lua"
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

Generator methods are `files`, `read`, `read_source`, `emit`, and `remove`.
Reads observe an immutable snapshot taken before that generator starts, so a
generator does not read back its own mutations. Later generators observe
earlier generators' final output.

Generators can enumerate processed outputs only. Raw source files can be read
by exact path but cannot be enumerated. Reads are recorded for incremental
invalidation.

## Hooks And Modules

`plugin:on_start(fn)` and `plugin:on_finish(fn)` register lifecycle hooks.
`require("helpers")` resolves `helpers.lua` or `helpers/init.lua` within the
plugin package.

Builtins are `rpp.json`, `rpp.toml`, `rpp.hash`, `rpp.path`, `rpp.log`, and
`rpp.str`. There is no `io`, `debug`, dynamic loading, C module loading, or
unrestricted filesystem access. Lua states have memory and execution limits.

See [`examples/plugins`](../examples/plugins) for complete plugins.
