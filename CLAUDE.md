# sshwarma - Coding Agent Context

sshwarma is an SSH-accessible partyline where humans and AI models collaborate in shared rooms. Think MUD meets IRC meets collaborative coding—a text adventure interface for multi-user, multi-model conversations with tool access.

## Core Concepts

### The Metaphor

- **Partyline**: A room where users and models hang out. Named after telephone party lines where multiple people share a connection.
- **Lobby**: Where you land on connection. List rooms, join or create partylines.
- **Models**: AI models (qwen-8b, qwen-4b, future claude/gemini) that lurk in rooms and respond to @mentions.
- **Artifacts**: Things created during sessions—MIDI files, audio, text. Can be picked up, dropped, shared, played.

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
artifacts: jazzy-intro-7f3a (MIDI)

hootenanny> hey bob, let's jam

amy: hey bob, let's jam

hootenanny> @qwen-8b generate a 4-bar jazz intro

amy → qwen-8b: generate a 4-bar jazz intro

qwen-8b: Running orpheus_generate...
qwen-8b: Done. Created jazzy-intro-7f3a (MIDI, 4 bars)
```

### Input Modes

| Input | Meaning |
|-------|---------|
| `plain text` | Chat message to the room |
| `@model message` | Address a specific model |
| `/command [args]` | Execute a command |

### Commands

```
Navigation:
  /rooms              List partylines
  /join <room>        Enter a partyline
  /leave              Return to lobby
  /create <name>      New partyline

Looking:
  /look               Room summary
  /look <thing>       Examine artifact/user/model
  /who                Who's online
  /history [n]        Recent messages

Artifacts:
  /get <artifact>     Pick up into inventory
  /drop <artifact>    Leave in room
  /inv                Your inventory
  /play <artifact>    Play on audio garden
  /stop               Stop playback

Tools:
  /tools              List available MCP tools
  /run <tool> [args]  Invoke tool

Settings:
  /set flavor off     Disable generated room descriptions
  /status             Session info
  /quit               Disconnect
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         sshwarma                                │
│                                                                 │
│   ┌─────────────┐                         ┌─────────────┐      │
│   │  SSH REPL   │                         │  MCP Server │      │
│   │  (russh)    │                         │  (baton)    │      │
│   └──────┬──────┘                         └──────┬──────┘      │
│          │                                       │              │
│          └────────────► Shared World ◄───────────┘              │
│                         - Partylines                            │
│                         - Users & Models                        │
│                         - Artifacts                             │
│                                                                 │
│               ┌─────────────────────────────────┐              │
│               │         MCP Clients             │              │
│               │   holler (hootenanny tools)     │              │
│               │   exa (web search)              │              │
│               │   others...                     │              │
│               └─────────────────────────────────┘              │
│                                                                 │
│               ┌─────────────────────────────────┐              │
│               │         LLM Backend             │              │
│               │   llama.cpp on :2020            │              │
│               │   (qwen-8b, qwen-4b)            │              │
│               └─────────────────────────────────┘              │
└─────────────────────────────────────────────────────────────────┘
```

### Dual Transport

sshwarma exposes the same capabilities over two transports:

1. **SSH**: Human users connect with `ssh user@host -p 2222`
2. **MCP**: Claude Code (and other MCP clients) connect via HTTP

Both can join the same partylines, see the same messages, use the same tools.

### MCP Tool Proxy

sshwarma acts as a gateway to other MCP servers:
- Connects to holler, exa, etc. as an MCP client
- Proxies their tools via `/run <tool>` command
- Results become artifacts in the current partyline

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

### Module Structure

```
src/
├── main.rs           # SSH server, config, entry point
├── world.rs          # Partylines, artifacts, room state
├── player.rs         # Per-connection session state
├── model.rs          # Model registry, handles
├── comm.rs           # Communication: say, tell, broadcast
├── interp.rs         # Command parser
├── interp/commands.rs # Command implementations
├── mcp.rs            # MCP client (to holler) and server (to Claude Code)
├── llm.rs            # OpenAI-compatible client to llama.cpp
└── db.rs             # sqlite persistence
```

### Dependencies

- **russh**: SSH server
- **baton**: MCP client and server (from hootenanny, soon standalone)
- **async-openai**: OpenAI-compatible API client for llama.cpp
- **rusqlite**: Persistence
- **tokio**: Async runtime

## Integration with Hootenanny

sshwarma is built to work with hootenanny's ecosystem:

- **holler**: MCP server exposing orpheus, musicgen, abc, audio garden, etc.
- **baton**: MCP library for client/server
- **Artifacts**: References hootenanny's CAS and artifact system

When running with holler, users can:
- Generate MIDI with orpheus via `/run orpheus_generate`
- Play audio on the garden via `/play <artifact>`
- Convert and analyze audio

## Model Handles

Models are addressed by short names:

| Handle | Model | Backend |
|--------|-------|---------|
| `qwen-8b` | Qwen3-VL-8B-Instruct | llama.cpp :2020 |
| `qwen-4b` | Qwen3-VL-4B-Instruct | llama.cpp :2020 |
| `claude-*` | Claude variants | (future) Claude SDK |
| `gemini-*` | Gemini variants | (future) Gemini API |

## Flavor Text

Room descriptions can be generated by a small model for atmosphere:

```
───────────────────────────────────────
hootenanny
───────────────────────────────────────

    Screens show waveforms mid-edit. bob's been here a while—dozen
    artifacts scattered around. The qwen-8b seems interested in
    something about chord progressions.

users: amy, bob
models: qwen-8b, qwen-4b
```

Users can disable with `/set flavor off` for terse output.

## Version Control

Same rules as hootenanny:
- Never use wildcards when staging files
- Add files by explicit path
- Review with `git diff --staged` before committing
- Use Co-Authored-By for model attributions

## Model Attributions

```
🤖 Claude <claude@anthropic.com>
💎 Gemini <gemini@google.com>
```
