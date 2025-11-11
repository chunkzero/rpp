---@alias ProcessStopResult { type: "stop" }
---@alias ProcessContinueResult { type: "continue", data_type: dataType, data: any }
---@alias ProcessResult ProcessStopResult | ProcessContinueResult

local Result = {}

---@return ProcessStopResult
function Result.stop()
    return {
        type = "stop"
    }
end

return Result
