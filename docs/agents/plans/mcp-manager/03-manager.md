# 03: McpManager Core

**File:** `src/mcp/manager.rs`
**Focus:** Connection lifecycle management
**Dependencies:** 01-backoff, 02-events
**Unblocks:** 04-lua-api, 05-integration

---

## Task

Implement `McpManager` that tracks desired vs actual connection state and manages connection lifecycle in background tasks.

**Why this second?** This is the core of the refactor. It depends on backoff and events, and enables the Lua API and integration.

**Deliverables:**
1. `src/mcp/manager.rs` with `McpManager` struct
2. Background task for connection attempts
3. Integration tests for connect/disconnect flow
4. Replace `McpClients` references with `McpManager`

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test mcp::manager
```

## Out of Scope

- Lua bindings — that's 04-lua-api
- HUD integration — that's 05-integration
- Tool call routing — preserve existing `call_tool` logic

Focus ONLY on connection lifecycle.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                       McpManager                             │
│                                                              │
│  ┌─────────────────┐     ┌──────────────────────────────┐  │
│  │  Desired State  │     │     Connection State          │  │
│  │  (what we want) │     │     (what we have)            │  │
│  │                 │     │                               │  │
│  │  holler → url   │     │  holler → Connected(service)  │  │
│  │  otlp → url     │     │  otlp → Reconnecting(2)       │  │
│  └─────────────────┘     └──────────────────────────────┘  │
│           │                          ▲                      │
│           │                          │                      │
│           ▼                          │                      │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              Background Tasks                         │   │
│  │  - Spawn on add(), cancel on remove()                │   │
│  │  - Connect with backoff                              │   │
│  │  - Emit events on state change                       │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

---

## Types

```rust
use crate::mcp::{Backoff, McpEvent, McpEventSender};
use rmcp::{RoleClient, model::Tool, service::RunningService};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

type McpService = RunningService<RoleClient, ()>;

/// Connection state machine
#[derive(Debug, Clone)]
pub enum ConnectionState {
    /// Initial state, connection task spawned
    Connecting,
    /// Successfully connected
    Connected {
        tool_count: usize,
    },
    /// Failed, retrying with backoff (infinite retry)
    Reconnecting {
        attempt: u32,
        last_error: String,
    },
}

/// Internal connection tracking
struct ManagedConnection {
    /// Endpoint URL
    endpoint: String,
    /// Current state
    state: ConnectionState,
    /// Active service (when connected)
    service: Option<Arc<McpService>>,
    /// Cached tools
    tools: Vec<Tool>,
    /// Stats
    call_count: u64,
    last_tool: Option<String>,
    /// Cancellation token for background task
    cancel: CancellationToken,
}

/// MCP connection manager
pub struct McpManager {
    /// Desired connections (name → endpoint)
    desired: RwLock<HashMap<String, String>>,
    /// Actual connections (name → state)
    connections: RwLock<HashMap<String, ManagedConnection>>,
    /// Event broadcaster
    events: McpEventSender,
}
```

---

## Methods to Implement

**Construction:**
- `new() -> Self` — create manager with event sender
- `with_events(sender: McpEventSender) -> Self` — use external sender

**Control Plane (non-blocking, called by Lua):**
- `add(&self, name: &str, endpoint: &str)` — add to desired, spawn task if new. Idempotent: same name+url is no-op; different url triggers reconnect.
- `remove(&self, name: &str) -> bool` — remove from desired, cancel task, emit Removed event
- `status(&self, name: &str) -> Option<ConnectionStatus>` — get status
- `list(&self) -> Vec<ConnectionStatus>` — list all connections

**Data Plane (called by existing tool logic):**
- `call_tool(&self, name: &str, args: Value) -> Result<ToolResult>` — route to connected MCP
- `list_tools(&self) -> Vec<ToolInfo>` — all tools from all connected MCPs
- `rig_tools(&self) -> Option<RigToolContext>` — for rig agent integration
- `refresh_tools(&self, name: &str) -> Result<()>` — refresh tool list from connected MCP

**Events:**
- `subscribe(&self) -> McpEventReceiver` — get event stream

**Internal:**
- `spawn_connection_task(&self, name: String, endpoint: String)` — start background connect
- `connection_loop(name, endpoint, state, events, cancel)` — background task

---

## rmcp Patterns

Connection:
```rust
use rmcp::{service::ServiceExt, transport::StreamableHttpClientTransport};

