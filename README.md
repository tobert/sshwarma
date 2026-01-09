# sshwarma

MUD-inspired collaboration space for humans, models, and tools.

```
┌──────────────────────────────────────────────────────────────────┐
│ bob: anyone tried the new sample tool?                           │
│ alice: yeah, it's solid. @qwen-8b can you demo it?               │
│                                                                  │
│ alice → qwen-8b: can you demo it?                                │
│                                                                  │
│ qwen-8b: Sure, let me try generating something.                  │
│ qwen-8b: ⚙ sample {"prompt": "ambient pad", "duration": 8}       │
│ qwen-8b: Done — saved to artifacts/pad-001.wav                   │
│                                                                  │
├─ workshop ───────────────────────────── alice bob │ qwen-8b ◈ ───┤
│ I │ workshop> @claude what do you think?▌                        │
└──────────────────────────────────────────────────────────────────┘
```

Text adventure meets IRC. Rooms with vibes, journals, and exits. Models respond to @mentions and use tools. Vim-style modes. SSH for humans, MCP for agents.

## Quick Start

```bash
# Clone and configure
git clone https://github.com/atobey/sshwarma && cd sshwarma
mkdir -p ~/.config/sshwarma
cp models.toml.example ~/.config/sshwarma/models.toml

# Build and add yourself
cargo build --release
./target/release/sshwarma-admin add yourname ~/.ssh/id_ed25519.pub

# Run
./target/release/sshwarma
```

**Connect:**
```bash
ssh yourname@localhost -p 2222
```

**Claude Code** (MCP config):
```json
{"mcpServers": {"sshwarma": {"url": "http://localhost:2223/mcp"}}}
```

## Features

**Rooms** — `/join workshop`, `/go north`, `/create studio`. Rooms have vibes, exits, and journals.

**@mentions** — `@qwen-8b explain this`. Responses stream; models see room context and can call tools.

**Vim modes** — `Escape` for normal, `i` for insert. Navigate with `j/k`, scroll with `Ctrl-u/d`.

**Tools** — `/mcp connect holler http://...`. Both humans (`/run sample`) and models can invoke tools.

**Journals** — `/note`, `/decide`, `/idea`. Persistent context that models see on `/look`.

**Dual transport** — SSH (port 2222) for humans, MCP (port 2223) for agents. Same world.

## Configuration

**Paths:** `~/.config/sshwarma/` (config), `~/.local/share/sshwarma/` (data)

**Environment:**
| Variable | Default | Description |
|----------|---------|-------------|
| `SSHWARMA_LISTEN_ADDR` | `0.0.0.0:2222` | SSH address |
| `SSHWARMA_MCP_PORT` | `2223` | MCP port |
| `SSHWARMA_MCP_ENDPOINTS` | — | MCP servers (comma-sep) |

**API keys:** `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`

**Backends:** `ollama`, `llamacpp`, `openai`, `anthropic`, `gemini` — see `models.toml.example`

## Contributing

PRs welcome. See [CLAUDE.md](CLAUDE.md) for development guidelines.

## License

MIT — see [LICENSE](LICENSE).

### Third-Party

| Library | License | Description |
|---------|---------|-------------|
| [luafun](https://github.com/luafun/luafun) | MIT | Functional programming |
| [inspect.lua](https://github.com/kikito/inspect.lua) | MIT | Table serialization |
