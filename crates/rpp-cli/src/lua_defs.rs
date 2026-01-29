use anyhow::Result;
use std::fs;
use std::path::Path;

/// Generate LuaLS type definitions for RPP plugin API.
pub fn generate_lua_definitions(output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir)?;

    let def_path = output_dir.join("rpp.lua");
    let definitions = generate_definitions();

    fs::write(def_path, definitions)?;

    Ok(())
}

fn generate_definitions() -> String {
    r#"---@meta

---@class JsonApi
---@field decode fun(json: string): table|string|number|boolean|nil
---@field encode fun(value: table|string|number|boolean|nil): string
---@field encode_compact fun(value: table|string|number|boolean|nil): string
local json = {}

--- Decode a JSON string into a Lua value.
---@param json string The JSON string to decode
---@return table|string|number|boolean|nil value The decoded value
function json.decode(json) end

--- Encode a Lua value to pretty-printed JSON.
---@param value table|string|number|boolean|nil The value to encode
---@return string json The JSON string
function json.encode(value) end

--- Encode a Lua value to compact JSON (no whitespace).
---@param value table|string|number|boolean|nil The value to encode
---@return string json The compact JSON string
function json.encode_compact(value) end

---@class HashApi
---@field xxhash3 fun(data: string): number
---@field sha256 fun(data: string): string
---@field md5 fun(data: string): string
local hash = {}

--- Fast non-cryptographic hash using xxHash3_64.
---@param data string The data to hash
---@return number hash The 64-bit hash value
function hash.xxhash3(data) end

--- SHA-256 cryptographic hash (lowercase hex).
---@param data string The data to hash
---@return string hash The hex-encoded SHA-256 hash
function hash.sha256(data) end

--- MD5 hash (lowercase hex). For compatibility only, not cryptographically secure.
---@param data string The data to hash
---@return string hash The hex-encoded MD5 hash
function hash.md5(data) end

---@class LogApi
---@field debug fun(message: string)
---@field info fun(message: string)
---@field warn fun(message: string)
---@field error fun(message: string)
local log = {}

--- Log a debug message.
---@param message string The message to log
function log.debug(message) end

--- Log an info message.
---@param message string The message to log
function log.info(message) end

--- Log a warning message.
---@param message string The message to log
function log.warn(message) end

--- Log an error message.
---@param message string The message to log
function log.error(message) end

---@class Input
---@field path string The relative path of the file being processed
---@field content string The file content as a string
local input = {}

---@class ProcessResult
---@field action "continue"|"skip"|"cancel" The action to take
---@field content string|nil The modified content (required if action is "continue")
---@field path string|nil Optional new output path (relative to output directory)
local result = {}

---@class Context
---@field json JsonApi JSON encoding/decoding operations
---@field hash HashApi Hashing functions
---@field log LogApi Logging functions
local ctx = {}

---@class Plugin
local plugin = {}

--- Process a file. Called by RPP for each matching file.
---@param ctx Context The context with available APIs
---@param input Input The input file information
---@return ProcessResult result The processing result
function plugin.process(ctx, input) end

return plugin
"#.to_string()
}