-- Hash-based File Renamer
-- Renames files based on their content hash for cache-busting

return {
    name = "hash_renamer",
    version = "1.0.0",
    type = "processor",

    -- Match PNG textures
    patterns = {"**/*.png"},

    -- Run after other processors
    priority = 200,

    process = function(ctx, input)
        -- Compute hash of content
        local hash = ctx.hash.xxhash3(input.content)

        -- Extract directory and filename
        local dir = input.path:match("(.*/)")  or ""
        local basename = input.path:match("[^/]+$"):match("(.+)%..+") or "file"
        local ext = input.path:match("%.([^.]+)$") or "png"

        -- Create new path with hash
        local new_path = string.format("%s%s.%s.%s", dir, basename, hash, ext)

        ctx.log.info("Renaming: " .. input.path .. " → " .. new_path)

        return {
            action = "continue",
            content = input.content,
            path = new_path
        }
    end
}
