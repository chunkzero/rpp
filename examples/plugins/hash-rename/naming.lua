-- naming.lua: the single source of truth for how hash-rename derives a hashed
-- output path from a file's path and contents. Both the processor (which renames
-- files) and the generator (which emits the rename map) require this module, so
-- they are guaranteed to agree byte-for-byte.
--
-- This also demonstrates plugin-local `require`: `require("naming")` resolves to
-- this file inside the hash-rename package directory.

local rpp = require("rpp")

local M = {}

-- The default patterns when the `files` option is omitted/empty. We target a
-- `custom/` texture subtree on purpose: renaming vanilla textures would break
-- the model/blockstate references that point at fixed names, but author-owned
-- `custom/` assets are safe to fingerprint (the rename map records the mapping).
M.DEFAULT_FILES = { "assets/*/textures/custom/**/*.png" }

-- Build the patterns list from the plugin options, falling back to the default.
function M.patterns(options)
    local files = options and options.files
    if type(files) == "table" and #files > 0 then
        return files
    end
    return M.DEFAULT_FILES
end

-- Compute the hashed output path for `path` given its `bytes`.
--
-- `logo.png` with content hash `deadbeef…` becomes `logo.deadbeef.png`. The
-- hash is the first 8 hex chars of xxh3 — short, collision-safe enough for
-- cache-busting asset names, and stable across builds for identical content.
function M.hashed_path(path, bytes)
    local digest = rpp.hash.xxh3(bytes):sub(1, 8)
    local dir = rpp.path.dirname(path)
    local base = rpp.path.basename(path)
    local ext = rpp.path.ext(path)

    -- Strip the extension from the basename to get the stem.
    local stem = base
    if ext ~= "" then
        stem = base:sub(1, #base - #ext - 1)
    end

    local new_base
    if ext ~= "" then
        new_base = stem .. "." .. digest .. "." .. ext
    else
        new_base = stem .. "." .. digest
    end

    if dir ~= "" then
        return dir .. "/" .. new_base
    end
    return new_base
end

-- Is `s` an 8-character lowercase hex string (the shape of our hash infix)?
local function is_hash8(s)
    return type(s) == "string" and #s == 8 and s:match("^[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]$") ~= nil
end

-- Recover the original path from a hashed one, or return nil if `path` does not
-- look like one of our outputs. `stem.<hash8>.ext` -> `stem.ext`.
--
-- The generator uses this to rebuild the rename map purely from the (renamed)
-- output set, which is the only file listing it can see — the inverse of
-- `hashed_path`. It is exact because the hash infix is a fixed, recognizable
-- shape that never collides with a normal filename segment.
function M.original_path(path)
    local dir = rpp.path.dirname(path)
    local base = rpp.path.basename(path)
    local ext = rpp.path.ext(path)

    local without_ext = base
    if ext ~= "" then
        without_ext = base:sub(1, #base - #ext - 1)
    end

    -- Split off the last dotted segment of the stem; that should be the hash.
    local stem, maybe_hash = without_ext:match("^(.*)%.([^%.]+)$")
    if stem == nil or not is_hash8(maybe_hash) then
        return nil
    end

    local original_base
    if ext ~= "" then
        original_base = stem .. "." .. ext
    else
        original_base = stem
    end

    if dir ~= "" then
        return dir .. "/" .. original_base
    end
    return original_base
end

return M
