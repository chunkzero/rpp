-- JSON Minifier Plugin
-- Removes whitespace from JSON files to reduce pack size

return {
    name = "json_minify",
    version = "1.0.0",
    type = "processor",

    -- Match all JSON files
    patterns = {"**/*.json"},

    -- Run early (lower priority = earlier)
    priority = 50,

    process = function(ctx, input)
        ctx.log.info("Minifying JSON: " .. input.path)

        -- Decode JSON
        local json_data = ctx.json.decode(input.content)

        -- Re-encode as compact JSON
        local minified = ctx.json.encode_compact(json_data)

        -- Calculate size savings
        local original_size = #input.content
        local new_size = #minified
        local savings = original_size - new_size
        local percent = (savings / original_size) * 100

        ctx.log.info(string.format("  Saved %d bytes (%.1f%%)", savings, percent))

        return {
            action = "continue",
            content = minified
        }
    end
}
