-- ui/layout.lua - Pure Lua constraint solver
--
-- Provides Rect operations and constraint resolution for layout computation.
-- Replaces src/ui/layout.rs (~1000 lines) with simpler Lua implementation.
--
-- Rect operations:
--   layout.rect(x, y, w, h) -> rect table
--   layout.full(cols, rows) -> rect for full terminal
--   layout.sub(r, x, y, w, h) -> clipped sub-rect
--   layout.shrink(r, top, right, bottom, left) -> shrunk rect
--   layout.split_vertical(r, at) -> top, bottom rects
--   layout.split_horizontal(r, at) -> left, right rects
--
-- Constraint parsing:
--   layout.parse(v) -> {type="absolute"|"percent", value=n}
--   layout.resolve(c, parent_size) -> pixels
--   layout.resolve_position(c, parent_size) -> position (handles negatives)

local M = {}

-- ==========================================================================
-- Rect operations
-- ==========================================================================

--- Create a new rectangle
---@param x number X position (0-indexed)
---@param y number Y position (0-indexed)
---@param w number Width
---@param h number Height
---@return table rect {x, y, w, h, right(), bottom(), contains(), sub(), shrink()}
function M.rect(x, y, w, h)
    return {
        x = x,
        y = y,
        w = w,
        h = h,
    }
end

--- Create a rect for the full terminal
---@param cols number Terminal width
---@param rows number Terminal height
---@return table rect
function M.full(cols, rows)
    return M.rect(0, 0, cols, rows)
end

--- Get right edge of rect
---@param r table rect
---@return number
function M.right(r)
    return r.x + r.w
end

--- Get bottom edge of rect
---@param r table rect
---@return number
function M.bottom(r)
    return r.y + r.h
end

--- Check if rect contains a point
---@param r table rect
---@param x number
---@param y number
---@return boolean
function M.contains(r, x, y)
    return x >= r.x and x < M.right(r) and y >= r.y and y < M.bottom(r)
end

--- Create a clipped sub-rect within a parent rect
---@param r table parent rect
---@param x number relative x offset
---@param y number relative y offset
---@param w number desired width
---@param h number desired height
---@return table sub-rect (clipped to parent bounds)
function M.sub(r, x, y, w, h)
    -- Clamp width and height to not exceed parent bounds
    local max_w = math.max(0, r.w - x)
    local max_h = math.max(0, r.h - y)
    return M.rect(
        r.x + x,
        r.y + y,
        math.min(w, max_w),
        math.min(h, max_h)
    )
end

--- Shrink rect by margins
---@param r table rect
---@param top number top margin
---@param right number right margin (optional, defaults to top)
---@param bottom number bottom margin (optional, defaults to top)
---@param left number left margin (optional, defaults to right)
---@return table shrunk rect
function M.shrink(r, top, right, bottom, left)
    right = right or top
    bottom = bottom or top
    left = left or right
    return M.rect(
        r.x + left,
        r.y + top,
        math.max(0, r.w - left - right),
        math.max(0, r.h - top - bottom)
    )
end

--- Split rect vertically at a row offset
---@param r table rect
---@param at number row to split at (relative to rect)
---@return table top_rect
---@return table bottom_rect
function M.split_vertical(r, at)
    local clamped = math.min(at, r.h)
    local top = M.rect(r.x, r.y, r.w, clamped)
    local bottom = M.rect(r.x, r.y + clamped, r.w, math.max(0, r.h - clamped))
    return top, bottom
end

--- Split rect horizontally at a column offset
---@param r table rect
---@param at number column to split at (relative to rect)
---@return table left_rect
---@return table right_rect
function M.split_horizontal(r, at)
    local clamped = math.min(at, r.w)
    local left = M.rect(r.x, r.y, clamped, r.h)
    local right = M.rect(r.x + clamped, r.y, math.max(0, r.w - clamped), r.h)
    return left, right
end

-- ==========================================================================
-- Constraint parsing and resolution
-- ==========================================================================

--- Parse a constraint value
---@param v number|string Value to parse (number for absolute, "50%" for percent)
---@return table|nil constraint {type="absolute"|"percent", value=n}
function M.parse(v)
    if type(v) == "number" then
        return { type = "absolute", value = v }
    elseif type(v) == "string" then
        if v:match("%%$") then
            local pct = tonumber(v:sub(1, -2))
            if pct then
                return { type = "percent", value = pct / 100 }
            end
        else
            local n = tonumber(v)
            if n then
                return { type = "absolute", value = n }
            end
        end
    end
    return nil
end

--- Resolve a constraint to absolute pixels
---@param c table constraint from parse()
---@param parent_size number parent dimension
---@return number pixels
function M.resolve(c, parent_size)
    if not c then return 0 end
    if c.type == "absolute" then
        return c.value
    else -- percent
        return math.floor(parent_size * c.value)
    end
end

--- Resolve a position constraint (handles negative = from end)
---@param c table constraint from parse()
---@param parent_size number parent dimension
---@return number position (always >= 0)
function M.resolve_position(c, parent_size)
    local val = M.resolve(c, parent_size)
    if val < 0 then
        return math.max(0, parent_size + val)
    else
        return val
    end
end

-- ==========================================================================
-- Bar-specific layout computation
-- ==========================================================================

--- Compute layout for bars stacking from edges
---
--- Bars stack from edges inward based on position and priority:
--- - top bars: higher priority = closer to top edge, stack downward
--- - bottom bars: higher priority = closer to bottom edge, stack upward
--- - content gets the remaining space in the middle
---
---@param cols number terminal width
---@param rows number terminal height
---@param bars table array of bar definitions {name, position, priority, height, condition}
---@param state table current state (passed to condition functions)
---@return table layout {[name] = {row=n, height=n}, content={row=n, height=n}}
function M.compute_bars(cols, rows, bars, state)
    -- Filter to visible bars
    local visible = {}
    for _, bar in ipairs(bars) do
        if not bar.condition or bar.condition(state) then
            table.insert(visible, bar)
        end
    end

    -- Separate by position
    local top_bars = {}
    local bottom_bars = {}
    for _, bar in ipairs(visible) do
        if bar.position == "top" then
            table.insert(top_bars, bar)
        else
            table.insert(bottom_bars, bar)
        end
    end

    -- Sort by priority (descending - higher priority closer to edge)
    table.sort(top_bars, function(a, b) return (a.priority or 0) > (b.priority or 0) end)
    table.sort(bottom_bars, function(a, b) return (a.priority or 0) > (b.priority or 0) end)

    -- Assign rows from edges inward
    local result = {}
    local top_cursor = 0

    for _, bar in ipairs(top_bars) do
        local height = bar.height or 1
        result[bar.name] = { row = top_cursor, height = height }
        top_cursor = top_cursor + height
    end

    local bottom_cursor = rows - 1

    for _, bar in ipairs(bottom_bars) do
        local height = bar.height or 1
        result[bar.name] = { row = bottom_cursor - height + 1, height = height }
        bottom_cursor = bottom_cursor - height
    end

    -- Content gets the middle
    local content_height = math.max(0, bottom_cursor - top_cursor + 1)
    result.content = { row = top_cursor, height = content_height }

    return result
end

return M
