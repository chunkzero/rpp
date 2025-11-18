---@meta _

---Registers a processor type
---@param id string
---@param filter Filter
---@param data_type dataType
function registerProcessorType(id, filter, data_type) end

---Registers a processor type
---@param processor_type_id string The processor type to register to
---@param processor Processor
function createProcessor(processor_type_id, processor) end
