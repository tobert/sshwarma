# 04: Expanded Tools API

**File:** `src/lua/tools.rs`
**Focus:** Add Lua bindings for all command operations
**Dependencies:** None
**Unblocks:** 05-commands (needs tools.* for all operations)

---

## Task

Expand the `tools.*` Lua API to support all operations that commands need. Commands will call these tools instead of having business logic in Rust.

**Why this first?** Lua commands need to call Rust operations. This defines the interface.

**Deliverables:**
1. Room operations: `tools.rooms()`, `tools.join()`, `tools.create()`, `tools.leave()`
2. Inventory operations: `tools.inventory()`, `tools.equip()`, `tools.unequip()`
3. Journal operations: `tools.journal()`, `tools.journal_add()`
4. Navigation: `tools.go()`, `tools.exits()`, `tools.dig()`
5. MCP: `tools.mcp_tools()`, `tools.mcp_call()`
6. All operations return structured Lua tables (not formatted strings)

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- Command formatting/display — that's 05-commands
- Streaming — that's 07-streaming
- Rendering — that's 06-chat

Focus ONLY on adding the Rust→Lua bindings for operations.

---

## API Design Principles

1. **Return structured data**, not strings
2. **Simple success/error pattern**: `{success=true, data=...}` or `{success=false, error="..."}`
3. **Use session context** for room, username when needed
4. **Consistent naming**: verb_noun pattern

---

## Room Operations

```lua
-- tools.rooms() -> {rooms = [{name, description, user_count, model_count}, ...]}
-- List all rooms
local result = tools.rooms()
for _, room in ipairs(result.rooms) do
    print(room.name, room.user_count)
end

-- tools.join(room_name) -> {success, room, error}
-- Join a room
local result = tools.join("workshop")
if result.success then
    print("Joined " .. result.room.name)
else
    print("Error: " .. result.error)
end

-- tools.create(room_name, description?) -> {success, room, error}
-- Create a new room
local result = tools.create("my-room", "A description")

-- tools.leave() -> {success, error}
-- Leave current room, return to lobby
local result = tools.leave()

-- tools.look() -> {room, users, models, exits, vibe, assets}
-- Get current room info
local info = tools.look()
print(info.room.name, info.vibe)

-- tools.who() -> {users = [{name, status}, ...]}
-- Who's in current room
local result = tools.who()
```

---

## Inventory Operations

```lua
-- tools.inventory() -> {equipped = [...], available = [...]}
-- Get inventory for current room
local inv = tools.inventory()
for _, item in ipairs(inv.equipped) do
    print(item.qualified_name, item.description)
end

-- tools.equip(qualified_name) -> {success, added, removed, equipped, error}
-- Equip a tool
local result = tools.equip("holler:sample")
if result.success then
    for _, name in ipairs(result.added) do
        print("Equipped: " .. name)
    end
end

-- tools.unequip(qualified_name) -> {success, removed, equipped, error}
-- Unequip a tool
local result = tools.unequip("holler:sample")

-- tools.portal(server, direction) -> {success, error}
-- Create portal/exit to MCP server's rooms
local result = tools.portal("holler", "music")
```

---

## Journal Operations

```lua
-- tools.journal(kind?, limit?) -> {entries = [{kind, content, author, created_at}, ...]}
-- Read journal entries
local result = tools.journal("note", 10)

-- tools.journal_add(kind, content) -> {success, entry, error}
-- Add a journal entry
local result = tools.journal_add("note", "This is my note")
-- kind: "note", "decision", "idea", "milestone", "question"
```

---

## Navigation

```lua
-- tools.exits() -> {exits = [{direction, target_room}, ...]}
-- List exits from current room
local result = tools.exits()
for _, exit in ipairs(result.exits) do
    print(exit.direction .. " -> " .. exit.target_room)
end

-- tools.go(direction) -> {success, room, error}
-- Navigate through an exit
local result = tools.go("north")

-- tools.dig(direction, target_room, bidirectional?) -> {success, error}
-- Create an exit
local result = tools.dig("studio", "music-studio", true)

-- tools.fork(new_name) -> {success, room, error}
-- Fork current room
local result = tools.fork("my-fork")
```

---

## Context Operations

```lua
-- tools.vibe() -> string
-- Get current room's vibe
local vibe = tools.vibe()

-- tools.set_vibe(text) -> {success, error}
-- Set current room's vibe
local result = tools.set_vibe("Collaborative coding session")

-- tools.inspire(text?) -> {inspirations = [...]} or {success, added, error}
-- Get or add inspirations
local inspirations = tools.inspire()  -- get
local result = tools.inspire("New idea!")  -- add
```

---

## Asset Operations

