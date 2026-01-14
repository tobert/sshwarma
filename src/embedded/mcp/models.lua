-- mcp/models.lua - List available AI models
-- Read-only MCP tool implementation

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "list_models",
    description = "List available AI models",
    schema = {
        type = "object",
        properties = {},
    },
    module_path = "mcp.models"
}

--- Handler function called when the tool is invoked
--- @param params table The parameters passed to the tool (unused)
--- @return table Array of model information
function M.handler(params)
    local models = tools.list_models()
    local result = {}
    for i, model in ipairs(models) do
        result[i] = {
            short_name = model.short_name,
            display_name = model.display_name,
            backend = model.backend,
            context_window = model.context_window
        }
    end
    return result
end

return M
