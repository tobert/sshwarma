--- commands/history.lua - History command handlers
---
--- Commands for viewing chat history, tool calls, and usage statistics.
--- Uses luafun for iteration and util for shared formatters.

local page = require('page')
local fun = require('fun')
local util = require('util')

local M = {}

--------------------------------------------------------------------------------
-- /history [n] - View recent chat messages
-- /history --tools - View tool call history
-- /history --stats - View tool usage statistics
--------------------------------------------------------------------------------

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
        page.show("History", "No messages in history.\n\nStart chatting or @mention a model!")
        return {}
    end

    local lines = {string.format("Last %d messages:\n\n", #messages)}

    fun.iter(messages):each(function(_, msg)
        local author = msg.author or "unknown"
        local content = msg.content or ""
        local time_str = util.format_time(msg.timestamp)

        -- Add icons for different message types
        local icon = ""
        if msg.is_model then
            icon = "  "  -- model response
        elseif msg.is_thinking then
            icon = "  "  -- thinking
        end

        -- Truncate long messages and collapse newlines
        local preview = util.truncate(content, 60):gsub("\r?\n", " ")

        table.insert(lines, string.format("%s%s (%s): %s\n", icon, author, time_str, preview))
    end)

    page.show("History", table.concat(lines))
    return {}
end

--------------------------------------------------------------------------------
-- /history --tools - View recent tool calls
--------------------------------------------------------------------------------

function M.history_tools(args)
    local limit = 20
    local n = tonumber(args:match("^%s*(%d+)"))
    if n then
        limit = math.max(1, math.min(50, n))
    end

    local calls = tools.history_tools(limit)

    if #calls == 0 then
        page.show("Tool History", "No tool calls in history.\n\nTool calls will appear here when models use MCP tools.")
        return {}
    end

    local lines = {string.format("Last %d tool calls:\n\n", #calls)}

    fun.iter(calls):each(function(_, call)
        local tool_name = call.tool or "unknown"
        local time_str = util.format_time(call.timestamp)
        local status = call.success and "" or ""

        table.insert(lines, string.format("%s %s (%s)\n", status, tool_name, time_str))

        -- Show args if present
        if call.args then
            local args_preview = util.truncate(call.args, 50)
            table.insert(lines, string.format("   Args: %s\n", args_preview))
        end

        -- Show result preview if present
        if call.result then
            local result_preview = util.truncate(call.result, 50):gsub("\r?\n", " ")
            table.insert(lines, string.format("   Result: %s\n", result_preview))
        end

        table.insert(lines, "\n")
    end)

    page.show("Tool History", table.concat(lines))
    return {}
end

--------------------------------------------------------------------------------
-- /history --stats - View tool usage statistics
--------------------------------------------------------------------------------

function M.history_stats(args)
    local stats = tools.history_stats()

    if stats.total == 0 then
        page.show("Tool Stats", "No tool usage yet.\n\nTool statistics will appear here after models use MCP tools.")
        return {}
    end

    local lines = {
        "Tool Usage Statistics\n\n",
        string.format("Total calls: %d\n\n", stats.total)
    }

    if stats.tools and #stats.tools > 0 then
        table.insert(lines, "By tool:\n")

        -- Find max count for bar scaling
        local max_count = fun.iter(stats.tools)
            :map(function(_, entry) return entry.count end)
            :max()

        -- Create bar chart
        fun.iter(stats.tools):each(function(_, entry)
            local bar_width = 20
            local filled = math.floor((entry.count / max_count) * bar_width)
            local bar = string.rep("", filled) .. string.rep("", bar_width - filled)
            table.insert(lines, string.format("  %s %s %d\n", bar, entry.name, entry.count))
        end)
    end

    page.show("Tool Stats", table.concat(lines))
    return {}
end

return M
