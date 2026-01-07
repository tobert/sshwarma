# 09: Cleanup - Delete Deprecated Code

**File:** Multiple files
**Focus:** Remove old Rust code that's been replaced by Lua
**Dependencies:** 08-integration (must work first!)
**Unblocks:** Complete

---

## Task

Delete deprecated Rust code that has been replaced by the Lua implementation. This is the final step.

**Why this task?** Dead code is confusing and increases maintenance burden. Clean break.

**Precondition:** 08-integration must pass all tests. Only proceed if everything works.

**Deliverables:**
1. Delete `src/commands.rs`
2. Simplify `src/ssh/handler.rs`
3. Remove overlay code from `src/lua/tools.rs`
4. Update module exports
5. Verify build and tests pass

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
# Full manual test after cleanup
```

## Out of Scope

- Any new functionality
- Refactoring beyond deletion
- Documentation updates (separate task)

Focus ONLY on removing deprecated code.

---

## Files to Delete

### `src/commands.rs` (2061 lines)

Delete the entire file. All command logic is now in Lua.

```bash
rm src/commands.rs
```

Update `src/lib.rs` or `src/main.rs` to remove the module:
```rust
// Remove this line:
// mod commands;
```

---

## Files to Simplify

### `src/ssh/handler.rs`

Remove command-related code. Keep only:
- SSH connection handling
- Byte forwarding to Lua
- Session management

**Delete:**
- `handle_input()` method (input goes to Lua)
- Command dispatch logic
- Any direct command calls

**Keep:**
- `data()` handler (forwards to Lua)
- `shell_request()`, `pty_request()` etc.
- Session setup

### `src/ssh/input.rs`

This file may be entirely replaceable. If all input handling is in Lua:

```bash
rm src/ssh/input.rs
```

Or keep minimal structure if still needed for SSH protocol level.

### `src/lua/tools.rs`

**Delete overlay-related code:**
- `OverlayState` struct
- `overlay` field in `LuaToolState`
- `show_overlay()` method
- `close_overlay()` method
- `has_overlay()` method
- `overlay_state()` method
- `overlay_scroll_up()` method
- `overlay_scroll_down()` method
- `tools.overlay` Lua function
- `tools.close_overlay` Lua function

**Keep:**
- All new tools.* functions added in 04-tools-api
- Core tool state management
- MCP bridge
- Notification system (if still used)

---

## Module Updates

### `src/lib.rs`

```rust
// Remove:
// pub mod commands;

// Keep:
pub mod db;
pub mod lua;
pub mod mcp;
pub mod ops;
pub mod ssh;
pub mod ui;
// ... etc
```

### `src/ssh/mod.rs`

```rust
// Update exports if input.rs deleted
pub mod handler;
pub mod screen;
pub mod session;
pub mod streaming;
// Remove: pub mod input;
```

---

## Verification Steps

After each deletion, run:

```bash
cargo check
```

If it fails, either:
1. Something still depends on deleted code → find and update
2. Import paths need updating → fix imports

After all deletions:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

---

## Code to Keep (Do Not Delete)

| File | Reason |
|------|--------|
| `src/ui/layout.rs` | Used by Lua regions |
| `src/ui/render.rs` | RenderBuffer used by Lua |
| `src/ui/scroll.rs` | ScrollState used by Lua |
| `src/lua/mod.rs` | Core runtime |
| `src/lua/tools.rs` | Expanded API (minus overlay) |
| `src/lua/data.rs` | Row/Buffer bindings |
| `src/lua/wrap.rs` | Context composition |
| `src/ops.rs` | Business logic (called by tools.*) |
| `src/db/*` | All database code |
| `src/mcp/*` | All MCP code |

---

## Potential Issues

### Orphaned Imports

After deleting commands.rs, other files may have:
```rust
use crate::commands::*;
```

Find and remove these.

### Test Dependencies

Some tests might use deleted code. Update or remove:
```rust
#[test]
fn test_cmd_join() {
    // If this tests Rust command, delete it
    // Lua commands tested via integration
}
```

### Feature Flags

If commands.rs had feature-gated code, ensure features still work:
```bash
cargo check --all-features
```

---

## Line Count Verification

Before:
```bash
wc -l src/commands.rs src/ssh/input.rs
# ~2259 lines
```

After:
```bash
# These files should not exist
ls src/commands.rs src/ssh/input.rs
# Should fail
```

Verify total codebase reduction:
```bash
git diff --stat HEAD~1
# Should show significant line reduction
```

---

## Acceptance Criteria

- [ ] `src/commands.rs` deleted
- [ ] `src/ssh/input.rs` deleted (if fully replaced)
- [ ] Overlay code removed from `src/lua/tools.rs`
- [ ] Module exports updated
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] No orphaned imports
- [ ] SSH connection still works
- [ ] All functionality preserved via Lua
- [ ] ~2500+ lines removed
