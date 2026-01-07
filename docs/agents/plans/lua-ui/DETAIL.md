# Lua UI Design Rationale

**Purpose:** Deep context for revision sessions. Read when you need to understand *why*.

---

## Why Lua Owns Everything?

### The Split Problem

Before this change, rendering was split:
- Rust formatted command output as strings
- Lua rendered those strings to screen
- Rust managed overlay state
- Lua called back to Rust for data

This created friction:
- Adding color to `/inv` output meant changing Rust string formatting
- Overlay was special-cased, couldn't have multiple or resize
- Hot-reload only affected rendering, not behavior
- Command logic duplicated between MCP tools and slash commands

### The Solution

Lua owns the entire SSH UI. Benefits:
- Rich rendering: colors, layouts, dynamic sizing all in Lua
- Hot-reload everything: change command behavior without rebuild
- Unified region model: overlays are just regions with z-index
- User customization: override any command or UI element
- Testable: mock `tools.*`, test UI logic in isolation

Rust becomes a backend:
- High-performance database operations
- MCP client/server
- LLM streaming
- SSH transport (byte shuffling)

---

## Why Raw Bytes for Input?

### Alternative: Parsed Key Events

Could have Rust parse escape sequences, send `{key="Up", ctrl=false}` to Lua.

**Why rejected:**
- Loses information (exact byte sequences vary by terminal)
- Can't implement terminal-specific behavior
- Rust needs terminal database (terminfo) or hardcoded sequences
- Two places to update when adding key support

### Raw Bytes Approach

Lua receives exact bytes from SSH channel. Lua parses escape sequences.

**Benefits:**
- Full control over interpretation
- Can handle any terminal
- Single source of truth for key handling
- Can log/debug exact input

**Tradeoff:**
- More Lua code
- Need escape sequence parser in Lua

Worth it for full control and debuggability.

---

## Why Custom Require System?

### Standard Lua Require

Lua's `require` searches `package.path` for `.lua` files. Works fine for filesystem modules.

### Our Needs

1. **Embedded modules**: Compiled into binary, no filesystem
2. **User overrides**: `~/.config/sshwarma/lua/` shadows embedded
3. **Future**: Database-stored scripts per room
4. **Standard libs**: Still want penlight, etc. from system

### Solution: Custom Searcher

Add a searcher to `package.searchers` that:
1. Checks embedded modules first
2. Then user config directory
3. Then falls through to standard path

```lua
-- Pseudo-implementation
table.insert(package.searchers, 2, function(modname)
    -- Check embedded
    local embedded = sshwarma.embedded_module(modname)
    if embedded then return load(embedded) end

    -- Check user config
    local user_path = sshwarma.config_path .. "/lua/" .. modname:gsub("%.", "/") .. ".lua"
    local f = io.open(user_path)
    if f then
        local content = f:read("*a")
        f:close()
        return load(content, "@" .. user_path)
    end

    -- Fall through to standard searchers
    return nil
end)
```

---

## Why Regions Instead of Overlays?

### Current Overlay

Single overlay state: on or off, covers chat area, fixed size.

```rust
// In LuaToolState
overlay: Option<OverlayState>  // Only one, or none
```

### Limitations

- Can't have two overlays (e.g., help + confirmation dialog)
- Can't have side panel or bottom drawer
- Can't resize dynamically
- Special-cased in input handling (ESC closes)

### Region Model

Regions are named areas that can be shown/hidden, positioned anywhere, layered.

```lua
regions.define('help', { width = "80%", height = "80%", z = 10 })
regions.define('confirm', { width = 40, height = 5, z = 20 })  -- On top of help
regions.define('sidebar', { right = 0, width = 40, z = 5 })
```

**Benefits:**
- Multiple simultaneous popups
- Flexible positioning
- Z-ordering for layering
- Same API for everything
- Dynamic sizing (collapsible panels)

**Key insight:** The existing `src/ui/layout.rs` already supports this! We just need to expose it to Lua properly and let Lua manage visibility.

---

## Why Streaming via Row Callbacks?

### Alternative: Polling

Lua could poll for new streaming chunks:
```lua
function background(tick)
    local chunks = tools.get_stream_chunks()
    for _, chunk in ipairs(chunks) do
        -- handle
    end
end
```

**Problems:**
- 500ms tick = visible latency
- Faster polling = CPU waste
- Race conditions between poll and render

### Alternative: Direct Callback per Chunk

```lua
function on_stream_chunk(text)
    append_to_chat(text)
    tools.mark_dirty('chat')
end
```

**Problems:**
- Chat needs to track streaming state
- Can't replay history (chunks not persisted)
- Different path than regular messages

### Row Callback Approach

Rust writes each chunk as a Row with `content_method = "message.model.chunk"`. Lua gets notified.

```lua
function on_row_added(buffer_id, row)
    tools.mark_dirty('chat')  -- Just trigger redraw
end
```

Chat rendering reads rows, handles streaming naturally:
```lua
for _, row in rows:iter() do
    if row.content_method == "message.model.chunk" then
        -- Append to current streaming message
    elseif row.content_method == "message.model" then
        -- Complete message
    end
end
```

**Benefits:**
- Streaming chunks are persisted (can replay)
- Same rendering path for history and live
- No polling, immediate notification
- LLM tool calls also become rows (unified model)

---

## Why Big Bang Rewrite?

### Alternative: Incremental Migration

Move one command at a time, maintain compatibility.

**Problems:**
- Transitional code (Rust calls Lua, Lua calls Rust)
- Two implementations to maintain
- Harder to reason about
- Takes longer overall

### Big Bang Approach

Write new Lua implementation. Delete old Rust. Ship.

**Why it works here:**
- Early code, few users
- Git history preserves everything
- Clean break = cleaner code
- Parallel agents = fast execution

**Risk mitigation:**
- Integration task (08) validates everything works together
- Cleanup task (09) only runs after integration passes
- Can always revert if needed

---

## Cross-Cutting Concerns

### Error Handling

Lua errors should:
1. Be caught by Rust runtime
2. Logged at ERROR level
3. Displayed to user (in overlay or status)
4. Not crash the session

```rust
match lua.call_function("on_input", bytes) {
    Ok(_) => {}
    Err(e) => {
        tracing::error!("Lua input error: {}", e);
        lua.tool_state().show_error(&e.to_string());
    }
}
```

### Hot Reloading

When Lua files change:
1. Detect via mtime check (existing mechanism)
2. Clear module cache for changed module
3. Re-require affected modules
4. Trigger full redraw

Question: Per-module reload or full reload? Start with full reload for simplicity.

### Testing

Lua commands can be tested by:
1. Mocking `tools.*` API
2. Calling command handlers directly
3. Asserting on return values / state changes

```lua
-- test_commands.lua
local mock_tools = { ... }
_G.tools = mock_tools

local commands = require 'commands'
local result = commands.dispatch('/inv')
assert(result.title == "Inventory")
```

---

## Rejected Alternatives

| Alternative | Why Rejected |
|-------------|--------------|
| Keep commands in Rust, just move rendering | Still split, still friction |
| Use Fennel instead of Lua | Extra complexity, less familiar |
| WebSocket transport instead of improving SSH | Different project |
| React-like virtual DOM | Overkill for terminal UI |
| Move everything to Rust (no Lua) | Loses hot-reload, customization |

---

## Open Questions

| Question | Context | Status |
|----------|---------|--------|
| Vendor penlight or require install? | Penlight is useful but large | Discuss |
| Module hot-reload granularity? | Full reload simpler, per-module faster | Start with full |
| Error display location? | Overlay intrusive, status may be missed | Try overlay first |