let transport = StreamableHttpClientTransport::from_uri(endpoint);
let service = ().serve(transport).await?;
let tools = service.list_tools(Default::default()).await?.tools;
```

Graceful shutdown:
```rust
service.cancellation_token().cancel();
```

---

## Background Task Pattern

```rust
async fn connection_loop(
    name: String,
    endpoint: String,
    connections: Arc<RwLock<HashMap<String, ManagedConnection>>>,
    events: McpEventSender,
    cancel: CancellationToken,
) {
    let mut backoff = Backoff::new();

    loop {
        // Check if cancelled (user called mcp_remove)
        if cancel.is_cancelled() {
            events.send(McpEvent::Removed { name: name.clone() });
            break;
        }

        events.send(McpEvent::Connecting { name: name.clone(), endpoint: endpoint.clone() });

        match connect(&endpoint).await {
            Ok((service, tools)) => {
                events.send(McpEvent::Connected {
                    name: name.clone(),
                    endpoint: endpoint.clone(),
                    tool_count: tools.len(),
                });
                backoff.reset();

                // Store connection
                let service_cancel = service.cancellation_token().clone();
                update_state(&connections, &name, ConnectionState::Connected {
                    tool_count: tools.len()
                }, Some(service), tools).await;

                // Wait for cancellation OR service death, then reconnect
                tokio::select! {
                    _ = cancel.cancelled() => {
                        events.send(McpEvent::Removed { name: name.clone() });
                        break;
                    }
                    _ = service_cancel.cancelled() => {
                        // Service died, loop will reconnect
                        continue;
                    }
                }
            }
            Err(e) => {
                let delay = backoff.next_delay();
                events.send(McpEvent::Reconnecting {
                    name: name.clone(),
                    attempt: backoff.attempt(),
                    delay_ms: delay.as_millis() as u64,
                    error: e.to_string(),
                });

                update_state(&connections, &name, ConnectionState::Reconnecting {
                    attempt: backoff.attempt(),
                    last_error: e.to_string(),
                }, None, vec![]).await;

                // Wait with cancellation check
                tokio::select! {
                    _ = tokio::time::sleep(delay) => continue,
                    _ = cancel.cancelled() => {
                        events.send(McpEvent::Removed { name: name.clone() });
                        break;
                    }
                }
            }
        }
    }
}
```

---

## Status Struct

For Lua API responses:

```rust
/// Connection status for API responses
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    pub name: String,
    pub endpoint: String,
    pub state: String,  // "connecting", "connected", "reconnecting", "failed"
    pub tool_count: usize,
    pub error: Option<String>,
    pub attempt: Option<u32>,
}
```

---

## Tracing / OpenTelemetry

All connection lifecycle operations must be instrumented:

```rust
use tracing::{info, warn, error, debug, instrument, Span};

#[instrument(skip(self), fields(mcp.server = %name, mcp.endpoint = %endpoint))]
pub fn add(&self, name: &str, endpoint: &str) {
    info!("adding MCP connection");
    // ...
}

#[instrument(skip(self), fields(mcp.server = %name))]
pub fn remove(&self, name: &str) -> bool {
    info!("removing MCP connection");
    // ...
}
```

Background task instrumentation:
```rust
#[instrument(
    name = "mcp.connect_loop",
    skip_all,
    fields(mcp.server = %name, mcp.endpoint = %endpoint)
)]
async fn connection_loop(...) {
    loop {
        let attempt_span = tracing::info_span!(
            "mcp.connect_attempt",
            mcp.attempt = backoff.attempt()
        );
        let _guard = attempt_span.enter();

        match connect(&endpoint).await {
            Ok(_) => {
                info!("connection established");
                // ...
            }
            Err(e) => {
                warn!(error = %e, "connection failed, will retry");
                // ...
            }
        }
    }
}
```

Required log levels:
| Event | Level |
|-------|-------|
| add() called | INFO |
| remove() called | INFO |
| Connection removed | INFO |
| Connection attempt starting | INFO |
| Connection succeeded | INFO |
| Connection failed (will retry) | WARN |
| Backoff delay starting | DEBUG |
| Tool call | DEBUG |

---

## Acceptance Criteria

- [ ] `add()` is non-blocking, spawns background task
- [ ] `remove()` cancels background task gracefully
- [ ] Connection retries with exponential backoff
- [ ] Events emitted on all state transitions
- [ ] `call_tool()` works when connected
- [ ] `call_tool()` returns error when not connected
- [ ] Existing `list_tools()` and `rig_tools()` preserved
- [ ] Server starts even if MCP servers are down
- [ ] All lifecycle operations have tracing spans
- [ ] Log levels follow the table above
