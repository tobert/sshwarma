# MCP Manager Design Rationale

**Purpose:** Deep context for revision sessions. Read when you need to understand *why*.

---

## Why Non-Blocking Lua API?

The original `mcp_connect` used `block_in_place` + `block_on` to call async code from sync Lua context. This caused a deadlock during startup because:

1. `main()` is running in the tokio runtime
2. Lua runtime created in `main()` tries to call `mcp_connect`
3. `block_on` tries to create a nested runtime context
4. Deadlock

**Solution:** The Lua API becomes a "control plane" that declares intent. The actual connection logic runs in spawned tasks that don't block the caller.

---

## Why In-Memory State Only?

MCP connection state is ephemeral:
- Connections are reconfigured on each startup via `startup.lua`
- Persisting to DB would add complexity with no benefit
- `startup.lua` is the source of truth for desired connections
- Actual connection state (connected/reconnecting) is runtime-only

**Future:** If we add models.toml → Lua, the same pattern applies.

---

## Why Exponential Backoff with 3s Cap?

Per user request:
- Fast initial retry (100ms) for transient failures
- Exponential growth prevents CPU spin
- 3s cap keeps recovery time reasonable
- No max attempts — retry forever (user can `mcp_remove` if needed)

Alternative considered: Fixed delay. Rejected because it's either too fast (wastes resources) or too slow (poor UX).

---

## Why Event Broadcasting?

Multiple consumers need connection state changes:
1. **Logs/OTEL** — for observability and debugging
2. **HUD** — for user notifications
3. **Future** — webhooks, metrics, etc.

`tokio::sync::broadcast` allows multiple subscribers without the manager knowing who's listening.

---

## Why Not tokio::sync::watch?

`watch` only keeps the latest value, losing events. With `broadcast`:
- Each Reconnecting event is delivered
- Subscribers can track event history if needed
- Better for notifications ("connection failed, retrying in 200ms")

---

## Observability / OpenTelemetry Coverage

Connection lifecycle must be fully observable via tracing:

| Event | Span/Log | Level |
|-------|----------|-------|
| `add()` called | `info!` | INFO |
| Connection attempt starting | `span!` + `info!` | INFO |
| Connection succeeded | `info!` | INFO |
| Connection failed (will retry) | `warn!` | WARN |
| Retry delay starting | `debug!` | DEBUG |
| `remove()` called | `info!` | INFO |
| Connection removed | `info!` | INFO |
| Tool call starting | `span!` | DEBUG |
| Tool call completed | `debug!` | DEBUG |
| Tool call failed | `warn!` | WARN |

### Span Attributes

```rust
#[tracing::instrument(
    name = "mcp.connect",
    fields(
        mcp.server = %name,
        mcp.endpoint = %endpoint,
        mcp.attempt = attempt,
    )
)]
async fn connect_attempt(name: &str, endpoint: &str, attempt: u32) { ... }
```

### Integration with otlp-mcp

Since we're connecting to otlp-mcp, it's ironic if we don't emit proper spans. The connection lifecycle spans should be:

```
mcp.add
  └── mcp.connect_loop
       ├── mcp.connect_attempt (success or failure)
       ├── mcp.backoff_wait
       └── mcp.connect_attempt (retry)
```

---

## Cross-Cutting Concerns

### Thread Safety

`McpManager` must be `Send + Sync`:
- `desired` and `connections` use `RwLock`
- Background tasks communicate via channels
- No `Rc` or `RefCell`

### Graceful Shutdown

When `remove()` is called:
1. Remove from desired state
2. Cancel background task via `CancellationToken`
3. Gracefully close service if connected
4. Emit `Removed` event

When server shuts down:
- All background tasks cancelled via token hierarchy
- Services gracefully closed

### Error Handling

Connection errors should never panic. All errors:
1. Logged with context
2. Emitted as events
3. Trigger retry with backoff

---

## Rejected Alternatives

| Alternative | Why Rejected |
|-------------|--------------|
| Persist connections to DB | Adds complexity, startup.lua is source of truth |
| Max retry attempts | User wanted infinite retry with short cap |
| Sync connect in startup | Deadlocks, blocks startup |
| Single subscriber (no broadcast) | HUD and logs both need events |
| Watch channel | Loses intermediate events |
| tokio::select! in Lua | mlua doesn't support async well |

---

## Open Questions

| Question | Context | Status |
|----------|---------|--------|
| Replace models.toml with Lua? | User mentioned, out of scope | Deferred |
| Event persistence? | Log to DB for history? | Not needed yet |
| Health check interval? | Proactively detect connection loss? | Future work |
