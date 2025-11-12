---@alias dataType "structured" | "bytes" | "file"

---@alias ProcessFn<T> fun(meta: FileMeta, data: T, actions: Actions)

---@class Actions
---@field data_type dataType

---@class Processor
---@field data_type dataType
---@field process ProcessFn<any>

local Processor = {}

---Create a structured data processor
---@param process ProcessFn<table>
---@return Processor
function Processor.structured(process)
    return {
        data_type = "structured",
        process = process
    }
end

---Create a raw bytes processor
---@param process ProcessFn<Bytes>
---@return Processor
function Processor.bytes(process)
    return {
        data_type = "bytes",
        process = process
    }
end

---Create a raw file processor
---@param process ProcessFn<{ path: string }>
---@return Processor
function Processor.file(process)
    return {
        data_type = "file",
        process = process
    }
end

return Processor
