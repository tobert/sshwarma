--- help.lua - Help system for sshwarma
--- @module help

local str = require 'str'

local M = {}

--- Topic registry with descriptions
--- Each topic maps to an embedded help doc and brief description
local topics = {
    -- Libraries
    fun     = { doc = "help.fun",     desc = "Functional programming, lazy iterators" },
    str     = { doc = "help.str",     desc = "String utilities (split, strip, join)" },
    inspect = { doc = "help.inspect", desc = "Pretty-print tables for debugging" },
    -- System
    tools   = { doc = "help.tools",   desc = "MCP tool reference and patterns" },
    -- Features
    room    = { doc = "help.room",    desc = "Room navigation, vibes, exits" },
    journal = { doc = "help.journal", desc = "Notes, decisions, milestones, ideas" },
}

--- Get help for a topic
--- @param topic string|nil Topic name, or nil for topic list
--- @return string|nil Help text or nil if unknown topic
--- @return string|nil Error message if topic unknown
function M.help(topic)
    if not topic then
        return M.list_formatted()
    end

    local entry = topics[topic]
    if not entry then
        return nil, "Unknown topic: " .. topic .. ". Try help() for list."
    end

    return sshwarma.get_embedded_module(entry.doc)
end

--- List available topics with descriptions
--- @return table Array of {name, description}
function M.list()
    local result = {}
    for name, entry in pairs(topics) do
        result[#result + 1] = { name = name, description = entry.desc }
    end
    -- Sort by name for consistent output
    table.sort(result, function(a, b) return a.name < b.name end)
    return result
end

--- Format topic list for display
--- @return string Formatted topic list with usage hint
function M.list_formatted()
    local lines = { "Available help topics:", "" }

    -- Get sorted topics
    local sorted = M.list()
    for _, item in ipairs(sorted) do
        lines[#lines + 1] = string.format("  %-10s  %s", item.name, item.description)
    end

    lines[#lines + 1] = ""
    lines[#lines + 1] = "Usage: help('<topic>') or /help <topic>"
    return str.join(lines, "\n")
end

--- Add a new topic (for extensibility)
--- @param name string Topic name
--- @param doc string Embedded module path (e.g. "help.tools")
--- @param desc string Brief description
function M.register(name, doc, desc)
    topics[name] = { doc = doc, desc = desc }
end

return M
