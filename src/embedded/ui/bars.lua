-- ui/bars.lua - Bar system for edge chrome
--
-- Bars are horizontal strips that stack from terminal edges inward.
-- Inspired by weechat's bar system but simplified:
-- - Only top/bottom bars (no left/right)
-- - Items return arrays of segment tables
-- - Layout computed in pure Lua
--
-- Usage:
--   bars.define("status", {
--     position = "top",
--     priority = 100,
--     height = 1,
--     items = {"room_name", "spacer", "mode_indicator"},
--     style = {bg = "#1a1b26"},
--   })
--
--   bars.item("room_name", function(state, width)
--     return {{text = state.room.name, style = {fg = "#7dcfff"}}}
--   end)

local layout = require 'ui.layout'

local M = {}

-- Bar definitions registry {[name] = def}
local bar_defs = {}

-- Item renderer registry {[name] = fn(state, width) -> iterator}
local item_registry = {}

-- ==========================================================================
-- Bar Definition
-- ==========================================================================

--- Define a bar
---@param name string unique bar name
---@param opts table bar options:
---  - position: "top" | "bottom" (default "bottom")
---  - priority: number, higher = closer to edge (default 0)
---  - height: number of rows (default 1)
---  - condition: function(state) -> bool (default: always visible)
---  - items: list of item names to render
---  - style: base style {bg, fg} for bar background
function M.define(name, opts)
    bar_defs[name] = {
        name = name,
        position = opts.position or "bottom",
        priority = opts.priority or 0,
        height = opts.height or 1,
        condition = opts.condition,
        items = opts.items or {},
        style = opts.style or {},
    }
end

--- Get a bar definition by name
---@param name string
---@return table|nil
function M.get(name)
    return bar_defs[name]
end

--- Get all bar definitions as array
---@return table array of bar defs
function M.all()
    local result = {}
    for _, def in pairs(bar_defs) do
        table.insert(result, def)
    end
    return result
end

--- Clear all bar definitions (for testing/reset)
function M.clear()
    bar_defs = {}
end

-- ==========================================================================
-- Item Registration
-- ==========================================================================

--- Register an item renderer
---
--- Render functions receive state and available width, return an iterator
--- of segment tables: {text=, style=} or {spacer=true}
---
---@param name string item name
---@param render_fn function(state, width) -> iterator of segments
function M.item(name, render_fn)
    item_registry[name] = render_fn
end

--- Get an item renderer by name
---@param name string
---@return function|nil
function M.get_item(name)
    return item_registry[name]
end

-- ==========================================================================
-- Layout Computation
-- ==========================================================================

--- Compute bar layout positions
---@param cols number terminal width
---@param rows number terminal height
---@param state table current state
---@return table layout {[name] = {row, height}, content = {row, height}}
function M.compute_layout(cols, rows, state)
    return layout.compute_bars(cols, rows, M.all(), state)
end

-- ==========================================================================
-- Bar Rendering
-- ==========================================================================

--- Render a bar's items to segments
---
--- Returns a flat list of segments with spacers resolved.
---
---@param bar_def table bar definition
---@param width number available width
---@param state table current state
---@return table array of {text, style, x} segments
function M.render_items(bar_def, width, state)
    -- Collect raw segments from all items
    local raw_segments = {}
    local spacer_indices = {}
    local total_fixed_width = 0

    for _, item_name in ipairs(bar_def.items or {}) do
        local render = item_registry[item_name]
        if render then
            local segments = render(state, width)
            if segments then
                for _, seg in ipairs(segments) do
                    if seg.spacer then
                        table.insert(raw_segments, seg)
                        table.insert(spacer_indices, #raw_segments)
                    else
                        local text = seg.text or ""
                        total_fixed_width = total_fixed_width + #text
                        table.insert(raw_segments, seg)
                    end
                end
            end
        end
    end

    -- Distribute remaining space among spacers
    local remaining = math.max(0, width - total_fixed_width)
    local num_spacers = #spacer_indices
    local spacer_width = num_spacers > 0 and math.floor(remaining / num_spacers) or 0
    local extra = num_spacers > 0 and (remaining % num_spacers) or 0

    -- Assign spacer widths
    for i, idx in ipairs(spacer_indices) do
        local w = spacer_width + (i <= extra and 1 or 0)
        raw_segments[idx].width = w
    end

    -- Build final segments with x positions
    local result = {}
    local x = 0
    for _, seg in ipairs(raw_segments) do
        if seg.spacer then
            -- Spacer becomes empty space (could render with style for background)
            if seg.width and seg.width > 0 then
                table.insert(result, {
                    text = string.rep(" ", seg.width),
                    style = bar_def.style,
                    x = x,
                })
                x = x + seg.width
            end
        else
            local text = seg.text or ""
            if #text > 0 then
                table.insert(result, {
                    text = text,
                    style = seg.style or bar_def.style,
                    x = x,
                })
                x = x + #text
            end
        end
    end

    return result
end

--- Render a bar to a draw context
---
--- Fills background and draws item segments.
---
---@param ctx table DrawContext with print/fill methods
---@param bar_def table bar definition
---@param state table current state
function M.render(ctx, bar_def, state)
    -- Fill background
    if bar_def.style and bar_def.style.bg then
        ctx:fill(0, 0, ctx.w, ctx.h, " ", bar_def.style)
    end

    -- Get rendered segments
    local segments = M.render_items(bar_def, ctx.w, state)

    -- Draw each segment
    for _, seg in ipairs(segments) do
        ctx:print(seg.x, 0, seg.text, seg.style)
    end
end

-- ==========================================================================
-- Built-in Items
-- ==========================================================================

-- Spacer: expands to fill available space
M.item("spacer", function(_state, _width)
    return {{spacer = true}}
end)

return M
