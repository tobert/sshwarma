-- History command handlers for sshwarma
--
-- Commands for viewing chat history, tool calls, and usage statistics.
-- Each handler receives args (string) and returns {text, mode, title?}

local M = {}

-- Format timestamp as relative time
local function format_time(ts)
    local now = os.time()
    local diff = now - math.floor(ts / 1000)  -- ts is in milliseconds

    if diff < 60 then
        return "just now"
    elseif diff < 3600 then
        local mins = math.floor(diff / 60)
        return string.format("%dm ago", mins)
    elseif diff < 86400 then
        local hours = math.floor(diff / 3600)
        return string.format("%dh ago", hours)
    else
        local days = math.floor(diff / 86400)
        return string.format("%dd ago", days)
    end
end

-- Truncate string with ellipsis
local function truncate(s, max)
    if not s then return "" end
    if #s <= max then return s end
    return s:sub(1, max - 3) .. "..."
end

-- /history [n] - View recent chat messages
-- /history --tools - View tool call history
-- /history --stats - View tool usage statistics
function M.history(args)
    local trimmed = args:match("^%s*(.-)%s*$")

    -- Check for flags
    if trimmed == "--tools" or trimmed == "-t" then
        return M.history_tools("")
    elseif trimmed == "--stats" or trimmed == "-s" then
        return M.history_stats("")
    end

    -- Parse limit
    local limit = 20
    if trimmed ~= "" then
        local n = tonumber(trimmed)
        if n then
            limit = math.max(1, math.min(100, n))
        end
    end

    local messages = tools.history(limit)

    if #messages == 0 then
        return {
            text = "No messages in history.\r\n\r\nStart chatting or @mention a model!",
            mode = "overlay",
            title = "History"
        }
    end

    local lines = {}
    table.insert(lines, string.format("Last %d messages:\r\n\r\n", #messages))

    for i, msg in ipairs(messages) do
        local author = msg.author or "unknown"
        local content = msg.content or ""
        local time_str = format_time(msg.timestamp)

        -- Add icons for different message types
        local icon = ""
        if msg.is_model then
            icon = "  "  -- model response
        elseif msg.is_thinking then
            icon = "  "  -- thinking
        end

        -- Truncate long messages for display
        local preview = truncate(content, 60)
        preview = preview:gsub("\r?\n", " ")  -- Collapse newlines

        table.insert(lines, string.format("%s%s (%s): %s\r\n", icon, author, time_str, preview))
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "History"
    }
end

-- /history --tools - View recent tool calls
function M.history_tools(args)
    local limit = 20
    local n = tonumber(args:match("^%s*(%d+)"))
    if n then
        limit = math.max(1, math.min(50, n))
    end

    local calls = tools.history_tools(limit)

    if #calls == 0 then
        return {
            text = "No tool calls in history.\r\n\r\nTool calls will appear here when models use MCP tools.",
            mode = "overlay",
            title = "Tool History"
        }
    end

    local lines = {}
    table.insert(lines, string.format("Last %d tool calls:\r\n\r\n", #calls))

    for i, call in ipairs(calls) do
        local tool_name = call.tool or "unknown"
        local time_str = format_time(call.timestamp)
        local status = call.success and "" or ""

        table.insert(lines, string.format("%s %s (%s)\r\n", status, tool_name, time_str))

        -- Show args if present
        if call.args then
            local args_preview = truncate(call.args, 50)
            table.insert(lines, string.format("   Args: %s\r\n", args_preview))
        end

        -- Show result preview if present
        if call.result then
            local result_preview = truncate(call.result, 50)
            result_preview = result_preview:gsub("\r?\n", " ")
            table.insert(lines, string.format("   Result: %s\r\n", result_preview))
        end

        table.insert(lines, "\r\n")
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Tool History"
    }
end

-- /history --stats - View tool usage statistics
function M.history_stats(args)
    local stats = tools.history_stats()

    if stats.total == 0 then
        return {
            text = "No tool usage yet.\r\n\r\nTool statistics will appear here after models use MCP tools.",
            mode = "overlay",
            title = "Tool Stats"
        }
    end

    local lines = {}
    table.insert(lines, string.format("Tool Usage Statistics\r\n\r\n"))
    table.insert(lines, string.format("Total calls: %d\r\n\r\n", stats.total))

    if stats.tools and #stats.tools > 0 then
        table.insert(lines, "By tool:\r\n")

        -- Create a bar chart
        local max_count = 0
        for _, entry in ipairs(stats.tools) do
            if entry.count > max_count then
                max_count = entry.count
            end
        end

        for _, entry in ipairs(stats.tools) do
            local bar_width = 20
            local filled = math.floor((entry.count / max_count) * bar_width)
            local bar = string.rep("", filled) .. string.rep("", bar_width - filled)

            table.insert(lines, string.format("  %s %s %d\r\n", bar, entry.name, entry.count))
        end
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Tool Stats"
    }
end

return M
