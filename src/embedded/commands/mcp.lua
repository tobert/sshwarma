--- commands/mcp.lua - MCP command handlers
---
--- Commands for MCP (Model Context Protocol) management:
---   mcp   - List MCP servers and their status
---   tools - List available tools from connected servers
---   run   - Invoke an MCP tool directly

local page = require('page')
local fun = require('fun')
local util = require('util')

local M = {}

--------------------------------------------------------------------------------
-- /mcp [subcommand] - MCP server management
--
-- /mcp              - List servers
-- /mcp connect <name> <url>  - Connect to server
-- /mcp disconnect <name>     - Disconnect from server
-- /mcp refresh <name>        - Refresh tool list
--------------------------------------------------------------------------------

function M.mcp(args)
    -- Parse subcommand
    local subcmd, rest = "", ""
    if args and not args:match("^%s*$") then
        subcmd, rest = args:match("^%s*(%S+)%s*(.*)$")
        subcmd = subcmd or ""
        rest = rest or ""
    end

    if subcmd == "" or subcmd == "list" then
        -- List servers
        local result = tools.mcp_servers()

        if not result or not result.servers or #result.servers == 0 then
            page.show("MCP", "No MCP servers connected.\n\nUsage:\n  /mcp connect <name> <url>\n  /mcp disconnect <name>")
            return {}
        end

        local lines = {"Connected MCP servers:\n"}

        fun.iter(result.servers):each(function(_, server)
            local status = util.status_indicator(server.connected)
            table.insert(lines, string.format("  %s %s ... %d tools @ %s\n",
                status, server.name, server.tool_count or 0, server.endpoint or "?"))

            if server.error then
                table.insert(lines, string.format("    Error: %s\n", server.error))
            end
        end)

        page.show("MCP", table.concat(lines))
        return {}

    elseif subcmd == "connect" or subcmd == "add" then
        local name, url = rest:match("^%s*(%S+)%s+(%S+)%s*$")
        if not name or not url then
            return { text = "Usage: /mcp connect <name> <url>", mode = "notification" }
        end

        tools.mcp_add(name, url)
        return {
            text = string.format("Connecting to MCP server '%s' at %s (background)", name, url),
            mode = "notification"
        }

    elseif subcmd == "disconnect" or subcmd == "remove" then
        local name = rest:match("^%s*(%S+)%s*$")
        if not name then
            return { text = "Usage: /mcp disconnect <name>", mode = "notification" }
        end

        local removed = tools.mcp_remove(name)
        if removed then
            return { text = string.format("Removed MCP server '%s'", name), mode = "notification" }
        else
            return { text = string.format("MCP server '%s' not found", name), mode = "notification" }
        end

    elseif subcmd == "refresh" then
        local name = rest:match("^%s*(%S+)%s*$")
        if not name then
            return { text = "Usage: /mcp refresh <name>", mode = "notification" }
        end

        local status = tools.mcp_status and tools.mcp_status(name)
        if not status then
            return { text = string.format("MCP server '%s' not found", name), mode = "notification" }
        end

        return {
            text = string.format("Server '%s': %s, %d tools", name, status.state or "unknown", status.tools or 0),
            mode = "notification"
        }

    else
        page.show("MCP", string.format("Unknown MCP command: %s\n\nAvailable commands:\n  /mcp              List servers\n  /mcp connect <name> <url>\n  /mcp disconnect <name>\n  /mcp refresh <name>", subcmd))
        return {}
    end
end

--------------------------------------------------------------------------------
-- /tools [server] - List available tools
--------------------------------------------------------------------------------

function M.tools(args)
    local server_filter = nil
    if args and not args:match("^%s*$") then
        server_filter = args:match("^%s*(%S+)%s*$")
    end

    local result = tools.mcp_tools(server_filter)

    if not result or not result.tools or #result.tools == 0 then
        local msg = "No tools available"
        if server_filter then
            msg = msg .. string.format(" from server '%s'", server_filter)
        end
        page.show("Tools", msg .. ".\n\nUse /mcp connect <name> <url> to add an MCP server.")
        return {}
    end

    local title = server_filter
        and string.format("Tools from %s", server_filter)
        or "Available tools"
    local lines = {title .. ":\n"}

    fun.iter(result.tools):each(function(_, tool)
        table.insert(lines, string.format("  %s (%s)\n", tool.name, tool.server))
        if tool.description and tool.description ~= "" then
            local desc = util.truncate(tool.description, 60)
            table.insert(lines, string.format("    %s\n", desc))
        end
    end)

    page.show("Tools", table.concat(lines))
    return {}
end

--------------------------------------------------------------------------------
-- /run <tool> [args] - Invoke MCP tool
--
-- Note: JSON argument parsing removed for security.
-- Pass a Lua table literal or use without args.
--------------------------------------------------------------------------------

function M.run(args)
    if not args or args:match("^%s*$") then
        page.show("Run Tool", [[
Usage: /run <tool>

Invoke an MCP tool. Arguments are not yet supported from the command line.
Use @model to invoke tools with arguments via natural language.

Example: /run orpheus_generate
]])
        return {}
    end

    -- Just get the tool name, ignore any extra args for now
    local tool_name = args:match("^%s*(%S+)")
    if not tool_name then
        return { text = "Usage: /run <tool>", mode = "notification" }
    end

    -- Call the tool with empty args
    local request_id = tools.mcp_call and tools.mcp_call(nil, tool_name, {})

    if not request_id then
        return {
            text = string.format("Error: Could not call tool '%s'", tool_name),
            mode = "notification"
        }
    end

    page.show("Tool Output", string.format("Invoked %s (request: %s)\n\nResult will appear when complete.",
            tool_name, tostring(request_id)))
    return {}
end

return M
