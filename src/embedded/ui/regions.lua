-- ui/regions.lua
--
-- Region/layer system for flexible screen layout.
-- Replaces hardcoded overlay with named, z-ordered regions.
--
-- Each region has:
--   - constraints (top, bottom, left, right, width, height)
--   - visibility state
--   - z-order (higher renders on top)
--
-- Regions are resolved to pixel Rects via sshwarma.layout().

local M = {}

-- Region definitions (constraints, not yet resolved)
local definitions = {}

-- Visibility state
local visibility = {}

-- Z-order (higher = on top)
local z_order = {}

-- Cached resolved rects (invalidated on terminal resize)
local resolved = {}
local cached_width, cached_height = 0, 0

--- Define a region
---@param name string Region name
---@param constraints table {top, bottom, left, right, width, height, z, visible}
function M.define(name, constraints)
    definitions[name] = constraints
    z_order[name] = constraints.z or 0
    visibility[name] = constraints.visible ~= false  -- default visible
    resolved = {}  -- invalidate cache
end

--- Show a region
---@param name string Region name
function M.show(name)
    if definitions[name] and not visibility[name] then
        visibility[name] = true
        resolved = {}  -- invalidate cache so resolve() includes this region
        if tools and tools.mark_dirty then
            tools.mark_dirty(name)
        end
    end
end

--- Hide a region
---@param name string Region name
function M.hide(name)
    if definitions[name] and visibility[name] then
        visibility[name] = false
        resolved = {}  -- invalidate cache so resolve() excludes this region
        if tools and tools.mark_dirty then
            tools.mark_dirty(name)
        end
    end
end

--- Toggle region visibility
---@param name string Region name
function M.toggle(name)
    if visibility[name] then
        M.hide(name)
    else
        M.show(name)
    end
end

--- Check if region is visible
---@param name string Region name
---@return boolean
function M.is_visible(name)
    return visibility[name] == true
end

--- Hide the topmost visible overlay (for Escape key)
--- Only considers regions with z > 0 (overlay regions)
---@return boolean True if an overlay was hidden
function M.hide_top()
    local top_name, top_z = nil, -1
    for name, z in pairs(z_order) do
        if z > 0 and visibility[name] and z > top_z then
            top_name = name
            top_z = z
        end
    end
    if top_name then
        M.hide(top_name)
        return true
    end
    return false
end

--- Resolve all regions for current terminal size
---@param width number Terminal width
---@param height number Terminal height
---@return table<string, LuaArea>
function M.resolve(width, height)
    -- Use cache if size unchanged
    if width == cached_width and height == cached_height and next(resolved) then
        return resolved
    end

    cached_width, cached_height = width, height
    resolved = {}

    -- Build layout definition table for sshwarma.layout()
    local layout_defs = {}
    for name, constraints in pairs(definitions) do
        if visibility[name] then
            table.insert(layout_defs, {
                name = name,
                top = constraints.top,
                bottom = constraints.bottom,
                left = constraints.left,
                right = constraints.right,
                width = constraints.width,
                height = constraints.height,
                fill = constraints.fill,
            })
        end
    end

    -- Resolve via existing Rust layout engine
    if sshwarma and sshwarma.layout then
        resolved = sshwarma.layout(layout_defs, width, height)
    end

    return resolved
end

--- Get a specific region's rect
---@param name string Region name
---@return LuaArea|nil
function M.get(name)
    return resolved[name]
end

--- Get all visible regions sorted by z-order (for rendering)
---@return table[] Array of {name, area, z}
function M.visible_ordered()
    local result = {}
    for name, area in pairs(resolved) do
        if visibility[name] then
            table.insert(result, {
                name = name,
                area = area,
                z = z_order[name] or 0,
            })
        end
    end
    table.sort(result, function(a, b) return a.z < b.z end)
    return result
end

--- Get list of all defined region names
---@return string[]
function M.list()
    local names = {}
    for name in pairs(definitions) do
        table.insert(names, name)
    end
    return names
end

--- Set a property on a region (for dynamic sizing)
---@param name string Region name
---@param key string Property name
---@param value any Property value
function M.set(name, key, value)
    if definitions[name] then
        definitions[name][key] = value
        resolved = {}  -- invalidate cache
        if tools and tools.mark_dirty then
            tools.mark_dirty(name)
        end
    end
end

--- Get a region's definition (constraints)
---@param name string Region name
---@return table|nil
function M.definition(name)
    return definitions[name]
end

--- Clear all regions (for reset/testing)
function M.clear()
    definitions = {}
    visibility = {}
    z_order = {}
    resolved = {}
    cached_width, cached_height = 0, 0
end

--- Invalidate cache (force re-resolve on next call)
function M.invalidate()
    resolved = {}
    cached_width, cached_height = 0, 0
end

return M
