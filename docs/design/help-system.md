# Help System — Requirements & Ideas

**Status:** Design notes for future implementation

---

## Goals

1. **Agent-friendly** — Agents get concise context with tools, detailed help on demand
2. **Human-friendly** — SSH users can `/help <topic>` for quick reference
3. **Low maintenance** — Help docs derived from or linked to source when possible

## Two-Tier Architecture

### Tier 1: Tool Descriptions (Always Present)
- Terse summaries in MCP tool schemas
- ~1-2 sentences per tool
- Agents see these with every request

### Tier 2: Detailed Help (On Demand)
- Markdown primers in `src/embedded/help/`
- `help <topic>` tool returns full doc
- Examples, patterns, gotchas

## Proposed Tools

### `help(topic)`
Returns markdown help for a topic.

```
help("fun")     → contents of help/fun.md
help("str")     → contents of help/str.md
help("inspect") → contents of help/inspect.md
help()          → list of available topics
```

### `lib_source(name, section?)`
Returns source code of embedded libraries (read-only).

```
lib_source("fun")              → full fun.lua source
lib_source("fun", "operators") → just the operators section
lib_source("str", "split")     → just the split function
```

**Section markers** — Add comments to slow-changing source:
```lua
-- @section operators
local operator = {
    lt = function(a, b) return a < b end,
    -- ...
}
-- @endsection

-- @section generators
local range = function(start, stop, step)
    -- ...
end
-- @endsection
```

This lets agents request just what they need:
- Full source: ~7.5k tokens (fun.lua)
- Section: ~500 tokens (just operators)
- Function: ~50 tokens (just one function)

### `lib_list()`
Returns list of available libraries with brief descriptions.

```lua
{
    {name = "fun", description = "Functional programming, lazy iterators"},
    {name = "str", description = "String utilities (split, strip, etc.)"},
    {name = "inspect", description = "Pretty-print tables for debugging"},
}
```

## Integration Points

### MCP Server
Add to `mcp_server.rs`:
- `help` tool — returns help markdown
- `lib_source` tool — returns library source
- `lib_list` tool — returns library inventory

### SSH REPL
Add `/help <topic>` command:
- Uses same underlying implementation
- Formats markdown for terminal display

### Lua Runtime
Extend existing tools in `lua/tools.rs`:
- `tools.help(topic)` — for Lua scripts to access help
- `tools.lib_source(name)` — for introspection

## Help Doc Structure

```
src/embedded/help/
├── fun.md          # Luafun quick reference
├── str.md          # String utilities
├── inspect.md      # Pretty printing
├── commands.md     # Slash command reference (generated?)
├── screen.md       # Screen rendering API
└── index.md        # Topic index with descriptions
```

## Source Section Conventions

For `lib_source()` section extraction:

```lua
--- @section NAME
--- Description of this section
-- Code here
--- @endsection

--- @fn FUNCTION_NAME
--- Brief description
-- Function implementation
--- @endfn
```

Parser extracts between markers, includes doc comments.

## Agent Workflow

Typical agent interaction:

```
1. Agent sees tool list with terse descriptions
2. Agent wants to use luafun for a task
3. Agent calls help("fun") → gets quick reference
4. Agent still needs details on zip()
5. Agent calls lib_source("fun", "zip") → gets just that code
6. Agent implements solution
```

## Future Ideas

### Auto-Generated Help
- Parse `@param`, `@return` annotations from source
- Generate API reference automatically
- Keep hand-written examples separate

### Interactive Examples
- `/try fun.range(10):sum()` — eval and show result
- Sandbox for experimenting with libraries

### Context-Aware Help
- When agent makes luafun error, suggest relevant help section
- "Did you forget `:totable()`?"

### Version Markers
- Track when sections last changed
- Help docs can reference stable line numbers

---

*Ideas parking lot for future sessions.*
