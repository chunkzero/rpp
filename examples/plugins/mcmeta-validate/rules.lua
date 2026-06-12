-- rules.lua: the validation logic for mcmeta-validate, kept in a separate
-- module to demonstrate plugin-local `require`. The entry script pulls this in
-- with `require("rules")`; rpp resolves it to `rules.lua` inside this plugin
-- package's directory (and nowhere else — the sandbox forbids escaping).

local rpp = require("rpp")

local M = {}

-- Collect human-readable problems into a list so the caller can report them all
-- at once instead of failing on the first.
local function checker()
    local problems = {}
    return {
        problems = problems,
        check = function(cond, message)
            if not cond then
                problems[#problems + 1] = message
            end
        end,
    }
end

local function is_positive_integer(v)
    return type(v) == "number" and v > 0 and v == math.floor(v)
end

-- Validate the top-level `pack.mcmeta`.
--
-- `data` is the decoded JSON table, `expected_format` is `ctx.pack.format`
-- (may be nil when the project does not pin a pack_format).
-- Returns a list of problem strings (empty == valid).
function M.validate_pack(data, expected_format)
    local c = checker()

    c.check(type(data) == "table", "pack.mcmeta must be a JSON object")
    if type(data) ~= "table" then
        return c.problems
    end

    local pack = data.pack
    c.check(type(pack) == "table", "pack.mcmeta is missing the `pack` object")
    if type(pack) ~= "table" then
        return c.problems
    end

    c.check(is_positive_integer(pack.pack_format), "pack.pack_format must be a positive integer")
    c.check(
        type(pack.description) == "string" or type(pack.description) == "table",
        "pack.description must be a string or a JSON text component"
    )

    -- When the project pins a pack_format in rpp.toml, the manifest must agree.
    if expected_format ~= nil and is_positive_integer(pack.pack_format) then
        c.check(
            pack.pack_format == expected_format,
            string.format(
                "pack.pack_format (%d) does not match rpp.toml pack_format (%d)",
                pack.pack_format,
                expected_format
            )
        )
    end

    return c.problems
end

-- Validate a single `*.png.mcmeta` animation metadata file.
--
-- See https://minecraft.wiki/w/Resource_pack#Animation for the schema. We check
-- the structurally important fields rather than every optional one.
function M.validate_animation(path, data)
    local c = checker()

    c.check(type(data) == "table", path .. ": must be a JSON object")
    if type(data) ~= "table" then
        return c.problems
    end

    local anim = data.animation
    c.check(type(anim) == "table", path .. ": missing `animation` object")
    if type(anim) ~= "table" then
        return c.problems
    end

    if anim.frametime ~= nil then
        c.check(
            is_positive_integer(anim.frametime),
            path .. ": animation.frametime must be a positive integer"
        )
    end

    if anim.interpolate ~= nil then
        c.check(
            type(anim.interpolate) == "boolean",
            path .. ": animation.interpolate must be a boolean"
        )
    end

    if anim.frames ~= nil then
        c.check(type(anim.frames) == "table", path .. ": animation.frames must be an array")
        if type(anim.frames) == "table" then
            for i, frame in ipairs(anim.frames) do
                if type(frame) == "table" then
                    -- `{ index = N, time = T }` form.
                    c.check(
                        is_positive_integer(frame.index) or frame.index == 0,
                        path .. ": animation.frames[" .. i .. "].index must be a non-negative integer"
                    )
                    if frame.time ~= nil then
                        c.check(
                            is_positive_integer(frame.time),
                            path .. ": animation.frames[" .. i .. "].time must be a positive integer"
                        )
                    end
                else
                    -- Bare integer frame index.
                    c.check(
                        is_positive_integer(frame) or frame == 0,
                        path .. ": animation.frames[" .. i .. "] must be a non-negative integer"
                    )
                end
            end
        end
    end

    return c.problems
end

return M
