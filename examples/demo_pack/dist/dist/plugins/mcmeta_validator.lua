-- .mcmeta File Validator
-- Validates animation metadata and logs warnings

return {
    name = "mcmeta_validator",
    version = "1.0.0",
    type = "processor",

    -- Match .mcmeta files
    patterns = {"**/*.mcmeta"},

    -- Run early to validate before other processing
    priority = 10,

    process = function(ctx, input)
        ctx.log.info("Validating: " .. input.path)

        -- Decode JSON
        local data = ctx.json.decode(input.content)

        -- Validate animation metadata
        if data.animation then
            local anim = data.animation

            -- Check frametime
            if anim.frametime and (anim.frametime < 1 or anim.frametime > 100) then
                ctx.log.warn("  Unusual frametime: " .. anim.frametime)
            end

            -- Check frames array
            if anim.frames then
                if #anim.frames == 0 then
                    ctx.log.error("  Empty frames array!")
                else
                    ctx.log.info(string.format("  Animation: %d frames @ %d ticks",
                        #anim.frames, anim.frametime or 1))
                end
            end
        end

        -- Pass through unchanged
        return {
            action = "continue",
            content = input.content
        }
    end
}
