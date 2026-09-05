local rpp = require("rpp")
local plugin = rpp.plugin()

-- Dropped authoring files remain available through the source APIs.
plugin:processor("authoring-only", { files = { "items/*.lua" } }, function(ctx, file)
    file:drop()
end)

plugin:generator("catalog", function(ctx)
    local namespace = ctx.options.namespace
    assert(type(namespace) == "string" and namespace:match("^[a-z0-9_%-]+$"),
        "namespace must contain lowercase letters, digits, underscores or hyphens")
    local renames = rpp.json.decode(ctx:read("rename_map.json"))
    local function texture(reference)
        local ns, name = reference:match("^([^:]+):(.+)$")
        assert(ns and name, "texture must be a namespaced resource location")
        local path = "assets/" .. ns .. "/textures/" .. name .. ".png"
        path = renames[path] or path
        assert(ctx:read(path), "missing texture: " .. reference)
        local output_ns, output_name = path:match("^assets/([^/]+)/textures/(.+)%.png$")
        return output_ns .. ":" .. output_name
    end

    -- This generator sees hash-rename's outputs and can overwrite existing models.
    for _, path in ipairs(ctx:files("assets/*/models/**/*.json")) do
        local model = rpp.json.decode(ctx:read(path))
        local changed = false
        for key, reference in pairs(model.textures or {}) do
            if reference:sub(1, 1) ~= "#" then
                local resolved = texture(reference)
                if resolved ~= reference then
                    model.textures[key] = resolved
                    changed = true
                end
            end
        end
        if changed then
            ctx:emit(path, rpp.json.encode(model))
        end
    end

    local translations = {}
    local catalog = {}
    for _, path in ipairs(ctx:source_files("items/*.lua")) do
        local item = ctx:load_source(path)
        local id = path:match("^items/([a-z0-9_%-]+)%.lua$")
        assert(id, "item filename must be a lowercase identifier: " .. path)
        assert(type(item) == "table" and type(item.name) == "string"
            and type(item.texture) == "string", path .. " must return name and texture strings")
        local resolved = texture(item.texture)
        local model = namespace .. ":item/" .. id
        local translation = "item." .. namespace .. "." .. id
        translations[translation] = item.name
        ctx:emit("assets/" .. namespace .. "/models/item/" .. id .. ".json", rpp.json.encode({
            parent = "minecraft:item/generated",
            textures = { layer0 = resolved },
        }))
        catalog[id] = { model = model, translation = translation, texture = resolved }
    end
    ctx:emit("assets/" .. namespace .. "/lang/en_us.json", rpp.json.encode(translations))
    ctx:emit_output("catalog", "items.json", rpp.json.encode({
        pack = ctx.pack.name,
        items = catalog,
    }, { pretty = true }))
end)

return plugin
