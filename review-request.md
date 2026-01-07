# UI Rebuild Code Review Request

## Context

We've rebuilt the sshwarma UI system from a region-based layout to a weechat-inspired bar system. The goal was to move layout logic from Rust to pure Lua for faster iteration, implement vim-style modes, and enable ephemeral pages.

## Files to Review

### New Lua Modules (Primary Focus)

1. **`src/embedded/ui/layout.lua`** (~120 lines)
   - Pure Lua constraint solver replacing Rust layout code
   - Rect operations: sub(), shrink(), split_vertical(), split_horizontal()
   - Constraint parsing: absolute values and percentages
   - Bar-specific layout computation: compute_bars()

2. **`src/embedded/ui/bars.lua`** (~225 lines)
   - Bar definition system (name, position, priority, height, items, style)
   - Item registry for named render functions
   - Bar rendering with spacer distribution
   - Items return plain arrays of segment tables

3. **`src/embedded/ui/pages.lua`** (~135 lines)
   - Page stack management (chat is always index 1)
   - Navigation: nav_left(), nav_right(), goto()
   - Page CRUD: open(), close(), close_all()
   - Content and scroll position per page

4. **`src/embedded/ui/scroll.lua`** (~90 lines)
   - Per-page scroll state
   - Less-style following behavior (auto-scroll to bottom)
   - Navigation: up(), down(), to_top(), to_bottom()
   - State: offset, content_height, viewport_height, following

5. **`src/embedded/ui/mode.lua`** (~180 lines)
   - Vim-style mode system: "normal" and "insert"
   - Normal mode: hjkl navigation, page switching, scroll
   - Insert mode: readline editing, history, completion
   - Mode transitions: i//@/Enter/Escape/Ctrl+C
   - Global `on_input(bytes)` entry point

### Modified Files

6. **`src/embedded/ui/input.lua`**
   - Removed old `on_input()` and `handle_key()` (now in mode.lua)
   - Retains: parsing, UTF-8 handling, buffer operations, history

7. **`src/embedded/screen.lua`**
   - Complete rewrite using bars/pages/scroll/mode
   - Bar definitions: status (bottom), input (bottom)
   - Item definitions: room_name, participants, duration, mode_indicator, prompt, input_text
   - Content rendering: chat with scroll, help page

8. **`src/lua/mod.rs`**
   - Added module includes and registration for new UI modules
   - Added "send" as alias for "execute" action
   - Fixed module loading order (fun must load before bars)
   - Updated test for new bar system

## Architectural Changes

### Before
- Layout computed in Rust (~1000 lines in layout.rs)
- Regions system with z-ordering
- Input handling in input.lua with mode heuristics
- Scroll state managed in Rust (LuaScrollState)

### After
- Layout computed in pure Lua (~120 lines)
- Bars stack from edges inward (simpler model)
- Explicit vim-style modes (normal/insert)
- Scroll state in Lua, per-page
- Pages as ephemeral views within rooms

### Data Flow
```
on_input(bytes)           -- mode.lua entry point
  -> mode.handle_key()    -- routes by mode
    -> normal: scroll/pages/mode-switch
    -> insert: input buffer operations
  -> returns {type="send"|"redraw"|"quit"|...}

on_tick(dirty_tags, tick, ctx)  -- screen.lua entry point
  -> M.fetch_state()      -- get room/participants/history
  -> bars.compute_layout() -- pure Lua layout
  -> bars.render()        -- status/input bars
  -> M.render_chat()      -- content area
```

## Review Questions

1. **Layout module**: Is the constraint solving logic correct? Edge cases for tiny terminals?

2. **Bars module**: Is the spacer distribution correct? Any issues with segment rendering?

3. **Mode module**: Is the key routing complete? Any missing vim bindings?

4. **Pages module**: Is the page stack management sound? Memory leaks on page create/close?

5. **Scroll module**: Is the following behavior correct? Off-by-one errors in visible_range()?

6. **Integration**: Are there race conditions between mode state and input buffer?

7. **Testing**: The test suite passes but are there gaps in coverage for the new modules?

8. **Performance**: Any concerns with per-frame layout computation in Lua?

## Specific Concerns

- The `fun.iter()` pattern was replaced with plain arrays after iterator issues
- Module loading order is critical (fun must load before bars)
- The "send" vs "execute" action naming is a bit inconsistent
- Old Rust layout/scroll code is still present but unused

## How to Review

```bash
# View the new modules
bat src/embedded/ui/layout.lua src/embedded/ui/bars.lua \
    src/embedded/ui/pages.lua src/embedded/ui/scroll.lua \
    src/embedded/ui/mode.lua

# View the rewritten screen
bat src/embedded/screen.lua

# Check Rust changes
git diff HEAD~1 src/lua/mod.rs

# Run tests
cargo test
```

## Expected Output

Please provide:
1. Code correctness issues (bugs, edge cases)
2. API design feedback (naming, signatures)
3. Architecture concerns (coupling, complexity)
4. Performance observations
5. Testing gaps to address
6. Documentation improvements needed
