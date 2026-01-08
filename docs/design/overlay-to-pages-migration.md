# Overlay to Pages Migration

## Status: Blocked - Needs Implementation

## Problem

After the UI rebuild (bars/pages/mode system), commands like `/help` and `/rooms` no longer display their output. The commands work - they call `show_region("overlay", ...)` in Rust - but nothing renders that content because we removed the region-based rendering.

## Current State

### What Works
- Bar system (status bar, input bar) renders correctly
- Chat content renders in the content area
- Vim-style modes (normal/insert) work
- Page navigation exists but pages aren't populated by commands

### What's Broken
- `/help` - calls `show_region` but overlay not rendered
- `/rooms` - same issue
- Any command returning `mode = "overlay"` output

### Architecture Mismatch

**Old Flow (broken):**
```
Lua command handler
  -> returns {text, mode="overlay", title}
  -> Rust calls show_region("overlay", title, text)
  -> Rust stores in region_contents HashMap
  -> ??? (regions.lua deleted, nothing renders this)
```

**New Flow (needs implementation):**
```
Lua command handler
  -> returns {text, mode="overlay", title}
  -> Rust/Lua opens a page with the content
  -> pages.lua manages the page stack
  -> screen.lua renders pages in content area
```

## Files Involved

### Rust (to clean up)
- `src/lua/tools.rs` - `show_region`, `has_overlay`, `close_overlay`, `overlay_scroll_*`, `region_contents` field
- `src/ssh/handler.rs` - `has_overlay().await`, `close_overlay().await`, overlay scroll handlers

### Lua (to modify)
- `src/embedded/ui/pages.lua` - already has page stack, needs `open(name, content)` to accept command output
- `src/embedded/screen.lua` - needs to render page content (not just chat)
- `src/embedded/commands/init.lua` - command dispatch returns `{text, mode, title}`, needs to open pages

## Implementation Plan

### Step 1: Modify command dispatch to open pages

In `src/embedded/commands/init.lua` or wherever command results are processed:

```lua
local result = handler(args)
if result.mode == "overlay" then
    pages.open(result.title or "output", {
        content = result.text,
        scroll = 0,
    })
end
```

### Step 2: Update screen.lua to render page content

The `render_page` function exists but may need enhancement to display text content with scrolling.

### Step 3: Remove Rust overlay code

Delete from `src/lua/tools.rs`:
- `RegionContent` struct
- `region_contents` field from `LuaToolState`
- `show_region`, `hide_region`, `has_region_content`, `region_content` methods
- `show_overlay`, `close_overlay`, `has_overlay`, `overlay_scroll_*` methods
- `tools.overlay`, `tools.close_overlay` Lua bindings

Delete from `src/ssh/handler.rs`:
- `has_overlay().await` calls
- `close_overlay().await` calls
- Overlay scroll handling in PageUp/PageDown

### Step 4: Test commands

```bash
./target/debug/sshtest --cmd "/help" --wait-for "Navigation:"
./target/debug/sshtest --cmd "/rooms" --wait-for "Rooms:"
```

## Estimated Scope

- ~200 lines Rust deletion
- ~50 lines Lua modification
- Low risk - mostly removing code and wiring existing page system

## Related Commits

- `8dc8132` - refactor(ui): remove dead Rust layout/scroll code, add vim bindings
- `2c703e9` - refactor: delete Rust tab completion infrastructure
- `b97b5c6` - refactor: delete dead regions.lua module
