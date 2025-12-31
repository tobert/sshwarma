# 04: Lua API

**File:** `src/lua/tools.rs` (modify existing)
**Focus:** Lua tool bindings only
**Dependencies:** 03-manager
**Unblocks:** 05-integration

---

## Task

Replace the blocking `mcp_connect`/`mcp_disconnect` with non-blocking `mcp_add`/`mcp_remove`/`mcp_status`/`mcp_list` that interact with `McpManager`.

**Why this?** The current blocking API deadlocks. The new API declares intent and returns immediately.

**Deliverables:**
1. Remove `mcp_connect` and `mcp_disconnect` functions
2. Add `mcp_add`, `mcp_remove`, `mcp_status`, `mcp_list`
3. All functions are non-blocking
4. Update `startup.lua` example

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test lua
```

## Out of Scope

- Manager implementation — that's 03-manager
- Event subscription in Lua — that's 05-integration
- HUD status display — existing HUD code handles that

Focus ONLY on the Lua ↔ Rust binding.

---

## API Design

```lua
-- Add connection (non-blocking, idempotent)
-- Returns immediately, connection happens in background
tools.mcp_add("holler", "http://localhost:8080/mcp")

-- Remove connection (graceful disconnect)
-- Returns true if was present, false if not found
local removed = tools.mcp_remove("holler")

-- Get status of one connection
local status = tools.mcp_status("holler")
-- Returns: {
--   name = "holler",
--   state = "connected",  -- or "connecting", "reconnecting", "failed"
--   tools = 47,
--   error = nil,          -- error message if failed/reconnecting
--   attempt = nil         -- retry attempt if reconnecting
-- }
-- Returns nil if connection not found

-- List all connections
local all = tools.mcp_list()
-- Returns: {
--   {name="holler", state="connected", tools=47},
--   {name="otlp-mcp", state="reconnecting", tools=0, error="...", attempt=2},
--   ...
-- }
```

---

## Implementation Pattern

The functions interact with `McpManager` via `SharedState`:

```rust
// Get manager from shared state (returns Option)
let shared = match state.shared_state() {
    Some(s) => s,
    None => {
        // No shared state during early startup - silently ignore
        return Ok(());
    }
};

// Manager methods are sync/non-blocking for control plane
// (internally they spawn async tasks via tokio::spawn)
```

**Important:** The manager's control plane methods (`add`, `remove`, `status`, `list`) must be callable from sync context. They use `tokio::spawn` internally to start async work, but return immediately without awaiting.

---

## Functions to Implement

**mcp_add:**
```rust
let mcp_add = {
    let state = state.clone();
    lua.create_function(move |_, (name, url): (String, String)| {
        if let Some(shared) = state.shared_state() {
            shared.mcp.add(&name, &url);  // Non-blocking, spawns task
        }
        Ok(())
    })?
};
```

**mcp_remove:**
```rust
let mcp_remove = {
    let state = state.clone();
    lua.create_function(move |_, name: String| {
        match state.shared_state() {
            Some(shared) => Ok(shared.mcp.remove(&name)),  // Non-blocking
            None => Ok(false),
        }
    })?
};
```

**mcp_status:**
```rust
let mcp_status = {
    let state = state.clone();
    lua.create_function(move |lua, name: String| {
        let Some(shared) = state.shared_state() else {
            return Ok(LuaValue::Nil);
        };
        match shared.mcp.status(&name) {
            Some(status) => {
                let table = lua.create_table()?;
                table.set("name", status.name)?;
                table.set("state", status.state)?;
                table.set("tools", status.tool_count)?;
                if let Some(err) = status.error {
                    table.set("error", err)?;
                }
                if let Some(attempt) = status.attempt {
                    table.set("attempt", attempt)?;
                }
                Ok(LuaValue::Table(table))
            }
            None => Ok(LuaValue::Nil)
        }
    })?
};
```

**mcp_list:**
```rust
let mcp_list = {
    let state = state.clone();
    lua.create_function(move |lua, ()| {
        let Some(shared) = state.shared_state() else {
            return Ok(lua.create_table()?);  // Empty table
        };
        let list = shared.mcp.list();  // Non-blocking

        let table = lua.create_table()?;
        for (i, status) in list.into_iter().enumerate() {
            let entry = lua.create_table()?;
            entry.set("name", status.name)?;
            entry.set("state", status.state)?;
            entry.set("tools", status.tool_count)?;
            if let Some(err) = status.error {
                entry.set("error", err)?;
            }
            if let Some(attempt) = status.attempt {
                entry.set("attempt", attempt)?;
            }
            table.set(i + 1, entry)?;
        }
        Ok(table)
    })?
};
```

---

## Update startup.lua.example

```lua
-- sshwarma startup script
-- ~/.config/sshwarma/startup.lua

function startup()
    print("Running sshwarma startup script...")

    -- Add MCP connections (non-blocking)
    -- Connections will retry automatically if servers are down
    tools.mcp_add("holler", "http://localhost:8080/mcp")
    tools.mcp_add("otlp-mcp", "http://localhost:4380/mcp")

    print("Startup complete!")
    print("MCP connections will establish in background")
end
```

---

## Acceptance Criteria

- [ ] `mcp_add` returns immediately (doesn't block on connection)
- [ ] `mcp_add` is idempotent (same name+url = no-op)
- [ ] `mcp_remove` cancels pending connection
- [ ] `mcp_status` returns current state including retry info
- [ ] `mcp_list` returns all connections with state
- [ ] Old `mcp_connect`/`mcp_disconnect` removed
- [ ] startup.lua works without blocking server startup
- [ ] Server starts even when MCP servers are down
