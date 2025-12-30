# sshwarma - Coding Agent Context

sshwarma is an SSH-accessible partyline where humans and AI models collaborate in shared rooms. Think MUD meets IRC meets collaborative coding—a text adventure interface for multi-user, multi-model conversations with tool access.

## Core Concepts

### The Metaphor

- **Partyline**: A room where users and models hang out. Named after telephone party lines where multiple people share a connection.
- **Lobby**: Where you land on connection. List rooms, join or create partylines.
- **Models**: AI models (qwen-8b, claude, etc.) that lurk in rooms and respond to @mentions.
- **Rooms**: Have vibes, journals, exits to other rooms, and bound assets.

### Interface Style

MUD-style text adventure with REPL ergonomics:

```
╭─────────────────────────────────────╮
│           sshwarma                  │
╰─────────────────────────────────────╯

Welcome, amy.

lobby> /rooms
Partylines:
  hootenanny ... 2 users, qwen-8b

lobby> /join hootenanny

───────────────────────────────────────
hootenanny
───────────────────────────────────────

users: amy (you), bob
models: qwen-8b, qwen-4b

hootenanny> hey bob, let's jam

amy: hey bob, let's jam

hootenanny> @qwen-8b what tools do you have?

amy → qwen-8b: what tools do you have?

qwen-8b: Here are the tools available...
```

### Input Modes

| Input | Meaning |
|-------|---------|
| `plain text` | Chat message to the room |
| `@model message` | Address a specific model (streams response) |
| `/command [args]` | Execute a command |

### Commands

```
Navigation:
  /rooms              List partylines
  /join <room>        Enter a partyline
  /leave              Return to lobby
  /create <name>      New partyline
  /fork [name]        Fork current room (inherits vibe, assets, inspirations)
  /go <direction>     Navigate through an exit
  /dig <dir> <room>   Create exit to another room
  /exits              List room exits
  /nav [on|off]       Toggle model navigation for room

Room Info:
  /look               Room summary (users, models, vibe, exits)
  /who                Who's in the room
  /history [n]        Recent messages
  /examine <role>     Inspect bound asset
  /vibe [text]        Get or set room vibe

Journal:
  /journal [kind]     View recent journal entries
  /note <text>        Add a note
  /decide <text>      Record a decision
  /idea <text>        Capture an idea
  /milestone <text>   Mark a milestone
  /inspire [text]     Add or view inspirations

Assets:
  /bring <id> as <role>  Bind artifact to room
  /drop <role>           Unbind artifact from room

MCP Tools:
  /mcp                    List connected MCP servers
  /mcp connect <name> <url>   Connect to MCP server
  /mcp disconnect <name>      Disconnect MCP server
  /mcp refresh <name>         Refresh tool list
  /tools                  List available tools
  /run <tool> [json]      Invoke MCP tool directly

System:
  /help               Show help
  /quit               Disconnect
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         sshwarma                                │
│                                                                 │
│   ┌─────────────┐                         ┌─────────────┐      │
│   │  SSH REPL   │                         │  MCP Server │      │
│   │  (russh)    │                         │  (rmcp)     │      │
│   │  port 2222  │                         │  port 2223  │      │
│   └──────┬──────┘                         └──────┬──────┘      │
│          │                                       │              │
│          └────────────► Shared World ◄───────────┘              │
│                         - Rooms & Users                         │
│                         - Journals & Vibes                      │
│                         - Bound Assets                          │
│                                                                 │
│               ┌─────────────────────────────────┐              │
│               │         MCP Clients             │              │
│               │   holler (hootenanny tools)     │              │
│               │   exa (web search)              │              │
│               │   others via /mcp connect       │              │
│               └─────────────────────────────────┘              │
│                                                                 │
│               ┌─────────────────────────────────┐              │
│               │      LLM Backend (rig)          │              │
│               │   Ollama, llama.cpp, OpenAI     │              │
│               │   Anthropic, Gemini             │              │
│               │   Streaming + Tool Use          │              │
│               └─────────────────────────────────┘              │
│                                                                 │
│               ┌─────────────────────────────────┐              │
│               │      Lua Runtime (HUD)          │              │
│               │   Per-user scripts              │              │
│               │   Hot-reloading                 │              │
│               └─────────────────────────────────┘              │
└─────────────────────────────────────────────────────────────────┘
```

