-- MCP tool registration init script
-- This runs at MCP server startup to register Lua-defined tools

local M = {}

--- Register a single tool from a module
--- @param tool_def table Tool definition with name, description, schema, module_path
local function register_single_tool(tool_def)
    tools.register_mcp_tool({
        name = tool_def.name,
        description = tool_def.description,
        schema = tool_def.schema,
        module_path = tool_def.module_path,
        handler_name = tool_def.handler_name  -- Optional: for multi-tool modules
    })
    tools.log_info("📦 Registered MCP tool: " .. tool_def.name)
end

--- Register a tool module with the MCP server
--- Supports both single-tool modules (M.tool + M.handler) and
--- multi-tool modules (M.tools array with handler_name references)
--- @param mod table Module with tool definition(s) and handler(s)
local function register_tool(mod)
    -- Single tool module: { tool = {...}, handler = function }
    if mod.tool and mod.handler then
        register_single_tool(mod.tool)
    end

    -- Multi-tool module: { tools = [{...}, {...}], handler1 = fn, handler2 = fn }
    if mod.tools then
        for _, tool_def in ipairs(mod.tools) do
            register_single_tool(tool_def)
        end
    end
end

--- Initialize all MCP tools
--- Called at server startup
function M.init()
    -- Wave 1: Read-only tools
    register_tool(require('mcp.rooms'))
    register_tool(require('mcp.models'))
    register_tool(require('mcp.help'))

    -- Wave 2: Tree-building tools
    register_tool(require('mcp.rows'))
    register_tool(require('mcp.row'))

    -- Wave 3: Say with @mention support
    register_tool(require('mcp.say'))

    -- Wave 4: Room mutation tools
    register_tool(require('mcp.create_room'))
    register_tool(require('mcp.set_vibe'))
    register_tool(require('mcp.add_exit'))
    register_tool(require('mcp.fork_room'))
    register_tool(require('mcp.room_context'))

    -- Wave 5: Scripts/Things tools
    register_tool(require('mcp.inventory'))
    register_tool(require('mcp.things'))
    register_tool(require('mcp.scripts'))

    -- Echo test tool for debugging
    register_tool(require('mcp.echo_test'))
end

-- Initialize on load
M.init()

-- Optional mcp_init function called after script loads
function mcp_init()
    tools.log_info("🚀 MCP tools registered successfully")
end

return M
