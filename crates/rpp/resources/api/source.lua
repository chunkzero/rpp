---@class Source
---@field meta FileMeta
---@field data_type dataType
---@field data any

local Source = {}

---@param meta FileMeta
---@param data_type dataType
---@param data any
---@return Source
function Source.new(meta, data_type, data)
    return {
        meta = meta,
        data_type = data_type,
        data = data
    }
end

return Source
