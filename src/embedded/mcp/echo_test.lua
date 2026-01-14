-- mcp/echo_test.lua - Echo test tool for MCP server
-- This demonstrates how to implement a Lua MCP tool handler

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "echo_test",
    description = "Test tool that echoes params back",
    schema = {
        type = "object",
        properties = {
            message = {
                type = "string",
                description = "Message to echo back"
            }
        }
    },
    module_path = "mcp.echo_test"
}

--- Handler function called when the tool is invoked
--- @param params table The parameters passed to the tool
--- @return table The result to return to the MCP client
function M.handler(params)
    local message = params.message or "(no message provided)"

    return {
        echo = params,
        message = "This is a Lua MCP tool!",
        received_message = message,
        timestamp = os.time()
    }
end

return M
