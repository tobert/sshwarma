-- ui/scroll.lua - Scroll state per page
--
-- Less-style scrolling:
-- - Content streams in at bottom when following
-- - Scroll up detaches viewport
-- - Jump to bottom reattaches

local M = {}

-- Per-page scroll state keyed by page name
local scroll_states = {}

--- Get scroll state for a page (creates if needed)
---@param page string page name
---@return table state
function M.get(page)
    if not scroll_states[page] then
        scroll_states[page] = {
            offset = 0,
            content_height = 0,
            viewport_height = 0,
            following = true,
        }
    end
    return scroll_states[page]
end

--- Set content height and auto-scroll if following
---@param page string
---@param height number total content lines
function M.set_content_height(page, height)
    local s = M.get(page)
    s.content_height = height
    if s.following then
        s.offset = math.max(0, height - s.viewport_height)
    end
end

--- Set viewport height
---@param page string
---@param height number visible rows
function M.set_viewport_height(page, height)
    local s = M.get(page)
    s.viewport_height = height
end

--- Scroll up by lines
---@param page string
---@param lines number (default 1)
function M.up(page, lines)
    local s = M.get(page)
    s.offset = math.max(0, s.offset - (lines or 1))
    s.following = false
end

--- Scroll down by lines
---@param page string
---@param lines number (default 1)
function M.down(page, lines)
    local s = M.get(page)
    local max_offset = math.max(0, s.content_height - s.viewport_height)
    s.offset = math.min(max_offset, s.offset + (lines or 1))
    s.following = (s.offset >= max_offset)
end

--- Jump to top
---@param page string
function M.to_top(page)
    local s = M.get(page)
    s.offset = 0
    s.following = false
end

--- Jump to bottom (and start following)
---@param page string
function M.to_bottom(page)
    local s = M.get(page)
    s.offset = math.max(0, s.content_height - s.viewport_height)
    s.following = true
end

--- Get visible range [start, end) line indices
---@param page string
---@return number start
---@return number end_
function M.visible_range(page)
    local s = M.get(page)
    return s.offset, s.offset + s.viewport_height
end

--- Get scroll percentage (0.0 - 1.0)
---@param page string
---@return number
function M.percent(page)
    local s = M.get(page)
    if s.content_height <= s.viewport_height then
        return 1.0
    end
    return s.offset / (s.content_height - s.viewport_height)
end

--- Check if following bottom
---@param page string
---@return boolean
function M.is_following(page)
    return M.get(page).following
end

--- Reset scroll state for a page
---@param page string
function M.reset(page)
    scroll_states[page] = nil
end

--- Reset all scroll states
function M.reset_all()
    scroll_states = {}
end

return M
