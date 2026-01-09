# sshwarma

MUD-inspired collaboration space for humans, models, and tools.

Rooms are containers for context and conversation. Multiple agents — human and model — update and experience a shared snapshot of now. Navigate a spatial world with vibes and exits. Models respond to @mentions, streaming responses and calling tools. Vim-style editing. SSH for humans, MCP for agents.

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

**Rooms** — Containers for context. `/join`, `/go north`, `/create`. Vibes, exits, shared state.

**@mentions** — `@qwen-8b explain this`. Responses stream; models see context via `wrap()` and call tools.

**Vim modes** — `Escape` for normal, `i` for insert. `j/k` to navigate, `Ctrl-u/d` to scroll.

**Tools & Equipment** — `/mcp connect` adds servers. `/inv all` shows available tools. `/equip holler:sample` binds tools to your session. Equipped tools are available to you and models you @mention.

**Dual transport** — SSH (2222) for humans, MCP (2223) for agents. Same world.

## Configuration

**Paths:** `~/.config/sshwarma/` (config), `~/.local/share/sshwarma/` (data)

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
