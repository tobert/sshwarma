# 02: Event Types and Broadcasting

**File:** `src/mcp/events.rs`
**Focus:** Event types and broadcast channel only
**Dependencies:** None
**Unblocks:** 03-manager, 05-integration

---

## Task

Create event types for MCP connection status changes and a broadcast mechanism for subscribers.

**Why this first?** Events are a pure type definition with no dependencies. The manager will emit these, and the HUD/logs will consume them.

**Deliverables:**
1. `src/mcp/events.rs` with event types
2. Broadcast channel wrapper
3. Module declared in `src/mcp/mod.rs`

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test mcp::events
```

## Out of Scope

- Connection logic — that's 03-manager
- HUD rendering — that's existing lua/hud code
- Lua API — that's 04-lua-api

Focus ONLY on event types and the broadcast mechanism.

---

## Design

Use `tokio::sync::broadcast` for multiple subscribers (HUD, logs, etc.). Events are cheap to clone and carry enough context for display.

---

## Types

```rust
/// MCP connection event for subscribers
#[derive(Debug, Clone)]
pub enum McpEvent {
    /// Connection attempt started
    Connecting {
        name: String,
        endpoint: String,
    },

    /// Successfully connected
    Connected {
        name: String,
        endpoint: String,
        tool_count: usize,
    },

    /// Connection failed, will retry
    Reconnecting {
        name: String,
        attempt: u32,
        delay_ms: u64,
        error: String,
    },

    /// Connection removed by user
    /// Note: We retry forever, so this only happens on mcp_remove()
    Removed {
        name: String,
    },

    /// Tools refreshed
    ToolsRefreshed {
        name: String,
        tool_count: usize,
    },
}

/// Broadcast sender for MCP events
#[derive(Clone)]
pub struct McpEventSender {
    tx: tokio::sync::broadcast::Sender<McpEvent>,
}

/// Receiver for MCP events
pub struct McpEventReceiver {
    rx: tokio::sync::broadcast::Receiver<McpEvent>,
}
```

---

## Methods to Implement

**McpEventSender:**
- `new(capacity: usize) -> Self` — create with buffer capacity (16 is fine)
- `send(&self, event: McpEvent)` — send event (ignore lagged receivers)
- `subscribe(&self) -> McpEventReceiver` — create new subscriber

**McpEventReceiver:**
- `recv(&mut self) -> Option<McpEvent>` — receive next event (async)

**McpEvent:**
- `name(&self) -> &str` — get connection name from any variant
- `is_error(&self) -> bool` — true for Reconnecting

---

## Implementation Notes

Broadcast channel pattern:
```rust
use tokio::sync::broadcast;

impl McpEventSender {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn send(&self, event: McpEvent) {
        // Ignore send errors (no receivers or lagged)
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> McpEventReceiver {
        McpEventReceiver { rx: self.tx.subscribe() }
    }
}
```

---

## Acceptance Criteria

- [ ] All event variants defined with required fields
- [ ] `name()` returns connection name for any event
- [ ] Multiple subscribers receive same events
- [ ] Lagged subscribers don't block sender
- [ ] Events are Clone + Send + Sync
