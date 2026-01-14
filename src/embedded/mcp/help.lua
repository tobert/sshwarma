-- mcp/help.lua - Get help documentation
-- Read-only MCP tool implementation

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "help",
    description = "Get help docs. No topic = list available.",
    schema = {
        type = "object",
        properties = {
            topic = {
                type = "string",
                description = "Help topic to look up (optional - omit for topic list)"
            }
        },
    },
    module_path = "mcp.help"
}

--- Handler function called when the tool is invoked
--- @param params table Parameters with optional topic field
--- @return table|string Help content or topic list
function M.handler(params)
    -- Use the existing help module from embedded Lua
    local help = require('lib.help')

    if params.topic and params.topic ~= "" then
        -- Get specific topic
        local content, err = help.help(params.topic)
        if err then
            return { error = err }
        end
        return {
            topic = params.topic,
            content = content
        }
    else
        -- List all topics
        local topics = help.list()
        return {
            topics = topics,
            usage = "Pass a topic name to get detailed help"
        }
    end
end

return M
