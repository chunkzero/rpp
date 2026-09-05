-- hash-rename: fingerprints asset files by content hash.
--
-- A generator renames each matching output to `<stem>.<xxh3-8>.<ext>` and
-- emits `rename_map.json`, mapping original paths to hashed paths so references
-- can be rewritten.
--
-- Hashing happens after all processors, so fingerprints cover the bytes that
-- are actually written. Generator reads and mutations are recorded for cache
-- replay, so no cross-worker state is needed.

local rpp = require("rpp")
local naming = require("naming")

local plugin = rpp.plugin()

plugin:generator("rename-map", function(ctx)
    local map = {}
    local count = 0
    for _, pattern in ipairs(naming.patterns(ctx.options)) do
        for _, path in ipairs(ctx:files(pattern)) do
            if map[path] == nil then
                local bytes = ctx:read(path)
                local hashed = naming.hashed_path(path, bytes)
                ctx:emit(hashed, bytes)
                ctx:remove(path)
                map[path] = hashed
                count = count + 1
            end
        end
    end
    ctx:emit("rename_map.json", rpp.json.encode(map, { pretty = true }))
    ctx.log.info("wrote rename_map.json with " .. count .. " entr" .. (count == 1 and "y" or "ies"))
end)

return plugin
