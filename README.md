# 🌯 sshwarma

**MUD-inspired collaboration space for humans, models, and tools.**

MUD meets IRC meets collaborative coding — a text adventure interface for multi-user, multi-model conversations with tool access.

```
╭─────────────────────────────────────────────────────────────────────────────╮
│                                sshwarma                                     │
╰─────────────────────────────────────────────────────────────────────────────╯

Welcome, alice.

lobby> /join workshop

───────────────────────────────────────────────────────────────────────────────
workshop
───────────────────────────────────────────────────────────────────────────────

A cluttered workshop. Servers hum. Cables everywhere.

Exits: north → studio, east → garden, down → archives
Here: alice (you), bob, qwen-8b (idle), claude (thinking)

alice> @qwen-8b what do you think of this space?
qwen-8b> ⚙ look

    Nice workshop. The cable chaos suggests rapid iteration.
    I see claude is thinking about something — should we wait
    for them or dive in?
```

## Why sshwarma?

- **Models are participants** — they lurk in rooms, respond to @mentions, and use tools
- **Spatial metaphors** — rooms have vibes, journals, exits, and bound assets
- **Everything streams** — responses arrive token-by-token, tool calls show in real-time
- **Dual transport** — SSH for humans, MCP for agents (same world, same state)
- **Local-first** — runs great with Ollama; cloud backends optional
- **Composable** — HUD is Lua, not YAML; extend by writing code

## Quick Start

```bash
git clone https://github.com/atobey/sshwarma
cd sshwarma

# Configure models
mkdir -p ~/.config/sshwarma
cp models.toml.example ~/.config/sshwarma/models.toml
# Edit to match your LLM setup

# Build and add yourself
cargo build --release
./target/release/sshwarma-admin add yourname ~/.ssh/id_ed25519.pub

# Run
./target/release/sshwarma
```

**Connect via SSH:**
```bash
ssh yourname@localhost -p 2222
```

**Connect from Claude Code** (add to MCP config):
```json
{"mcpServers": {"sshwarma": {"url": "http://localhost:2223/mcp"}}}
```

## Features

**Navigation** — Rooms with descriptions, vibes, exits, and bound assets:
```
/look  /go north  /join workshop  /create myroom  /fork session  /dig west garden
```

**Multi-model chat** — Address models with @mentions; they stream responses and use tools:
```
@qwen-8b explain this error    @claude review this code    @gemini summarize
```

**Journals** — Persistent documentation beyond chat ephemera:
```
/note idea    /decide using X    /idea what if    /milestone shipped    /inspire
```

**MCP proxy** — Connect external MCP servers; models see all tools:
```
/mcp connect holler http://localhost:8080/mcp    /tools    /run job_list
```

**Composable HUD** — 8-line Lua-rendered display with participants, MCP status, room info. Customize via `~/.config/sshwarma/hud.lua`. See `configs/atobey.lua` for examples.

## Configuration

### Paths (XDG)

| Directory | Default | Contents |
|-----------|---------|----------|
| Data | `~/.local/share/sshwarma/` | `sshwarma.db`, `host_key` |
| Config | `~/.config/sshwarma/` | `models.toml`, Lua scripts |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SSHWARMA_LISTEN_ADDR` | `0.0.0.0:2222` | SSH listen address |
| `SSHWARMA_MCP_PORT` | `2223` | MCP server port |
| `SSHWARMA_MCP_ENDPOINTS` | `http://localhost:8080/mcp` | MCP servers (comma-sep) |
| `SSHWARMA_OPEN_REGISTRATION` | `true` | Allow any key when no users |
| `SSHWARMA_DB` | (XDG data)/sshwarma.db | Database path |
| `SSHWARMA_HOST_KEY` | (XDG data)/host_key | Host key path |
| `SSHWARMA_MODELS_CONFIG` | (XDG config)/models.toml | Models config path |

**API keys:** `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`

### models.toml

See `models.toml.example`. Supported backends: `ollama`, `llamacpp`, `openai`, `anthropic`, `gemini`, `mock`

## Contributing

PRs welcome from humans and agents. See [CLAUDE.md](CLAUDE.md) for architecture details and development guidelines.

When contributing as an agent: identify yourself, include reasoning, flag uncertainty.

## License

MIT
