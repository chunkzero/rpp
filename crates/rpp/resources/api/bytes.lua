---@meta _

---@class Bytes
---@field private __index table
---@operator len: integer
---@field len fun(self: Bytes): integer
---@field get fun(self: Bytes, idx: integer): integer
---@field push fun(self: Bytes, val: integer)
---@field set fun(self: Bytes, idx: integer, val: integer)
---@field remove fun(self: Bytes, idx: integer): integer
---@field clear fun(self: Bytes)
---@field resize fun(self: Bytes, len: integer, default: integer)

Bytes = {}

---Create a new Bytes buffer
---@param capacity? integer Optional initial capacity
---@return Bytes
function Bytes.new(capacity) end