### Dual Transport

sshwarma exposes the same capabilities over two transports:

1. **SSH**: Human users connect with `ssh user@host -p 2222`
2. **MCP**: Claude Code (and other MCP clients) connect to port 2223

Both can interact with the same rooms and see the same state.

### Internal Tools

When a model is @mentioned, it gets access to internal tools via rig's tool system:

| Tool | Description |
|------|-------------|
| `look` | Get room info (users, models, vibe, exits) |
| `who` | List users in room |
| `rooms` | List all rooms |
| `history` | Get recent messages |
| `exits` | List room exits |
| `journal` | Get journal entries |
| `tools` | List available MCP tools |
| `say` | Say something to the room |
| `vibe` | Get or set room vibe |
| `note`, `decide`, `idea`, `milestone` | Add journal entries |
| `inspire` | Add or get inspirations |
| `join`, `leave`, `go` | Navigation (if enabled for room) |
| `create`, `fork` | Room creation (if enabled) |

Navigation tools can be disabled per-room via `/nav off` for focused sessions where you don't want models wandering.

### MCP Tool Proxy

sshwarma acts as a gateway to other MCP servers:
- Connects to holler, exa, etc. as an MCP client
- Models can use these tools when @mentioned
- Results can become artifacts in the room

### Schema Normalization

