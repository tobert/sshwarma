# Code Review: sshwarma

## Overview
`sshwarma` is a Rust-based SSH application acting as a collaborative space (MUD-style) for humans and LLMs. It integrates `russh` for the SSH layer, `rig` for LLM orchestration, `rmcp` for the Model Context Protocol, and `rusqlite` for persistence.

The project is functional but contains critical logical errors ("confabulations"), significant technical debt, and some architectural inconsistencies that should be addressed before a stable release.

## Critical Issues & Confabulations

### 1. MCP Tool Routing Bug (Critical)
**File:** `src/mcp.rs`
**Method:** `McpClients::rig_tools()`

The method aggregates tools from **all** connected MCP clients but creates a `RigToolContext` using only the peer of the **first** connection:

```rust
// src/mcp.rs:136
let first_conn = clients.values().next()?;
let peer = first_conn.service.peer().to_owned();
// ... aggregates tools from ALL clients ...
```

**Impact:** If multiple MCP servers are connected (e.g., "filesystem" and "github"), all tool calls will be routed to the first one found in the map. Calls to tools belonging to other servers will fail.
**Fix:** The `RigToolContext` or the `rig` integration needs to support multiple peers, or `ToolServer` needs to route based on tool name/source.

### 2. Incomplete LLM Streaming
**File:** `src/llm.rs`
**Method:** `chat_stream`

The method signature implies streaming, but the implementation falls back to blocking/complete execution:

```rust
// src/llm.rs:437
// For now, fall back to non-streaming and send result all at once
// TODO: implement proper streaming with rig's stream_prompt
```

**Impact:** Users experience higher latency as they must wait for the full generation before seeing any output.

### 3. Ignored Conversation History
**File:** `src/llm.rs`
**Method:** `chat_with_context`

The `_history` parameter is unused:

```rust
// src/llm.rs:115
pub async fn chat_with_context(
    &self,
    model: &ModelHandle,
    system_prompt: &str,
    _history: &[(String, String)], // TODO: integrate history
```

**Impact:** Models have no memory of previous turns in the conversation unless context is manually injected into the `message` or `system_prompt` by the caller (which `ssh.rs` partially does via Lua `wrap`, but the API signature is misleading).

### 4. LlamaCpp Schema Hack in SSH Handler
**File:** `src/ssh.rs`
**Method:** `normalize_schema_for_llamacpp`

The SSH handler contains specific logic to strip "default" keys and fix schemas for `llama.cpp`.

**Impact:** This logic violates separation of concerns. It belongs in `src/llm.rs` or a model adapter layer, not in the SSH session handler.

## Technical Debt

### 1. God Object: `SshHandler`
**File:** `src/ssh.rs`

The `SshHandler` struct manages:
- SSH session state
- Line editing (`LineEditor`)
- Terminal display & ANSI parsing
- HUD rendering (via Lua)
- Task spawning (multiple tasks per session)
- Tool server creation and registration
- LLM interaction

**Recommendation:** Split this into smaller components. For example:
- `SessionRenderer`: Handle `DisplayBuffer`, `Ledger`, and ANSI output.
- `AgentOrchestrator`: Handle `ToolServer` creation and LLM interaction.

### 2. Database Concurrency & Migrations
**File:** `src/db.rs`

*   **Panic Safety:** Uses `self.conn.lock().unwrap()`. If a thread panics while holding the lock, the DB becomes poisoned and the server will crash on subsequent accesses.
*   **Ad-hoc Migrations:** `run_migrations` executes `ALTER TABLE` commands blindly and ignores errors.
    ```rust
    // src/db.rs
    let _ = conn.execute("ALTER TABLE ...", []);
    ```
    This masks legitimate errors and makes schema evolution fragile. Use `user_version` pragma or a migration table to track schema state.

### 3. Code Duplication in `LlmClient`
**File:** `src/llm.rs`

The methods `chat`, `chat_with_tools`, `chat_with_tool_server`, and `stream_with_tool_server` all repeat large `match` blocks for every backend (`Ollama`, `LlamaCpp`, `OpenAI`, `Anthropic`).

**Recommendation:** Refactor backend dispatch into a trait or helper method to centralize the logic.

### 4. Confusing Module Structure
*   `src/lib.rs` exports both `commands` and `interp::commands`.
*   `src/interp/commands.rs` is effectively empty/placeholder.
*   `src/embedded/` contains Lua scripts that are included via `include_str!`.

## Consistency & Style

*   **Error Handling:** Inconsistent use of `anyhow::Result` vs specific errors. `src/internal_tools.rs` converts `anyhow` to `std::io::Error` boxed in `ToolError`, which loses error context.
*   **Hardcoded Values:**
    *   `HUD_HEIGHT` (8) is hardcoded in `src/display/hud/mod.rs` (inferred) and used in `ssh.rs`.
    *   Magic numbers for token limits and ledger sizes.

## Missing Features (stubbed)
*   **Gemini Support:** `src/llm.rs` has a `Gemini` match arm that returns "not yet supported".
*   **MCP Server:** `src/mcp.rs` contains a TODO to implement sshwarma's own MCP server to expose its internal tools to outside agents (like Claude Code).

## Action Plan
1.  **Fix MCP Routing:** Rewrite `McpClients::rig_tools` to properly handle multiple peers or route tools correctly.
2.  **Refactor `LlmClient`:** Remove code duplication and implement proper streaming.
3.  **Clean up `SshHandler`:** Extract model/tool logic into a dedicated struct.
4.  **Harden Database:** Implement proper migration tracking and safer locking.
