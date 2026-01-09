# sshwarma - Coding Agent Context

MUD-inspired collaboration space for humans, models, and tools. SSH for humans (port 2222), MCP for agents (port 2223). Rooms with vibes, journals, and tool access.

## Architecture Overview

```
SSH (russh:2222) ──┐                    ┌── MCP Clients (holler, exa, etc.)
                   ├──► Shared World ◄──┤
MCP (rmcp:2223) ───┘    (SQLite)        └── LLM Backends (ollama, anthropic, etc.)
                            │
                    Lua Runtime (mlua/Luau)
                    - Screen rendering
                    - Input handling
                    - Context composition
```

## Source Layout

```
src/
├── ssh/           # SSH handler, screen rendering, input, streaming
├── db/            # SQLite: agents, buffers, rows, rooms, rules, scripts
├── lua/           # LuaRuntime, tools API, render, mcp_bridge
├── ui/            # RenderBuffer, DrawContext (Rust side)
├── mcp/           # MCP client connections
├── embedded/      # Lua source (see below)
├── main.rs        # Entry point
├── state.rs       # SharedState
├── llm.rs         # LLM client (rig)
├── model.rs       # ModelRegistry
├── internal_tools.rs  # Tools for @mentioned models
├── mcp_server.rs  # MCP server for Claude Code
└── ops.rs         # Pure business logic
```

### Embedded Lua (`src/embedded/`)

```
embedded/
├── init.lua       # Module loader, searcher
├── screen.lua     # Main render loop, on_tick, display_width
├── wrap.lua       # LLM context composition
├── ui/
│   ├── bars.lua   # Bar definitions, layout
│   ├── layout.lua # 1-indexed row computation
│   ├── input.lua  # Input buffer state
│   ├── mode.lua   # Vim-style normal/insert modes
│   ├── pages.lua  # Page navigation (chat, help, etc.)
│   └── scroll.lua # Scroll state management
├── lib/
│   ├── fun.lua    # luafun - functional programming
│   ├── str.lua    # String utilities
│   ├── inspect.lua # Pretty printing
│   └── help.lua   # Help system
└── commands/      # Slash command handlers
```

## Development Guidelines

### Lua Style

**Use luafun for iteration** — it's bundled and idiomatic:
```lua
local fun = require 'fun'

-- Prefer this
fun.iter(items):filter(pred):map(transform):totable()

-- Over this
local result = {}
for _, item in ipairs(items) do
    if pred(item) then table.insert(result, transform(item)) end
end
```

See `/help fun` in-app or `src/embedded/help/fun.md` for full reference.

**Luau module loading**: Modules must be preloaded in `LuaRuntime::new()`:
```rust
// BOTH steps required:
modules.insert("foo".to_string(), FOO_MODULE);  // for get_embedded_module
let chunk = lua.load(FOO_MODULE).set_name("embedded:lib/foo.lua").eval::<Table>()?;
loaded.set("foo", chunk)?;  // for require()
```

**Multiple return values**: Wrap in parens to capture only first:
```lua
-- BAD: gsub returns (result, count)
table.insert(lines, chunk:gsub("%s+$", ""))

-- GOOD
table.insert(lines, (chunk:gsub("%s+$", "")))
```

### Rust Style

- `anyhow::Result` for all fallible operations, propagate with `?`
- Add context with `.context()` for debugging
- Rich types over primitives (newtypes for IDs)
- Comments explain "why", not "what"

**Async/blocking**: Wrap blocking locks in async contexts:
```rust
// BAD: panics
let world = self.state.world.blocking_write();

// GOOD
let world = tokio::task::block_in_place(|| self.state.world.blocking_write());
```

### Version Control

**Commit workflow:**
1. Run `cargo test` before committing
2. **Always** stage files by explicit path
3. Review with `git diff --staged`
4. Commit with Co-Authored-By attribution

Use `git -C /path/to/repo` for precision.

**History philosophy:**
- Each commit on main is permanent - treat it that way
- Fix commits are good commits - they document the journey
- When something breaks, create a new fix commit
- Reserve `--amend` for typos caught immediately, before any other work

### Testing

```bash
cargo test                    # All tests
cargo test lua::              # Specific module
cargo test --test e2e         # E2E tests (MCP server)
```

## Tools API (Lua)

~90 functions exposed via `tools.*`. Key categories:

| Category | Functions |
|----------|-----------|
| State | `look`, `who`, `exits`, `vibe`, `session`, `rooms` |
| History | `history`, `history_tools`, `history_stats` |
| Journal | `journal`, `journal_add`, `inspirations`, `inspire` |
| Navigation | `join`, `leave`, `go`, `create`, `fork`, `dig` |
| Input | `input`, `set_input`, `set_cursor_pos` |
| Display | `mark_dirty`, `display_width`, `truncate` |
| MCP | `mcp_call`, `mcp_result`, `mcp_connections`, `mcp_tools` |
| Things | `things_get`, `things_find`, `equipped_list`, `equip` |
| Prompts | `prompts`, `prompt_set`, `prompt_push`, `get_prompt` |
| Rules | `rules`, `rules_add`, `rules_del` |
| Cache | `kv_get`, `kv_set`, `kv_delete` |
| Logging | `log_info`, `log_warn`, `log_error`, `notify` |

Full signatures: `src/lua/tools.rs`

## Key Patterns

### Adding a Slash Command

1. Create handler in `src/embedded/commands/mycommand.lua`
2. Register in `src/embedded/commands/init.lua`
3. Add help in `src/embedded/help/mycommand.md`

### Adding a Lua API Function

1. Add in `src/lua/tools.rs`:
```rust
let my_fn = lua.create_function(|_, ()| Ok("result"))?;
tools.set("my_func", my_fn)?;
```

### Adding an Internal Tool (for models)

1. Add struct in `src/internal_tools.rs` implementing `rig::tool::Tool`
2. Register in `build_tools()`

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `SSHWARMA_LISTEN_ADDR` | `0.0.0.0:2222` | SSH listen address |
| `SSHWARMA_MCP_PORT` | `2223` | MCP server port |
| `SSHWARMA_DB` | XDG data dir | SQLite path |

Backend API keys: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`

## Dependencies

- **russh** — SSH server
- **rmcp** — MCP client/server
- **rig** — LLM orchestration
- **mlua** — Lua (Luau flavor)
- **rusqlite** — SQLite
- **tokio** — Async runtime

## Attribution

```
🤖 Claude <claude@anthropic.com>
💎 Gemini <gemini@google.com>
```
