# 05: Integration

**File:** Multiple files
**Focus:** Wire everything together
**Dependencies:** 03-manager, 04-lua-api
**Unblocks:** None (final task)

---

## Task

Integrate `McpManager` into the application, connecting events to HUD notifications and ensuring all existing functionality works.

**Why last?** This brings together all the pieces and validates the full flow.

**Deliverables:**
1. Update `SharedState` to use `McpManager` instead of `McpClients`
2. Wire event subscriber to HUD notifications
3. Update `/mcp` command to use new status info
4. Verify startup script flow works
5. End-to-end testing

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
# Manual: start server, verify MCP connections retry
```

## Out of Scope

- Changing manager internals — that's done
- Changing Lua API — that's done
- New HUD features beyond notifications

Focus ONLY on integration and verification.

---

## Files to Modify

| File | Change |
|------|--------|
| `src/state.rs` | `mcp: Arc<McpClients>` → `mcp: Arc<McpManager>` |
| `src/main.rs` | Initialize `McpManager`, maybe subscribe for logging |
| `src/commands.rs` | Update `/mcp` command to show new status format |
| `src/display/hud/state.rs` | Optionally add event receiver for notifications |
| `src/lua/tools.rs` | Existing MCP tool calls route through manager |

---

## state.rs Changes

```rust
// Before
use crate::mcp::McpClients;
pub struct SharedState {
    pub mcp: Arc<McpClients>,
    // ...
}

// After
use crate::mcp::McpManager;
pub struct SharedState {
    pub mcp: Arc<McpManager>,
    // ...
}
```

---

## main.rs Changes

```rust
// Before
let mcp = Arc::new(McpClients::new());

// After
let mcp = Arc::new(McpManager::new());

// Optional: log events for debugging
{
    let mut rx = mcp.subscribe();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match &event {
                McpEvent::Connected { name, tool_count, .. } => {
                    info!("MCP '{}' connected ({} tools)", name, tool_count);
                }
                McpEvent::Reconnecting { name, attempt, delay_ms, error } => {
                    warn!("MCP '{}' reconnecting (attempt {}, {}ms): {}",
                        name, attempt, delay_ms, error);
                }
                McpEvent::Removed { name } => {
                    info!("MCP '{}' removed", name);
                }
                _ => debug!("MCP event: {:?}", event),
            }
        }
    });
}
```

---

## /mcp Command Update

Current format:
```
MCP Connections:
  holler (http://localhost:8080/mcp)
    47 tools, 3 calls, last: sample
```

New format (with state):
```
MCP Connections:
  holler: connected (47 tools)
    http://localhost:8080/mcp
    3 calls, last: sample
  otlp-mcp: reconnecting (attempt 2)
    http://localhost:4380/mcp
    Error: connection refused
```

---

## HUD Notification Integration (Optional)

If we want MCP events in HUD notifications:

```rust
// In HudState or similar
pub struct HudState {
    // ...
    mcp_events: Option<McpEventReceiver>,
}

impl HudState {
    pub fn poll_mcp_events(&mut self) {
        if let Some(rx) = &mut self.mcp_events {
            while let Ok(event) = rx.try_recv() {
                self.add_notification(event.into());
            }
        }
    }
}
```

This is optional — the existing HUD already polls connection state. Events are mainly useful for transient notifications.

---

## Verification Checklist

**Startup Flow:**
1. [ ] Server starts with no MCP servers running
2. [ ] startup.lua executes without blocking
3. [ ] MCP connections retry in background
4. [ ] When MCP server starts, connection succeeds

**Runtime:**
1. [ ] `/mcp` shows connection status
2. [ ] `/tools` lists tools from connected MCPs
3. [ ] Tool calls work when connected
4. [ ] Tool calls fail gracefully when disconnected

**HUD:**
1. [ ] MCP section shows connection state
2. [ ] Reconnecting state visible
3. [ ] Connected state shows tool count

---

## Testing Script

```bash
# Terminal 1: Start server with no MCP
cargo run --release

# Server should start without blocking
# Logs should show reconnection attempts

# Terminal 2: Start holler
cd ~/src/hootenanny && cargo run --bin holler

# Watch Terminal 1: should connect automatically

# Terminal 3: SSH in
ssh -p 2222 localhost
/mcp
# Should show holler: connected
```

---

## Acceptance Criteria

- [ ] Server starts without MCP servers running
- [ ] startup.lua with mcp_add doesn't block
- [ ] Connections retry automatically with backoff
- [ ] `/mcp` shows accurate status
- [ ] Tool calls work when connected
- [ ] All existing tests pass
- [ ] No regressions in MCP functionality
