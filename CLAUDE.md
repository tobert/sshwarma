# sshwarma - Coding Agent Context

sshwarma is an SSH-accessible partyline where humans and AI models collaborate in shared rooms. Think MUD meets IRC meets collaborative coding—a text adventure interface for multi-user, multi-model conversations with tool access.

## Core Concepts

### The Metaphor

- **Partyline**: A room where users and models hang out. Named after telephone party lines where multiple people share a connection.
- **Lobby**: Where you land on connection. List rooms, join or create partylines.
- **Models**: AI models (qwen-8b, qwen-4b, etc.) that lurk in rooms and respond to @mentions.
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

Room Info:
  /look               Room summary (users, models, vibe, exits)
  /who                Who's in the room
  /history [n]        Recent messages
  /exits              List room exits
  /vibe [text]        Get or set room vibe

Journal:
  /journal [n]        View recent journal entries
  /note <text>        Add a note
  /decide <text>      Record a decision
  /idea <text>        Capture an idea
  /milestone <text>   Mark a milestone
  /inspire [text]     Add or view inspirations

Assets:
  /bring <id> [role]  Bind artifact to room
  /drop <id>          Unbind artifact from room
  /examine <id>       Inspect bound asset

MCP Tools:
  /mcp add <name> <url>   Connect to MCP server
  /mcp remove <name>      Disconnect MCP server
  /mcp list               List connected servers
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
│               │   others via /mcp add           │              │
│               └─────────────────────────────────┘              │
│                                                                 │
│               ┌─────────────────────────────────┐              │
│               │      LLM Backend (rig)          │              │
│               │   llama.cpp on :2020            │              │
│               │   (qwen-8b, qwen-4b)            │              │
│               │   Streaming + Tool Use          │              │
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

Navigation tools can be disabled per-room for focused sessions.

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

## Module Structure

```
src/
├── main.rs           # Entry point, server setup
├── ssh.rs            # SSH handler, @mention processing, streaming
├── commands.rs       # Slash command implementations
├── world.rs          # Rooms, messages, room state
├── player.rs         # Per-connection session state
├── model.rs          # Model registry, backend config
├── llm.rs            # LLM client with rig, streaming, tool support
├── mcp.rs            # MCP client connections (to holler, etc.)
├── mcp_server.rs     # MCP server (exposes sshwarma tools to external clients)
├── internal_tools.rs # rig-compatible tools for models (look, say, join, etc.)
├── ops.rs            # Business logic layer (room ops, journal, etc.)
├── prompt.rs         # 4-layer system prompt builder
├── db.rs             # SQLite persistence
├── config.rs         # Config loading (models.toml)
├── state.rs          # SharedState type
├── ansi.rs           # ANSI escape handling
├── line_editor.rs    # Line editing (readline-style)
├── interp.rs         # Input parsing
├── comm.rs           # Broadcast utilities
└── lib.rs            # Library exports
```

## Dependencies

- **russh**: SSH server
- **rmcp**: MCP client and server
- **rig**: LLM orchestration (agents, tools, streaming, multi-turn)
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

### Version Control

- Never use wildcards when staging files
- Add files by explicit path
- Review with `git diff --staged` before committing
- Use Co-Authored-By for model attributions

## Model Configuration

Models are configured in `models.toml`:

```toml
ollama_endpoint = "http://localhost:2020"

[[models]]
name = "qwen-8b"
display = "Qwen3-VL-8B-Instruct"
model = "qwen3-vl-8b"
backend = "llamacpp"
```

Supported backends: `llamacpp`, `ollama`, `openai`, `anthropic`, `gemini`

## Integration with Hootenanny

sshwarma is built to work with hootenanny's ecosystem:

- **holler**: MCP server exposing orpheus, musicgen, abc, audio garden, etc.
- **Artifacts**: References hootenanny's CAS and artifact system

When running with holler connected, models can:
- Generate MIDI with orpheus via `sample` tool
- Play audio on the garden
- Convert and analyze audio

## Model Attributions

```
🤖 Claude <claude@anthropic.com>
💎 Gemini <gemini@google.com>
```
