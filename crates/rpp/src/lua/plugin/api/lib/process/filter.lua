---@class Filter
---@field type string
---@field regex? string
---@field filters? Filter[]
---@field filter? Filter

local Filter = {}

---Create a path filter
---@param regex string Regular expression to match against file paths
---@return Filter
function Filter.path(regex)
    return {
        type = "path",
        regex = regex,
    }
end

---Create a filename filter
---@param regex string Regular expression to match against file names
---@return Filter
function Filter.filename(regex)
    return {
        type = "filename",
        regex = regex,
    }
end

---Create an ALL filter (all conditions must match)
---@param ... Filter Filters to combine
---@return Filter
function Filter.all(...)
    local filters = { ... }
    return {
        type = "all",
        filters = filters,
    }
end

---Create an ANY filter (any condition must match)
---@param ... Filter Filters to combine
---@return Filter
function Filter.any(...)
    local filters = { ... }
    return {
        type = "any",
        filters = filters,
    }
end

---Create a NEGATE filter (inverts the condition)
---@param filter Filter Filter to negate
---@return Filter
function Filter.negate(filter)
    return {
        type = "negate",
        filter = filter,
    }
end

return Filter
