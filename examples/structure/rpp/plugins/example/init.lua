local rpp = require "rpp"

local plugin = rpp.plugin("id", 123)

addProcessor(plugin, {
    filter = "",
    process = function()
        return "hi"
    end
})

return plugin
