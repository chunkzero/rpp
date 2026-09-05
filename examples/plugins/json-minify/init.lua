-- json-minify: a pure processor that re-encodes JSON and `.mcmeta` files
-- compactly (or pretty, if asked). It is a faithful, well-commented example of
-- the rpp Lua processor API (see docs/SPEC.md §4).
--
-- Key ideas demonstrated here:
--   * `plugin:processor(name, opts, fn)` with a `files` glob list.
--   * Reading and writing `file.text`; mutation is what marks the file Modified.
--   * Reading plugin options via `ctx.options`.
--   * Defensive parsing: some packs ship intentionally quirky JSON, so a decode
--     failure is logged with `ctx.log.warn` and the file is passed through
--     unchanged rather than aborting the whole build.

local rpp = require("rpp")
local plugin = rpp.plugin()

plugin:processor("minify", {
    -- Minecraft resource packs use `.json` for models/blockstates/lang/etc. and
    -- `.mcmeta` for pack metadata and texture animation metadata. Both are JSON.
    files = { "**/*.json", "**/*.mcmeta" },
    -- Run late so other plugins (which may emit or rewrite JSON) see whitespace
    -- before we collapse it.
    priority = 100,
}, function(ctx, file)
    -- `pretty = true` re-indents instead of minifying; handy for diffing a built
    -- pack. Defaults to compact output.
    local pretty = ctx.options.pretty == true

    -- pcall so a single malformed file cannot fail the build.
    local ok, decoded = pcall(rpp.json.decode, file.text)
    if not ok then
        ctx.log.warn("skipping " .. file.path .. ": invalid JSON (" .. tostring(decoded) .. ")")
        return
    end

    -- Re-encode. Compact form drops all insignificant whitespace; the pretty
    -- form is stable two-space indentation from serde_json.
    file.text = rpp.json.encode(decoded, { pretty = pretty })
end)

return plugin
