-- mcmeta-validate: a generator that validates the pack's `.mcmeta` files.
--
-- Why a generator and not a processor? Processors are pure and per-file; they
-- cannot see the whole pack or cross-reference `ctx.pack`. Validation that
-- spans the project (e.g. "pack_format must match rpp.toml") belongs in the
-- generator phase, which runs once after processing with access to `ctx:files`,
-- `ctx:read`, `ctx:read_source` and `ctx.pack`.
--
-- This plugin emits no files. It either passes quietly (with a summary log) or
-- raises `error()` to fail the build with an attributed message. The validation
-- rules themselves live in `rules.lua` to demonstrate plugin-local `require`.

local rpp = require("rpp")
local rules = require("rules")

local plugin = rpp.plugin()

-- Decode JSON, raising a descriptive error tied to `path` on failure.
local function decode_or_fail(ctx, path, text)
    local ok, value = pcall(rpp.json.decode, text)
    if not ok then
        error(path .. ": not valid JSON: " .. tostring(value))
    end
    return value
end

plugin:generator("validate", function(ctx)
    local all_problems = {}
    local function add(problems)
        for _, p in ipairs(problems) do
            all_problems[#all_problems + 1] = p
        end
    end

    -- 1. The top-level manifest. We read the *source* copy so validation does
    --    not depend on whatever processors did to the output. This read is
    --    recorded in the generator's read-set, so the build re-validates only
    --    when pack.mcmeta actually changes.
    local pack_text = ctx:read_source("pack.mcmeta")
    if pack_text == nil then
        error("pack.mcmeta is missing from the pack source")
    end
    local pack_data = decode_or_fail(ctx, "pack.mcmeta", pack_text)
    add(rules.validate_pack(pack_data, ctx.pack.format))

    -- 2. Every texture animation metadata file, enumerated from the source
    --    tree so renaming or dropping processors cannot hide a file from
    --    validation.
    local animation_count = 0
    for _, path in ipairs(ctx:source_files("**/*.png.mcmeta")) do
        local data = decode_or_fail(ctx, path, ctx:read_source(path))
        add(rules.validate_animation(path, data))
        animation_count = animation_count + 1
    end

    -- 3. Report. Any problem fails the build with all problems listed.
    if #all_problems > 0 then
        error(
            "mcmeta validation failed:\n  - " .. table.concat(all_problems, "\n  - ")
        )
    end

    ctx.log.info(
        string.format(
            "validated pack.mcmeta and %d animation file(s); all OK",
            animation_count
        )
    )
end)

return plugin
