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
local fun = require 'fun'

local M = {}

-- ==========================================================================
-- Width Calculation (Unicode-aware)
-- ==========================================================================

--- Calculate display width of text (handles emoji, CJK, etc.)
---@param str string
---@return number
local function display_width(str)
    if not str or str == "" then return 0 end
    -- tools.display_width MUST be available - no fallback
    assert(tools and tools.display_width,
        "tools.display_width not available - check Lua init order")
    return tools.display_width(str)
end

--- Truncate string to fit within max_width display cells
---@param str string
---@param max_width number
---@return string truncated string
local function truncate_to_width(str, max_width)
    if not str or str == "" then return "" end
    if max_width <= 0 then return "" end

    local width = display_width(str)
    if width <= max_width then return str end

    -- Walk through characters, accumulating width
    local result = {}
    local current_width = 0

    for _, code in utf8.codes(str) do
        local char = utf8.char(code)
        local char_width = display_width(char)

        if current_width + char_width > max_width then
            break
        end

        table.insert(result, char)
        current_width = current_width + char_width
    end

    return table.concat(result)
end

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
    return fun.iter(bar_defs):map(function(_, def) return def end):totable()
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
                        -- Use display_width for proper Unicode width
                        local text_width = display_width(text)
                        seg._width = text_width  -- cache for later
                        total_fixed_width = total_fixed_width + text_width
                        table.insert(raw_segments, seg)
                    end
                end
            end
        end
    end

    -- Always log bar width calculation for debugging
    if tools and tools.log_info then
        local item_widths = {}
        for _, seg in ipairs(raw_segments) do
            if not seg.spacer then
                table.insert(item_widths, string.format("%q=%d", seg.text:sub(1,8), seg._width or 0))
            end
        end
        tools.log_info(string.format(
            "bar %s: total=%d width=%d | %s",
            bar_def.name, total_fixed_width, width, table.concat(item_widths, ", ")
        ))
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

    -- Build final segments with x positions, truncating at width boundary
    local result = {}
    local x = 0
    for _, seg in ipairs(raw_segments) do
        if x >= width then break end  -- Stop if we've filled the bar

        if seg.spacer then
            -- Spacer becomes empty space
            local sw = seg.width or 0
            -- Clamp spacer to remaining width
            sw = math.min(sw, width - x)
            if sw > 0 then
                table.insert(result, {
                    text = string.rep(" ", sw),
                    style = bar_def.style,
                    x = x,
                })
                x = x + sw
            end
        else
            local text = seg.text or ""
            local text_width = seg._width or display_width(text)

            -- Truncate if would overflow
            if x + text_width > width then
                local available = width - x
                if available > 0 then
                    text = truncate_to_width(text, available)
                    text_width = display_width(text)
                else
                    text = ""
                    text_width = 0
                end
            end

            if text_width > 0 then
                table.insert(result, {
                    text = text,
                    style = seg.style or bar_def.style,
                    x = x,
                })
                x = x + text_width
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

    -- Debug: calculate total rendered width
    local total_width = 0
    for _, seg in ipairs(segments) do
        total_width = total_width + display_width(seg.text)
    end
    if total_width > ctx.w and tools and tools.log_warn then
        tools.log_warn(string.format(
            "bar %s overflow: %d > %d (ctx.w)",
            bar_def.name, total_width, ctx.w
        ))
    end

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
