-- hash-rename: fingerprints asset files by content hash.
--
-- It does two things, demonstrating that one plugin can register *both* a
-- processor and a generator:
--
--   1. A **processor** renames each matching file to
--      `<stem>.<xxh3-8>.<ext>` by mutating `file.path`. This is "cache busting":
--      the filename changes whenever the content changes, so downstream caches
--      (CDNs, launchers) never serve a stale asset under a reused name.
--
--   2. A **generator** emits `rename_map.json`, an object mapping each original
--      path to its hashed path, so anything that referenced the old name can be
--      rewritten.
--
-- ## Why the generator derives the map from the output, not processor state
--
-- Processors run on a pool of worker threads, each with its own Lua state, and
-- the engine may serve unchanged files straight from cache without running the
-- processor at all. A module-level table populated by the processor is
-- therefore NOT a reliable place to accumulate the map — different workers see
-- different fragments, and cached files contribute nothing.
--
-- The generator instead rebuilds the map from the *output* listing, which is
-- the only file set the generator host can enumerate (`ctx:files`). By the
-- generator phase the processor has already renamed the files, so `ctx:files`
-- returns the hashed names; the generator recovers each original name with
-- `naming.original_path` (the exact inverse of `naming.hashed_path`). This is
-- deterministic regardless of worker scheduling or caching, and the `ctx:files`
-- read is recorded so the map is rebuilt only when the in-scope file set
-- changes.
--
-- ## Scope
--
-- By default only `assets/*/textures/custom/**/*.png` is fingerprinted (see
-- `naming.lua`): renaming vanilla textures would break the fixed names that
-- models and blockstates reference, while author-owned `custom/` assets are
-- safe. Override with the `files` option.

local rpp = require("rpp")
local naming = require("naming")

local plugin = rpp.plugin()

-- Does `path` fall within the configured `files` patterns?
local function in_scope(options, path)
    for _, pattern in ipairs(naming.patterns(options)) do
        if rpp.path.match(pattern, path) then
            return true
        end
    end
    return false
end

-- The processor is declared with the broadest default scope as its static
-- `files` glob, then narrows to `ctx.options.files` at runtime. (Processor globs
-- are fixed at registration time, before options exist, so the option is
-- applied here instead.)
plugin:processor("fingerprint", {
    files = naming.DEFAULT_FILES,
    -- Run early so later processors (and the rename map) see the final path.
    priority = 10,
}, function(ctx, file)
    if not in_scope(ctx.options, file.path) then
        return -- not in the configured scope; leave untouched.
    end
    file.path = naming.hashed_path(file.path, file.bytes)
end)

plugin:generator("rename-map", function(ctx)
    -- The generator runs after processing, so `ctx:files` lists the *renamed*
    -- output files. We rebuild the map by recovering each original name from its
    -- hashed output name (the inverse of `naming.hashed_path`). This needs only
    -- the output listing — the one thing the generator host can enumerate — and
    -- is exact because the hash infix is a recognizable fixed shape.
    local map = {}
    local count = 0
    local seen = {}

    for _, pattern in ipairs(naming.patterns(ctx.options)) do
        for _, hashed in ipairs(ctx:files(pattern)) do
            if not seen[hashed] then
                seen[hashed] = true
                local original = naming.original_path(hashed)
                if original ~= nil then
                    map[original] = hashed
                    count = count + 1
                end
            end
        end
    end

    -- Emit a stable, pretty-printed map (it is a human-facing artifact).
    ctx:emit("rename_map.json", rpp.json.encode(map, { pretty = true }))
    ctx.log.info("wrote rename_map.json with " .. count .. " entr" .. (count == 1 and "y" or "ies"))
end)

return plugin
