-- hash-rename: fingerprints asset files by content hash.
--
-- A processor renames each matching file to `<stem>.<xxh3-8>.<ext>` by
-- mutating `file.path`, so downstream caches never serve stale bytes under a
-- reused name. A generator then emits `rename_map.json`, mapping original
-- paths to hashed paths, so references can be rewritten.
--
-- The generator derives the map from the *source* files rather than from
-- processor state: processors run on worker threads and are skipped entirely
-- for cached files, so a module-level table would see only fragments. Reading
-- the sources through `ctx` also records them as dependencies, so the map is
-- rebuilt exactly when an in-scope file changes.

local rpp = require("rpp")
local naming = require("naming")

local plugin = rpp.plugin()

local function in_scope(options, path)
    for _, pattern in ipairs(naming.patterns(options)) do
        if rpp.path.match(pattern, path) then
            return true
        end
    end
    return false
end

-- Processor globs are fixed at registration, before options exist, so the
-- broadest default is declared here and narrowed to `ctx.options.files` at
-- runtime.
plugin:processor("fingerprint", {
    files = naming.DEFAULT_FILES,
    -- Run early so later processors (and the rename map) see the final path.
    priority = 10,
}, function(ctx, file)
    if not in_scope(ctx.options, file.path) then
        return
    end
    file.path = naming.hashed_path(file.path, file.bytes)
end)

plugin:generator("rename-map", function(ctx)
    local map = {}
    local count = 0
    for _, pattern in ipairs(naming.patterns(ctx.options)) do
        for _, path in ipairs(ctx:source_files(pattern)) do
            if map[path] == nil then
                map[path] = naming.hashed_path(path, ctx:read_source(path))
                count = count + 1
            end
        end
    end
    ctx:emit("rename_map.json", rpp.json.encode(map, { pretty = true }))
    ctx.log.info("wrote rename_map.json with " .. count .. " entr" .. (count == 1 and "y" or "ies"))
end)

return plugin
