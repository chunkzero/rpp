local Filter = require("filter")
local Processor = require("process.processor")

registerProcessorType(
    "json",
    Filter.filename([[^.+\.json$]]),
    "structured"
)

createProcessor(
    "json",
    Processor.structured(
        function(meta, data, actions)
        end
    )
)
