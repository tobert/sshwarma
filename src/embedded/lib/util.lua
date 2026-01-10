--- util.lua - Shared utilities for sshwarma Lua code
---
--- Provides common helpers used across commands and UI modules.
--- Consolidates direction tables, formatting functions, and session helpers.
---
--- Copyright (c) 2025 Andrew Tobey
--- MIT License (see LICENSE)

local M = {}

--------------------------------------------------------------------------------
-- Direction Tables
--------------------------------------------------------------------------------

--- Direction glyphs for exits display
M.DIR_GLYPHS = {
    north = "^", south = "v", east = ">", west = "<",
    up = "^", down = "v",
    n = "^", s = "v", e = ">", w = "<", u = "^", d = "v"
}

--- Direction normalization (short -> canonical)
M.DIR_NORMALIZE = {
    n = "north", s = "south", e = "east", w = "west",
    u = "up", d = "down",
    north = "north", south = "south", east = "east", west = "west",
    up = "up", down = "down"
}

--- Opposite directions (for bidirectional exits)
M.DIR_OPPOSITE = {
    north = "south", south = "north",
    east = "west", west = "east",
    up = "down", down = "up"
}

--------------------------------------------------------------------------------
-- Formatting Helpers
--------------------------------------------------------------------------------

--- Format timestamp as relative time.
--- @param ts number Timestamp in milliseconds
--- @return string Relative time string (e.g., "5m ago")
function M.format_time(ts)
    local now = os.time()
    local diff = now - math.floor(ts / 1000)

    if diff < 60 then
        return "just now"
    elseif diff < 3600 then
        return string.format("%dm ago", math.floor(diff / 60))
    elseif diff < 86400 then
        return string.format("%dh ago", math.floor(diff / 3600))
    else
        return string.format("%dd ago", math.floor(diff / 86400))
    end
end

--- Truncate string with ellipsis.
--- @param s string|nil The string to truncate
--- @param max number Maximum length
--- @param suffix string|nil Suffix to add if truncated (default: "...")
--- @return string Truncated string
function M.truncate(s, max, suffix)
    if not s then return "" end
    suffix = suffix or "..."
    if #s <= max then return s end
    return s:sub(1, max - #suffix) .. suffix
end

--------------------------------------------------------------------------------
-- Session Helpers
--------------------------------------------------------------------------------

--- Get current agent ID from session.
--- @return string|nil Agent UUID or nil if not logged in
function M.get_agent_id()
    local session = tools.session()
    return session and session.agent_id
end

--- Get current room ID from session.
--- @return string|nil Room UUID or nil if not in a room
function M.get_room_id()
    local session = tools.session()
    return session and session.room_id
end

--------------------------------------------------------------------------------
-- Status Indicators
--------------------------------------------------------------------------------

--- Get a status indicator character.
--- @param connected boolean Whether the item is connected/active
--- @return string Status indicator ("+" for active, "o" for inactive)
function M.status_indicator(connected)
    return connected and "+" or "o"
end

return M
