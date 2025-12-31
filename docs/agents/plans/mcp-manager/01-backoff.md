# 01: Exponential Backoff Utility

**File:** `src/mcp/backoff.rs`
**Focus:** Retry timing logic only
**Dependencies:** None
**Unblocks:** 03-manager

---

## Task

Create a standalone exponential backoff utility for managing retry delays.

**Why this first?** The backoff logic is a pure utility with no dependencies. It can be developed and tested in isolation, then used by the manager.

**Deliverables:**
1. `src/mcp/backoff.rs` with `Backoff` struct
2. Unit tests for backoff behavior
3. Module declared in `src/mcp/mod.rs`

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test backoff
```

## Out of Scope

- Connection management — that's 03-manager
- Event handling — that's 02-events
- Async runtime — backoff is sync/pure

Focus ONLY on calculating retry delays.

---

## Design

Classic exponential backoff with jitter:
- Start at 100ms
- Double each retry
- Cap at 3 seconds (per user request)
- Optional jitter to prevent thundering herd

---

## Types

```rust
use std::time::Duration;

/// Exponential backoff calculator
#[derive(Debug, Clone)]
pub struct Backoff {
    /// Current attempt number (0-indexed)
    attempt: u32,
    /// Base delay (first retry)
    base: Duration,
    /// Maximum delay cap
    max: Duration,
}
```

---

## Methods to Implement

**Construction:**
- `new() -> Self` — default 100ms base, 3s cap
- `with_config(base: Duration, max: Duration) -> Self` — custom config

**Core:**
- `next_delay(&mut self) -> Duration` — calculate delay and increment attempt
- `reset(&mut self)` — reset to attempt 0 after success

**Queries:**
- `attempt(&self) -> u32` — current attempt number
- `current_delay(&self) -> Duration` — what delay would be without incrementing

---

## Implementation Notes

Delay calculation:
```rust
// delay = min(base * 2^attempt, max)
let delay_ms = self.base.as_millis() * 2u128.pow(self.attempt);
Duration::from_millis(delay_ms.min(self.max.as_millis()) as u64)
```

Optional: Add 10% jitter to prevent synchronized retries.

---

## Acceptance Criteria

- [ ] `new()` creates backoff with 100ms base, 3s max
- [ ] First delay is 100ms
- [ ] Delays double: 100ms, 200ms, 400ms, 800ms, 1600ms, 3000ms, 3000ms...
- [ ] Cap at 3s is respected
- [ ] `reset()` returns to initial state
- [ ] Thread-safe (just Clone, no shared state)
