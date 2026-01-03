# Wrap Integration Plan

## Problem

@mentions in SSH bypass the wrap() system entirely. Models get a basic "You are X" prompt with no room context, no history, no conversation continuity.

## Solution

Wire wrap() into @mention handling with smart history filtering and tools as a context layer.

---

## Phase 1: Wire wrap() properly (current)

### 1.1 Rename compose_context → wrap at Rust API
- `LuaRuntime::compose_context()` → `LuaRuntime::wrap()`
- Clearer alignment with Lua side

### 1.2 Enhance tools.history() with filter params
**File**: `src/lua/tools.rs`

Current: `tools.history(limit)` - returns all messages

New: `tools.history(opts)` where opts can be:
- `limit` (number) - backward compat, simple case
- `{limit, agents, thread, since_marker}` (table) - filtered

```lua
tools.history(30)  -- simple, all messages
tools.history({limit = 30, agents = {"alice", "qwenb"}})  -- filtered
tools.history({limit = 30, since_marker = "checkpoint"})  -- marker-based
tools.history({limit = 30, thread = "row_123"})  -- sub-conversation
```

### 1.3 Set session context with model
**File**: `src/ssh/input.rs` in `handle_mention()`

Before spawning model response, set:
```rust
lua.tool_state().set_session_context(Some(SessionContext {
    username: player.username.clone(),
    model: Some(model.clone()),  // NEW: was None
    room_name: player.current_room.clone(),
}));
```

### 1.4 Call wrap() in spawn_model_response
**File**: `src/ssh/handler.rs`

```rust
// Get context window from model config (default 8000)
let target_tokens = model.context_window.unwrap_or(8000);

// Build context via wrap()
let wrap_result = lua_runtime.wrap(wrap_state, target_tokens)?;

// Use composed prompts
let system_prompt = format!("{}\n\n{}", wrap_result.system_prompt, tool_guide);
let message = format!("{}\n\n{}", wrap_result.context, user_message);
```

### 1.5 Update wrap.lua history layer
**File**: `src/embedded/wrap.lua`

```lua
local function format_history_layer(limit)
    local model = tools.current_model()
    local user = tools.current_user()

    -- Filter to user + current model only
    local agents = nil
    if model and user then
        agents = {user.name, model.name}
    end

    local messages = tools.history({limit = limit, agents = agents})
    -- ... format as before
end
```

### 1.6 Fail visibly on wrap() errors
**File**: `src/ssh/handler.rs`

If wrap() returns error, send error to user via notification, don't call LLM.

---

## Phase 2: Tools as wrap layer (next)

### 2.1 Add tools.available_tools(scope) Lua callback
Returns tool definitions from ToolServer, optionally filtered by scope.

### 2.2 Add :tools(scope) layer to WrapBuilder
```lua
:tools("all")           -- all tools
:tools("none")          -- no tools
:tools({"holler"})      -- only holler MCP tools
:tools({"sshwarma_*"})  -- glob pattern
```

### 2.3 Extend WrapResult to include tools
```rust
struct WrapResult {
    system_prompt: String,
    context: String,
    tools: Vec<ToolDef>,  // NEW
}
```

### 2.4 Handler uses wrap.tools for LLM
Pass filtered tool subset to LLM. ToolServer still has all handlers.

### 2.5 Room-scoped tool config
Room KV or config specifies default tool scope.

---

## Phase 3: Thread/focus (later)

### 3.1 /focus command
`/focus @qwenb` creates parent row. Subsequent messages nest under it.

### 3.2 /checkpoint command
Creates marker row. wrap() can filter "since checkpoint".

### 3.3 /thread command
Shows current focus context, allows clearing.

### 3.4 wrap() respects focus
History layer checks for active focus/thread.

---

## Phase 4: Affordances (later)

- `/context` - preview what model will see
- Token usage in HUD
- Summarization for old context
- RowSet power-user API in wrap.lua

---

## Non-goals (for now)

- Multi-turn conversation tracking (lean into wrap + fresh context)
- Explicit session management
- Conversation IDs