```lua
-- tools.bring(artifact_id, role) -> {success, error}
-- Bind artifact to room
local result = tools.bring("abc123", "reference")

-- tools.drop(role) -> {success, error}
-- Unbind artifact from room
local result = tools.drop("reference")

-- tools.examine(role) -> {asset, error}
-- Inspect bound asset
local result = tools.examine("reference")
if result.asset then
    print(result.asset.artifact_id, result.asset.notes)
end
```

---

## MCP Operations

```lua
-- tools.mcp_servers() -> {servers = [{name, connected, tool_count}, ...]}
-- List MCP servers
local servers = tools.mcp_servers()

-- tools.mcp_tools(server?) -> {tools = [{name, description, server}, ...]}
-- List available tools (optionally filtered by server)
local all_tools = tools.mcp_tools()
local holler_tools = tools.mcp_tools("holler")

-- tools.mcp_call(server, tool, args) -> {success, result, error}
-- Call an MCP tool
local result = tools.mcp_call("holler", "sample", {prompt = "upbeat jazz"})
```

---

## Prompt Stack Operations

```lua
-- tools.prompts(target?) -> {prompts = [{name, content, priority}, ...]}
-- List prompts (for room or specific target like "system", "user")
local result = tools.prompts("system")

-- tools.prompt_set(name, content) -> {success, error}
-- Set a named prompt
local result = tools.prompt_set("my-prompt", "You are a helpful assistant")

-- tools.prompt_push(target, name) -> {success, error}
-- Push prompt onto target stack
local result = tools.prompt_push("system", "my-prompt")

-- tools.prompt_pop(target) -> {success, removed, error}
-- Pop from target stack
local result = tools.prompt_pop("system")

-- tools.prompt_delete(name) -> {success, error}
-- Delete a prompt
local result = tools.prompt_delete("my-prompt")
```

---

## Rules Operations

```lua
-- tools.rules() -> {rules = [{id, name, trigger, script, enabled}, ...]}
-- List room rules
local result = tools.rules()

-- tools.rules_add(trigger_kind, script_name, opts) -> {success, rule_id, error}
-- Add a rule
local result = tools.rules_add("tick", "my-script", {tick_divisor = 4})

-- tools.rules_del(rule_id) -> {success, error}
-- Delete a rule
local result = tools.rules_del("abc123")

-- tools.rules_enable(rule_id, enabled) -> {success, error}
-- Enable/disable a rule
local result = tools.rules_enable("abc123", false)

-- tools.scripts() -> {scripts = [{id, name, kind, description}, ...]}
-- List available Lua scripts
local result = tools.scripts()
```

---

## Rust Implementation Pattern

```rust
// In src/lua/tools.rs

// tools.rooms()
let rooms_fn = {
    let state = state.clone();
    lua.create_function(move |lua, ()| {
        let session = get_session_context(&state)?;
        let rooms = state.db.list_rooms()
            .map_err(|e| mlua::Error::runtime(e.to_string()))?;

        let result = lua.create_table()?;
        let rooms_table = lua.create_table()?;

        for (i, room) in rooms.iter().enumerate() {
            let room_table = lua.create_table()?;
            room_table.set("name", room.name.clone())?;
            room_table.set("description", room.description.clone())?;
            room_table.set("user_count", room.user_count)?;
            room_table.set("model_count", room.model_count)?;
            rooms_table.set(i + 1, room_table)?;
        }

        result.set("rooms", rooms_table)?;
        Ok(result)
    })?
};
tools.set("rooms", rooms_fn)?;

// tools.join(room_name)
let join_fn = {
    let state = state.clone();
    lua.create_function(move |lua, room_name: String| {
        let result = lua.create_table()?;

        // Get session context to update
        let session = get_session_context_mut(&state)?;

        match ops::join_room(&state, &session.username, &room_name) {
            Ok(room) => {
                session.room_name = Some(room_name.clone());
                result.set("success", true)?;

                let room_table = lua.create_table()?;
                room_table.set("name", room.name)?;
                room_table.set("description", room.description)?;
                result.set("room", room_table)?;
            }
            Err(e) => {
                result.set("success", false)?;
                result.set("error", e.to_string())?;
            }
        }

        Ok(result)
    })?
};
tools.set("join", join_fn)?;
```

---

## Acceptance Criteria

- [ ] All room operations work: rooms, join, create, leave, look, who
- [ ] All inventory operations work: inventory, equip, unequip, portal
- [ ] All journal operations work: journal, journal_add
- [ ] All navigation operations work: exits, go, dig, fork
- [ ] All context operations work: vibe, set_vibe, inspire
- [ ] All asset operations work: bring, drop, examine
- [ ] All MCP operations work: mcp_servers, mcp_tools, mcp_call
- [ ] All prompt operations work: prompts, prompt_set, prompt_push, prompt_pop
- [ ] All rules operations work: rules, rules_add, rules_del, rules_enable
- [ ] All operations return structured tables (not strings)
- [ ] Error cases return `{success=false, error="message"}`
- [ ] Session context properly tracked (current room, username)
