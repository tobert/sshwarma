-- sshwarma MCP command handlers
--
-- Lua command handlers for MCP (Model Context Protocol):
--   mcp   - List MCP servers and their status
--   tools - List available tools from connected servers
--   run   - Invoke an MCP tool directly
--
-- Each handler receives args string and returns:
--   {text = "output", mode = "overlay"|"notification", title = "Title"}

local M = {}

--------------------------------------------------------------------------------
-- Helper: connection status indicator
--------------------------------------------------------------------------------

local function status_indicator(connected)
    return connected and "+" or "o"
end

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

        if not result or not result.servers then
            return {
                text = "No MCP servers connected.\n\n" ..
                       "Usage:\n" ..
                       "  /mcp connect <name> <url>\n" ..
                       "  /mcp disconnect <name>",
                mode = "overlay",
                title = "MCP"
            }
        end

        if #result.servers == 0 then
            return {
                text = "No MCP servers connected.\n\n" ..
                       "Usage:\n" ..
                       "  /mcp connect <name> <url>\n" ..
                       "  /mcp disconnect <name>",
                mode = "overlay",
                title = "MCP"
            }
        end

        local lines = {}
        table.insert(lines, "Connected MCP servers:")

        for _, server in ipairs(result.servers) do
            local status = status_indicator(server.connected)
            local info = string.format("  %s %s ... %d tools @ %s",
                status, server.name, server.tool_count or 0, server.endpoint or "?")
            table.insert(lines, info)

            if server.error then
                table.insert(lines, string.format("    Error: %s", server.error))
            end
        end

        return {
            text = table.concat(lines, "\n"),
            mode = "overlay",
            title = "MCP"
        }

    elseif subcmd == "connect" or subcmd == "add" then
        -- Connect to server
        local name, url = rest:match("^%s*(%S+)%s+(%S+)%s*$")

        if not name or not url then
            return {
                text = "Usage: /mcp connect <name> <url>",
                mode = "notification"
            }
        end

        tools.mcp_add(name, url)

        return {
            text = string.format("Connecting to MCP server '%s' at %s (background)", name, url),
            mode = "notification"
        }

    elseif subcmd == "disconnect" or subcmd == "remove" then
        -- Disconnect from server
        local name = rest:match("^%s*(%S+)%s*$")

        if not name then
            return {
                text = "Usage: /mcp disconnect <name>",
                mode = "notification"
            }
        end

        local removed = tools.mcp_remove(name)

        if removed then
            return {
                text = string.format("Removed MCP server '%s'", name),
                mode = "notification"
            }
        else
            return {
                text = string.format("MCP server '%s' not found", name),
                mode = "notification"
            }
        end

    elseif subcmd == "refresh" then
        -- Refresh tool list
        local name = rest:match("^%s*(%S+)%s*$")

        if not name then
            return {
                text = "Usage: /mcp refresh <name>",
                mode = "notification"
            }
        end

        -- Note: tools.mcp_refresh might not exist, fall back to status check
        local status = tools.mcp_status and tools.mcp_status(name)

        if not status then
            return {
                text = string.format("MCP server '%s' not found", name),
                mode = "notification"
            }
        end

        return {
            text = string.format("Server '%s': %s, %d tools",
                name, status.state or "unknown", status.tools or 0),
            mode = "notification"
        }

    else
        return {
            text = string.format("Unknown MCP command: %s\n\n" ..
                   "Available commands:\n" ..
                   "  /mcp              List servers\n" ..
                   "  /mcp connect <name> <url>\n" ..
                   "  /mcp disconnect <name>\n" ..
                   "  /mcp refresh <name>", subcmd),
            mode = "overlay",
            title = "MCP"
        }
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

    if not result or not result.tools then
        return {
            text = "No tools available. Use /mcp connect <name> <url> to add an MCP server.",
            mode = "overlay",
            title = "Tools"
        }
    end

    if #result.tools == 0 then
        local msg = "No tools available"
        if server_filter then
            msg = msg .. string.format(" from server '%s'", server_filter)
        end
        return {
            text = msg .. ".\n\nUse /mcp connect <name> <url> to add an MCP server.",
            mode = "overlay",
            title = "Tools"
        }
    end

    local lines = {}
    local title = server_filter
        and string.format("Tools from %s", server_filter)
        or "Available tools"
    table.insert(lines, title .. ":")

    for _, tool in ipairs(result.tools) do
        table.insert(lines, string.format("  %s (%s)", tool.name, tool.server))
        if tool.description and tool.description ~= "" then
            -- Truncate long descriptions
            local desc = tool.description
            if #desc > 60 then
                desc = desc:sub(1, 57) .. "..."
            end
            table.insert(lines, string.format("    %s", desc))
        end
    end

    return {
        text = table.concat(lines, "\n"),
        mode = "overlay",
        title = "Tools"
    }
end

--------------------------------------------------------------------------------
-- /run <tool> [json args] - Invoke MCP tool
--------------------------------------------------------------------------------

function M.run(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /run <tool> [json args]\n" ..
                   'Example: /run orpheus_generate {"temperature": 1.0}',
            mode = "overlay",
            title = "Run Tool"
        }
    end

    -- Parse: <tool_name> [json_args]
    local tool_name, json_args = args:match("^%s*(%S+)%s*(.*)$")

    if not tool_name then
        return {
            text = "Usage: /run <tool> [json args]",
            mode = "notification"
        }
    end

    -- Parse JSON args if provided
    local tool_args = {}
    if json_args and not json_args:match("^%s*$") then
        -- Try to parse as JSON (simple approach)
        -- Note: In production, use a proper JSON parser
        local ok, parsed = pcall(function()
            -- Use Lua's load for simple JSON-like tables
            -- This is a simplified approach; real JSON parsing would be better
            return load("return " .. json_args)()
        end)

        if ok and type(parsed) == "table" then
            tool_args = parsed
        else
            return {
                text = string.format("Invalid JSON: %s", json_args),
                mode = "notification"
            }
        end
    end

    -- Call the tool via MCP bridge
    -- Note: tools.mcp_call is async, need to handle appropriately
    local request_id = tools.mcp_call and tools.mcp_call(nil, tool_name, tool_args)

    if not request_id then
        return {
            text = string.format("Error: Could not call tool '%s'", tool_name),
            mode = "notification"
        }
    end

    -- For now, return immediately with pending status
    -- Real implementation would poll for result
    return {
        text = string.format("Invoked %s (request: %s)\n\nResult will appear when complete.",
            tool_name, tostring(request_id)),
        mode = "overlay",
        title = "Tool Output"
    }
end

return M
