# 03: Region/Layer System

**File:** `src/embedded/ui/regions.lua`, `src/ui/layout.rs` (minor changes)
**Focus:** Flexible region management replacing hardcoded overlay
**Dependencies:** None
**Unblocks:** 05-commands (needs overlay regions), 06-chat (needs chat region)

---

## Task

Replace the special-cased overlay with a general region system. Any part of screen can be a named region with z-ordering.

**Why this first?** Commands need to show output in overlays. Chat needs its region. Status bar needs its region. All UI elements become regions.

**Deliverables:**
1. `regions.define(name, constraints)` — Define a region
2. `regions.show(name)` / `regions.hide(name)` — Visibility control
3. `regions.get(name)` — Get resolved Rect for rendering
4. Z-ordering for layered rendering
5. Remove `LuaToolState.overlay` in favor of regions
6. Screen layout defined in Lua via regions

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- Complex nested layouts — keep it simple for now
- Animations — future enhancement
- Drag/resize — future enhancement

Focus ONLY on define, show, hide, get, and z-ordering.

---

## Current Layout System

We already have `src/ui/layout.rs` with:
- `RegionDef` — Constraint-based region definition
- `Layout::resolve()` — Resolves constraints to `Rect`
- `LuaArea` — Lua userdata for resolved regions

This task builds on that foundation.

---

## Lua Module

```lua
-- ui/regions.lua

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
---@param constraints table {top, bottom, left, right, width, height, z}
function M.define(name, constraints)
    definitions[name] = constraints
    z_order[name] = constraints.z or 0
    visibility[name] = constraints.visible ~= false  -- default visible
    resolved = {}  -- invalidate cache
end

--- Show a region
---@param name string Region name
function M.show(name)
    if definitions[name] then
        visibility[name] = true
        tools.mark_dirty(name)
    end
end

--- Hide a region
---@param name string Region name
function M.hide(name)
    if definitions[name] then
        visibility[name] = false
        tools.mark_dirty(name)
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
    resolved = sshwarma.layout(layout_defs, width, height)
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

--- Set a property on a region (for dynamic sizing)
---@param name string Region name
---@param key string Property name
---@param value any Property value
function M.set(name, key, value)
    if definitions[name] then
        definitions[name][key] = value
        resolved = {}  -- invalidate cache
        tools.mark_dirty(name)
    end
end

return M
```

---

## Default Layout Setup

```lua
-- In screen.lua initialization

local regions = require 'ui.regions'

-- Base layout (always visible, z=0)
regions.define('chat', {
    top = 0,
    bottom = -2,
    fill = true,
    z = 0,
})

regions.define('status', {
    bottom = -2,
    height = 1,
    z = 0,
})

regions.define('input', {
    bottom = 0,
    height = 1,
    z = 0,
})

-- Overlay for command output (hidden by default, z=10)
regions.define('overlay', {
    width = "80%",
    height = "80%",
    z = 10,
    visible = false,
})

-- Confirmation dialog (hidden, z=20)
regions.define('dialog', {
    width = 50,
    height = 7,
    z = 20,
    visible = false,
})
```

---

## Rendering with Regions

```lua
-- In on_tick

function on_tick(dirty_tags, tick, ctx)
    local regions = require 'ui.regions'

    -- Resolve regions for current terminal size
    local resolved = regions.resolve(ctx.w, ctx.h)

    -- Get visible regions in z-order
    local ordered = regions.visible_ordered()

    -- Render each region
    for _, r in ipairs(ordered) do
        local render_fn = renderers[r.name]
        if render_fn and r.area then
            -- Create clipped draw context for this region
            local region_ctx = ctx:clip(r.area.x, r.area.y, r.area.w, r.area.h)
            render_fn(region_ctx, r.area)
        end
    end
end

-- Register renderers
local renderers = {
    chat = require('ui.chat').render,
    status = require('ui.status').render,
    input = require('ui.input').render,
    overlay = require('ui.overlay').render,
    dialog = require('ui.dialog').render,
}
```

---

## Clipped Draw Context

The existing `LuaDrawContext` may need a clip method:

```rust
// In src/ui/render.rs - add clip method to LuaDrawContext

methods.add_method("clip", |_lua, this, (x, y, w, h): (u16, u16, u16, u16)| {
    // Create a new context with offset origin and clipping bounds
    Ok(LuaDrawContext {
        buffer: this.buffer.clone(),
        offset_x: this.offset_x + x,
        offset_y: this.offset_y + y,
        clip_w: w.min(this.clip_w.saturating_sub(x)),
        clip_h: h.min(this.clip_h.saturating_sub(y)),
    })
});
```

---

## Remove Old Overlay Code

Delete from `src/lua/tools.rs`:
- `OverlayState` struct
- `overlay` field in `LuaToolState`
- `show_overlay()`, `close_overlay()`, `has_overlay()`, `overlay_state()`
- `overlay_scroll_up()`, `overlay_scroll_down()`
- `tools.overlay` Lua function
- `tools.close_overlay` Lua function

These are replaced by the region system.

---

## Acceptance Criteria

- [ ] `regions.define('foo', {top=0, height=10})` works
- [ ] `regions.show('foo')` / `regions.hide('foo')` toggle visibility
- [ ] `regions.get('foo')` returns resolved LuaArea
- [ ] Centered regions work (`width="50%"` with no anchors)
- [ ] Z-ordering works (higher z renders on top)
- [ ] `regions.hide_top()` closes topmost overlay
- [ ] Terminal resize invalidates cache, re-resolves
- [ ] Dynamic sizing works (`regions.set('drawer', 'height', 10)`)
- [ ] Old overlay code deleted from Rust
- [ ] Chat, status, input render as regions
