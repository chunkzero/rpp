-- grayscale-wasm: a processor that hands each PNG to a WASIp2 component.
--
-- Heavy per-file work (here, PNG decode/encode) lives in the component; Lua
-- only routes bytes. Processor component calls get a fresh guest instance per
-- file, so the guest may keep whatever state it likes.

local rpp = require("rpp")
local grayscale = require("rpp.component").load("grayscale")

local plugin = rpp.plugin()

plugin:processor("grayscale", {
    files = { "assets/*/textures/**/*.png" },
    -- Run before hash-rename (priority 10) so fingerprints see the final bytes.
    priority = 5,
}, function(ctx, file)
    local result = grayscale:call("grayscale", file.bytes)
    if result.err ~= nil then
        error(file.path .. ": " .. result.err)
    end
    file.bytes = result.ok
end)

return plugin
