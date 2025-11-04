--- @param id string
--- @param version number
--- @return Plugin
local function plugin(id, version)
    return { id = id, version = version }
end

return {
    plugin = plugin
}
