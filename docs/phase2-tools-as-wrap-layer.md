# Phase 2: Tools as Wrap Layer

## Overview

Move tool definitions into the wrap() context composition system so they:
1. Count against token budget
2. Can be filtered per-room or per-focus
3. Enable room-scoped tool availability

## Current State

Tool injection is separate from wrap():

```rust
// src/ssh/handler.rs:spawn_model_response()
let wrap_result = lua.wrap(wrap_state, target_tokens);
let tool_defs = tool_server.get_tool_defs(None).await;  // separate!
let system_prompt = format!("{}{}", wrap_result.system_prompt, tool_guide);
```

Tools don't count against budget. All tools are always visible to the model.

## Target State

Tools become a wrap() layer:

```lua
-- src/embedded/wrap.lua
function default_wrap(target_tokens)
    return wrap(target_tokens)
        :system()
        :model_identity()
        :tools(room_tool_scope())  -- NEW
        :room()
        :participants()
        :history({...})
end
```

WrapResult returns filtered tools:

```rust
// src/lua/wrap.rs
pub struct WrapResult {
    pub system_prompt: String,
    pub context: String,
    pub tools: Vec<ToolDef>,  // NEW: filtered subset
}
```

## Implementation Steps

### 1. Add `tools.available_tools(scope)` Lua callback

**File**: `src/lua/tools.rs`

```rust
// tools.available_tools(scope) -> [{name, description, schema}]
// scope can be:
//   "all" - all available tools
//   "none" - no tools
//   {"holler_*", "sshwarma_look"} - glob patterns
//   nil - defaults to "all"
```

This requires access to the ToolServer's tool definitions. Options:
- Store tool defs in LuaToolState when MCP connects
- Add a callback to fetch from ToolServer on demand
- Cache tool defs in SharedState

Recommend: Cache in LuaToolState, refresh when MCP tools change.

### 2. Add `:tools(scope)` layer to WrapBuilder

**File**: `src/embedded/wrap.lua`

```lua
function WrapBuilder:tools(scope)
    local tool_defs = tools.available_tools(scope)
    if not tool_defs or #tool_defs == 0 then
        return self
    end

    local lines = {"## Your Functions", ""}
    for _, tool in ipairs(tool_defs) do
        -- Format tool for LLM consumption
        local name = tool.name:gsub("^sshwarma_", "")
        lines[#lines + 1] = string.format("- **%s**: %s", name, tool.description)
    end

    -- Store tool_defs for later extraction
    self._tools = tool_defs

    return self:add_layer("tools", 50, table.concat(lines, "\n"), true)
end
```

### 3. Extend WrapResult to include tools

**File**: `src/lua/wrap.rs`

```rust
pub struct WrapResult {
    pub system_prompt: String,
    pub context: String,
    pub tools: Vec<ToolDef>,  // extracted from Lua builder._tools
}

// In compose_context():
// After getting system_prompt and context, also extract builder._tools
let tools_table: Option<Table> = builder.get("_tools").ok();
let tools = if let Some(tbl) = tools_table {
    // Convert Lua table to Vec<ToolDef>
    parse_tool_defs(tbl)?
} else {
    vec![]  // No tools layer = empty
};
```

### 4. Update handler to use wrap.tools

**File**: `src/ssh/handler.rs`

```rust
// Instead of:
let tool_defs = tool_server.get_tool_defs(None).await;

// Use:
let tool_names: Vec<String> = wrap_result.tools.iter()
    .map(|t| t.name.clone())
    .collect();

// Filter ToolServer to only expose tools the model knows about
// (ToolServer still has all handlers, but we filter what LLM sees)
```

### 5. Add room tool scope configuration

**File**: `src/db/rooms.rs` or room KV

```rust
// Room can store tool_scope in KV:
// tool_scope = "all" | "none" | ["holler_*", "sshwarma_*"]

// Lua helper:
function room_tool_scope()
    local scope = tools.room_kv_get("tool_scope")
    return scope or "all"
end
```

### 6. Add `/tools` command for runtime control

**File**: `src/commands.rs`

```
/tools           - list available tools
/tools off       - disable tools for this room
/tools on        - enable all tools
/tools holler    - only holler tools
/tools internal  - only sshwarma internal tools
```

## Testing

1. **Unit test**: `tools.available_tools("all")` returns tool list
2. **Unit test**: `tools.available_tools({"holler_*"})` filters correctly
3. **Unit test**: wrap() with `:tools()` includes tools in budget
4. **E2E test**: Room with `tool_scope = "none"` hides tools from model
5. **E2E test**: `/tools off` then @mention shows no tools

## Migration

- Default `room_tool_scope()` returns `"all"` for backward compat
- Existing rooms work unchanged
- New rooms can opt into scoped tools

## Open Questions

1. **Tool schema in context?** Currently we just show name + description. Should we include JSON schema for complex tools? That's many more tokens.

2. **Internal vs MCP tools?** Should internal sshwarma tools (look, say, join) always be available, even when room scope is restricted?

3. **Per-model tool filtering?** Some models handle tools better than others. Should wrap() consider model capabilities?

## Files to Modify

- `src/lua/tools.rs` - add `available_tools(scope)` callback
- `src/lua/wrap.rs` - extend WrapResult with tools field
- `src/embedded/wrap.lua` - add `:tools(scope)` layer
- `src/ssh/handler.rs` - use wrap.tools instead of direct fetch
- `src/db/rooms.rs` - tool_scope KV storage (optional)
- `src/commands.rs` - `/tools` command (optional)

## Dependencies

- Phase 1 must be complete (wrap() wired into @mention) ✅
- ToolServer must expose tool definitions (already does via `get_tool_defs`)
