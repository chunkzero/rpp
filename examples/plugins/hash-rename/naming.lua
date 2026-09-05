-- naming.lua: how hash-rename derives a hashed output path from a file's path
-- and contents. Both the processor (which renames files) and the generator
-- (which emits the rename map) require this module, so they agree byte-for-byte.
-- `require("naming")` resolves to this file inside the plugin package.

local rpp = require("rpp")

local M = {}

-- Default patterns when the `files` option is omitted. Renaming vanilla
-- textures would break the fixed names models and blockstates reference, so
-- only author-owned `custom/` assets are fingerprinted.
M.DEFAULT_FILES = { "assets/*/textures/custom/**/*.png" }

function M.patterns(options)
    local files = options and options.files
    if type(files) == "table" and #files > 0 then
        return files
    end
    return M.DEFAULT_FILES
end

-- `logo.png` with content hash `deadbeef...` becomes `logo.deadbeef.png`.
function M.hashed_path(path, bytes)
    local digest = rpp.hash.xxh3(bytes):sub(1, 8)
    local dir = rpp.path.dirname(path)
    local base = rpp.path.basename(path)
    local ext = rpp.path.ext(path)

    local stem = base
    if ext ~= "" then
        stem = base:sub(1, #base - #ext - 1)
    end
    local new_base = stem .. "." .. digest
    if ext ~= "" then
        new_base = new_base .. "." .. ext
    end
    if dir ~= "" then
        return dir .. "/" .. new_base
    end
    return new_base
end

return M