For llama.cpp compatibility, MCP tool schemas are normalized:
- Strips `"default"` keys (llama.cpp can't parse them)
- Adds `"type": "object"` to description-only schemas

Other backends receive full schemas unchanged.

## The Lua HUD

The heads-up display is rendered by Lua, making it fully customizable. An 8-line fixed region at the bottom of the terminal shows participants, MCP connections, room info, and notifications.

### How It Works

1. **Embedded default**: Ships with `src/embedded/hud.lua`
2. **User override**: Drop a script in `~/.config/sshwarma/hud.lua`
3. **Per-user scripts**: `~/.config/sshwarma/{username}.lua` for individual customization
4. **Hot-reloading**: Scripts are checked every second; no restart needed
5. **Example configs**: See `configs/` directory for shareable user scripts

### Lua API

```lua
-- Core state (read-only, updated by Rust)
tools.hud_state()              -- Get room/participant/MCP state
tools.clear_notifications()    -- Drain notification queue

-- KV Store (persistent across calls, shared between background and render)
tools.kv_get(key)              -- Read value or nil
tools.kv_set(key, value)       -- Write value
tools.kv_delete(key)           -- Remove key

-- Async MCP Operations (for background polling)
tools.mcp_call(server, tool, args)  -- Returns request_id immediately
tools.mcp_result(request_id)        -- Returns (result, status)
                                    -- status: "pending"|"complete"|"error"|"timeout"
```

### Required Functions

```lua
-- Called every ~100ms for rendering
function render_hud(now_ms, width, height)
    local ctx = tools.hud_state()
    local rows = {}
    -- Build 8 rows of segments: {Text = "...", Fg = "#rrggbb"}
    return rows
end

-- Optional: Called every 500ms (120 BPM) for background work
function background(tick)
    -- tick % 1 == 0: every 500ms
    -- tick % 2 == 0: every 1s
    -- tick % 4 == 0: every 2s
    if tick % 4 == 0 then
        poll_artifacts()  -- Your polling logic
    end
end
```

### Available State

The `tools.hud_state()` function returns:

```lua
{
    participants = {
        {name = "alice", kind = "user", status = "idle"},
        {name = "qwen-8b", kind = "model", status = "thinking", status_detail = "sample"},
    },
    mcp = {
        {name = "holler", tools = 12, calls = 3, last_tool = "sample", connected = true},
        {name = "exa", tools = 2, calls = 0, connected = true},
    },
    room = "workshop",
    vibe = "collaborative coding",
    exits = {n = "studio", e = "garden"},
    session_start_ms = 1234567890,
}
```

### Colors

`#rrggbb` hex codes, or use a palette: `dim`, `cyan`, `blue`, `green`, `yellow`, `red`, `orange`, `magenta`

### Status Glyphs

```
Agent status:   ◈ active   ◇ idle   ◌ offline   ◉ error
Spinners:       ⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏  (100ms cycle)
Progress:       █████░░░░░
Exit arrows:    ↑ north  → east  ↓ south  ← west
```

## Module Structure

```
src/
├── main.rs               # Entry point, server setup, host key management
├── ssh.rs                # SSH handler, @mention processing, streaming, HUD refresh
├── state.rs              # SharedState: Arc-wrapped world, db, llm, models, mcp
│
├── world.rs              # Room, Message, User, Artifact types
├── player.rs             # Per-connection session state (username, room, inventory)
├── ops.rs                # Pure business logic (look, who, rooms, history, exits, journal)
├── commands.rs           # Slash command implementations
│
├── interp/               # Input parsing
│   ├── mod.rs            # ParsedInput enum (Command, Mention, Chat, Empty)
│   └── commands.rs       # Command dispatch and help text
│
├── display/              # Terminal rendering
│   ├── mod.rs            # DisplayBuffer: manages render state
│   ├── ledger.rs         # Append-only conversation log, placeholder tracking
│   ├── renderer.rs       # Formats entries to terminal with ANSI codes
│   ├── styles.rs         # ANSI color codes, Tokyo Night palette
│   └── hud/              # Heads-up display
│       ├── mod.rs        # HUD module exports
│       ├── state.rs      # HudState: participants, MCP, notifications, presence
│       ├── renderer.rs   # Calls Lua to render HUD
│       └── spinner.rs    # Spinner animation frames
│
├── lua/                  # Lua scripting for HUD
│   ├── mod.rs            # LuaRuntime: state management, script loading, hot-reload
│   ├── context.rs        # Build HudContext to pass to Lua
│   ├── render.rs         # Parse Lua output into terminal segments
│   ├── tools.rs          # Register Lua callbacks (hud_state, kv_*, mcp_*)
│   ├── cache.rs          # ToolCache: KV store for background→render data sharing
│   └── mcp_bridge.rs     # Async MCP bridge: sync Lua ↔ async MCP calls
│
├── embedded/             # Compiled-in resources
│   └── hud.lua           # Default HUD script
│
├── configs/              # Example user HUD scripts (symlink to ~/.config/sshwarma/)
│   └── atobey.lua        # Example: holler integration with garden/job polling
│
├── completion/           # Tab completion
│   ├── mod.rs            # Completion engine
│   ├── commands.rs       # Complete slash commands
│   ├── rooms.rs          # Complete room names
│   ├── models.rs         # Complete model names
│   └── tools.rs          # Complete tool names
│
├── model.rs              # ModelRegistry, ModelHandle, backend enum
├── llm.rs                # LLM client with rig (streaming, tool calling)
├── prompt.rs             # 4-layer system prompt builder
├── internal_tools.rs     # rig-compatible tools for models (look, say, join, etc.)
│
├── mcp.rs                # MCP client: connect to external servers, call tools
├── mcp_server.rs         # MCP server: expose sshwarma to Claude Code
│
├── db.rs                 # SQLite: users, pubkeys, rooms, messages, artifacts
├── config.rs             # Config/ModelsConfig: load from toml, env vars
│
├── ansi.rs               # ANSI escape sequence parser
├── line_editor.rs        # Readline-style input with completion
├── comm.rs               # Broadcast utilities
└── lib.rs                # Library exports

src/bin/
└── sshwarma-admin.rs     # CLI for user management (add-user, etc.)
```

## Dependencies

- **russh**: SSH server
- **rmcp**: MCP client and server (official Rust SDK)
- **rig**: LLM orchestration (agents, tools, streaming, multi-turn)
- **mlua**: Lua scripting (Luau flavor, Send+Sync for async)
- **rusqlite**: SQLite persistence
- **tokio**: Async runtime
- **tracing**: Structured logging

## Development Guidelines

### Error Handling

- Use `anyhow::Result` for all fallible operations
- Never use `unwrap()` - propagate errors with `?`
- Add context with `.context()` for debugging

### Code Style

- Prioritize clarity over cleverness
- Rich types: avoid primitive obsession (use newtypes for IDs)
- Comments explain "why", not "what"
- No organizational comments

### Async/Blocking Patterns

When calling `blocking_read()` or `blocking_write()` on `RwLock` from async contexts (like `async fn` handlers), wrap them with `tokio::task::block_in_place()` to avoid panics:

```rust
// BAD: panics in async context
let world = self.state.world.blocking_write();

// GOOD: safe in async context
let world = tokio::task::block_in_place(|| self.state.world.blocking_write());
```

This applies to commands.rs, internal_tools.rs, and any async code that needs synchronous lock access.

### Version Control

- Never use wildcards when staging files
- Add files by explicit path
- Review with `git diff --staged` before committing
- Use Co-Authored-By for model attributions

## Model Configuration

Models are configured in `models.toml`:

```toml
ollama_endpoint = "http://localhost:11434"
llamacpp_endpoint = "http://localhost:2020"

[[models]]
name = "qwen-8b"
display = "Qwen3-8B"
model = "qwen3:8b"
backend = "ollama"

[[models]]
name = "local"
display = "Local Model"
model = "qwen3-vl-8b"
backend = "llamacpp"
```

Supported backends: `llamacpp`, `ollama`, `openai`, `anthropic`, `gemini`, `mock`

## Integration with Hootenanny

sshwarma is built to work with hootenanny's ecosystem:

- **holler**: MCP server exposing orpheus, musicgen, abc, audio garden, etc.
- **Artifacts**: References hootenanny's CAS and artifact system

When running with holler connected, models can:
- Generate MIDI with orpheus via `sample` tool
- Play audio on the garden
- Convert and analyze audio

## Open Questions

Design decisions still being explored:

1. **HUD height** — 4, 6, or 8 lines? More content vs more scroll buffer.

2. **Room event buffer** — Should `/look` include recent events? How many? How old? Feels right for agents to see "what just happened" not just "who's here."

3. **Agent following** — When you `/go north`, do agents follow? All of them? Just active ones? Just the one you're talking to?

4. **Context in HUD** — Show per-agent context usage, or just aggregate total?

5. **Notification persistence** — Log all notifications to scroll buffer, or just show in HUD ephemerally?

6. **Room-specific tool scoping** — Should different rooms have different tools available? A "music studio" room that only has audio tools?

## Future Directions

Ideas worth exploring:

- **Sixel/Kitty graphics** — Room maps, waveforms, album art rendered inline
- **Agent-to-agent orchestration** — Syntax for agents to delegate to each other
- **Transport controls** — Full playback controls in HUD when holler connected
- **Room templates** — Pre-configured rooms for specific workflows (jam session, code review, brainstorm)
- **Presence indicators** — Show typing, thinking, tool-running states more richly
- **Multi-room views** — Split view showing activity in multiple rooms

## Building Forward

If you're an agent working on sshwarma, here are good extension points:

### Easy Wins
- Add new internal tools in `internal_tools.rs` (follow the existing pattern)
- Create alternative HUD layouts in Lua
- Add tab completions in `completion/`

### Medium Complexity
- New slash commands in `commands.rs` → wire them in `interp/commands.rs`
- Extend `HudState` with new data → expose it in `lua/context.rs`
- Add persistence for new data types in `db.rs`

### Architectural
- New transports (WebSocket? Telnet for retro vibes?)
- Agent-to-agent communication channels
- Room inheritance and templating system

### Questions to Ask
- What would make the room metaphor more useful?
- How should agents coordinate when multiple are active?
- What context do agents need that they're not getting?

## Model Attributions

```
🤖 Claude <claude@anthropic.com>
💎 Gemini <gemini@google.com>
```
