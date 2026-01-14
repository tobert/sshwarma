-- mcp/scripts.lua - Script listing
-- Wave 5: Scripts/Things MCP tools

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "list_scripts",
    description = "List available Lua scripts",
    schema = {
        type = "object",
        properties = {},
    },
    module_path = "mcp.scripts"
}

--- Handler function called when the tool is invoked
--- @param params table Parameters (unused)
--- @return table List of scripts
function M.handler(params)
    local result = tools.scripts()
    local scripts = result and result.scripts or {}

    local script_list = {}
    for i, script in ipairs(scripts) do
        script_list[i] = {
            id = script.id,
            module_path = script.module_path,
            scope = script.scope,
            description = script.description
        }
    end

    return {
        scripts = script_list,
        count = #script_list
    }
end

return M
