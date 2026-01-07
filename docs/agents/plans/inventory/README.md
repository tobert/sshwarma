# Room Inventory: Bags + Equipped

## Vision

A MUD-inspired collaboration space where things live in bags (tree containment) and rooms/agents equip what they need (separate relationship).

**Core concepts:**
- **Things** — Everything in the system (rooms, agents, tools, data)
- **Bags** — Things that contain other things (rooms, agents, MCPs)
- **Equipped** — What's active in a context (room equips tools)

## The World Tree

```
world (container)
├── rooms (container)
│   ├── lobby (room)
│   │   └── prompt:welcome (data, room-specific)
│   └── workshop (room)
│       └── prompt:code-style (data, room-specific)
├── agents (container)
│   └── alice (agent)
│       └── prompt:my-style (data, personal)
├── mcps (container)
│   └── holler (mcp)
│       ├── sample (tool)
│       └── play (tool)
├── internal (container)
│   ├── look (tool, qualified: sshwarma:look)
│   ├── say (tool, qualified: sshwarma:say)
│   └── ...
├── defaults (container)
│   └── (no things - just equipped rows pointing to internal)
└── home (room, shared resources)
    └── prompt:common-style (data)
```

**Equipped relationships:**
```
defaults ──equips──► sshwarma:look (from internal)
defaults ──equips──► sshwarma:say (from internal)
...
lobby ──equips──► sshwarma:look (copied from defaults on creation)
lobby ──equips──► sshwarma:say
```


## Design Principles

### Simple tree containment

Things have one parent. `parent_id` is the only structural relationship.
- Rooms contain room-specific data
- Agents contain personal data
- MCPs contain the tools they provide
- No constraints on what kinds can parent what kinds

### Equipped is separate

Equipped is a many-to-many relationship, not containment.
- A tool lives in one place (its MCP or internal)
- Multiple rooms can equip the same tool
- Unequipping doesn't delete the tool

### Soft deletes everywhere

Nothing is hard-deleted. `deleted_at` preserves history.
- Manual garbage collection later if needed

### MUD interface, simple internals

Users see MUD commands. The bags/equipped model is hidden.
- `/look` not "query things where parent_id=room"
- `/inv` not "select from equipped join things"

## Schema

### Things (tree of everything)

```sql
CREATE TABLE things (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES things(id),
    kind TEXT NOT NULL,           -- 'container', 'room', 'agent', 'mcp', 'tool', 'data', 'reference'
    name TEXT NOT NULL,
    qualified_name TEXT,          -- 'holler:sample', 'sshwarma:look'
    description TEXT,

    -- Kind-specific
    content TEXT,                 -- For 'data' kind: inline content
    uri TEXT,                     -- For 'reference' kind: external URI
    metadata TEXT,                -- JSON: vibe, config, etc.

    -- Status
    available INTEGER DEFAULT 1,  -- MCP connected? Tool working?

    -- Lifecycle
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX idx_things_parent ON things(parent_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_things_kind ON things(kind) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX idx_things_qualified ON things(qualified_name)
    WHERE deleted_at IS NULL AND qualified_name IS NOT NULL;
```

### Equipped (what's active)

```sql
CREATE TABLE equipped (
    context_id TEXT NOT NULL REFERENCES things(id),  -- room or agent
    thing_id TEXT NOT NULL REFERENCES things(id),    -- tool or data
    priority REAL DEFAULT 0.0,
    created_at INTEGER NOT NULL,
    deleted_at INTEGER,
    PRIMARY KEY (context_id, thing_id)
);

CREATE INDEX idx_equipped_context ON equipped(context_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_equipped_thing ON equipped(thing_id) WHERE deleted_at IS NULL;
```

### Exits (room connections)

```sql
CREATE TABLE exits (
    from_room_id TEXT NOT NULL REFERENCES things(id),
    direction TEXT NOT NULL,
    to_room_id TEXT NOT NULL REFERENCES things(id),
    created_at INTEGER NOT NULL,
    deleted_at INTEGER,
    PRIMARY KEY (from_room_id, direction)
);

CREATE INDEX idx_exits_from ON exits(from_room_id) WHERE deleted_at IS NULL;
```

## Thing Kinds

| Kind | Description | Can contain | Example |
|------|-------------|-------------|---------|
| `container` | Structural grouping | (any) | rooms, agents, mcps, defaults |
| `room` | Collaboration space | data, reference | workshop, lobby, home |
| `agent` | Actor (human, model, bot) | data, reference | alice, qwen-8b |
| `mcp` | External tool provider | tool | holler, exa |
| `tool` | Invokable capability | (any) | holler:sample |
| `data` | Inline content | (any) | prompt:code-style |
| `reference` | URI to external resource | (any) | ref:design-doc |

No constraints on parent-child kinds. Conventions, not enforcement.

## Commands

### Inventory

```
/inv                     # Equipped + room's own bag
/inv me                  # Equipped + personal bag
/inv all                 # Above + available from MCPs/internal
/examine holler:sample   # Thing details
```

### Equip/Unequip

```
/equip room holler:*     # Equip tools (creates equipped rows)
/unequip room holler:*   # Unequip (soft-delete equipped rows)
/drop room prompt:local  # Delete thing from room's bag
```

### Navigation

```
/look                    # Room description (MUD style)
/go north                # Follow exit
/portal north studio     # Create exit (one-way)
/exits                   # List exits
```

### Room

```
/rooms                   # List all rooms
/join workshop           # Enter room
/leave                   # Return to lobby
/create myroom           # Create new room (copies from defaults)
```

## Operations

| Command | What happens |
|---------|--------------|
| `/equip room X` | INSERT into equipped (context_id=room, thing_id=X) |
| `/unequip room X` | SET deleted_at on equipped row |
| `/drop room X` | SET deleted_at on thing (must be in room's bag) |
| `/portal dir target` | INSERT into exits |
| MCP connects | Upsert MCP thing + tool things, available=1 |
| MCP disconnects | UPDATE tools SET available=0 |
| Create room | Insert room thing, copy equipped rows from defaults |

## wrap() and Tools

When a model is @mentioned, wrap() merges room + agent equipped:

```lua
function get_equipped_tools(room_id, agent_id)
    -- Tools equipped by room OR agent, room wins conflicts
    -- Available tools only
    return db.query([[
        SELECT DISTINCT t.* FROM things t
        JOIN equipped e ON e.thing_id = t.id
        WHERE e.context_id IN (?, ?)
          AND e.deleted_at IS NULL
          AND t.deleted_at IS NULL
          AND t.kind = 'tool'
          AND t.available = 1
        ORDER BY
          CASE WHEN e.context_id = ? THEN 0 ELSE 1 END,  -- room first
          e.priority
    ]], room_id, agent_id, room_id)
end

function get_equipped_data(room_id, agent_id)
    -- Data equipped by room OR agent (prompts, etc.)
    return db.query([[
        SELECT DISTINCT t.* FROM things t
        JOIN equipped e ON e.thing_id = t.id
        WHERE e.context_id IN (?, ?)
          AND e.deleted_at IS NULL
          AND t.deleted_at IS NULL
          AND t.kind = 'data'
        ORDER BY
          CASE WHEN e.context_id = ? THEN 0 ELSE 1 END,
          e.priority
    ]], room_id, agent_id, room_id)
end
```

## Tool Call Rows

Tool calls are recorded as rows in the room's chat buffer:

```
tool.call      - Tool invocation
tool.result    - Tool output (linked via parent_row_id)
tool.error     - Tool failure
tool.timeout   - Tool timeout
```

Rows are tagged with `tool:<qualified_name>` for filtering.

## Default Tools

New rooms get these tools equipped (copied from defaults collection):

**Core:**
```
sshwarma:look      - Describe room
sshwarma:who       - List participants
sshwarma:say       - Send message
sshwarma:history   - View conversation
sshwarma:vibe      - Get/set room vibe
sshwarma:inventory - Query inventory
```

**Journal:**
```
sshwarma:journal   - Read journal
sshwarma:note      - Add note
sshwarma:decide    - Record decision
sshwarma:idea      - Capture idea
sshwarma:milestone - Mark milestone
```

**Navigation:**
```
sshwarma:rooms     - List rooms
sshwarma:exits     - List exits
sshwarma:join      - Join room
sshwarma:leave     - Leave room
sshwarma:go        - Navigate exit
sshwarma:create    - Create room
```

To disable navigation: `/unequip room sshwarma:join,leave,go,create`

## Bootstrap Data

```sql
-- Root
INSERT INTO things (id, kind, name) VALUES ('world', 'container', 'world');

-- Top-level containers
INSERT INTO things (id, parent_id, kind, name) VALUES
    ('rooms', 'world', 'container', 'rooms'),
    ('agents', 'world', 'container', 'agents'),
    ('mcps', 'world', 'container', 'mcps'),
    ('internal', 'world', 'container', 'internal'),
    ('defaults', 'world', 'container', 'defaults');

-- Home is a special room for shared resources
INSERT INTO things (id, parent_id, kind, name, metadata) VALUES
    ('home', 'world', 'room', 'home', '{"vibe": "Shared resources"}');

-- Internal tools (canonical, unique qualified_name)
INSERT INTO things (id, parent_id, kind, name, qualified_name, description) VALUES
    ('tool_look', 'internal', 'tool', 'look', 'sshwarma:look', 'Describe current room'),
    ('tool_say', 'internal', 'tool', 'say', 'sshwarma:say', 'Send message to room'),
    ('tool_who', 'internal', 'tool', 'who', 'sshwarma:who', 'List participants');
    -- ... more tools

-- Defaults equips internal tools (no copies, just relationships)
INSERT INTO equipped (context_id, thing_id, created_at) VALUES
    ('defaults', 'tool_look', ?),
    ('defaults', 'tool_say', ?),
    ('defaults', 'tool_who', ?);
    -- ... more default equipped

-- Lobby room
INSERT INTO things (id, parent_id, kind, name, metadata) VALUES
    ('lobby', 'rooms', 'room', 'lobby', '{"vibe": "Welcome to sshwarma"}');

-- Lobby equips same tools (copied from defaults on creation)
INSERT INTO equipped (context_id, thing_id, created_at) VALUES
    ('lobby', 'tool_look', ?),
    ('lobby', 'tool_say', ?),
    ('lobby', 'tool_who', ?);
    -- ... more equipped
```

## MCP Lifecycle

### Connect

```sql
-- Upsert MCP thing
INSERT INTO things (id, parent_id, kind, name, qualified_name, available, created_at, updated_at)
VALUES ('mcp_holler', 'mcps', 'mcp', 'holler', 'holler', 1, ?, ?)
ON CONFLICT(id) DO UPDATE SET available = 1, updated_at = excluded.updated_at;

-- Upsert tool things
INSERT INTO things (id, parent_id, kind, name, qualified_name, description, available, created_at, updated_at)
VALUES ('tool_holler_sample', 'mcp_holler', 'tool', 'sample', 'holler:sample', 'Generate audio', 1, ?, ?)
ON CONFLICT(id) DO UPDATE SET description = excluded.description, available = 1, updated_at = excluded.updated_at;

-- Soft-delete tools no longer in MCP
UPDATE things
SET available = 0, deleted_at = ?, updated_at = ?
WHERE parent_id = 'mcp_holler'
  AND kind = 'tool'
  AND id NOT IN (/* current tool list */);
```

### Disconnect

```sql
-- Mark tools unavailable (not deleted)
UPDATE things
SET available = 0, updated_at = ?
WHERE parent_id = 'mcp_holler' AND kind = 'tool';
```

## Implementation Plan

### Phase A: Foundation

1. ✅ **Schema + migrations** (`src/db/schema.rs`)
   - Create `things` table
   - Create `equipped` table
   - Create `exits` table
   - Bootstrap world structure

2. ✅ **Things CRUD** (`src/db/things.rs`)
   - `insert_thing()`, `get_thing()`, `update_thing()`, `soft_delete_thing()`
   - `get_children()` - things with parent_id
   - `get_by_qualified_name()` - returns Option (unique)

3. ✅ **Equipped CRUD** (`src/db/equipped.rs`)
   - `equip()`, `unequip()`, `is_equipped()`
   - `get_equipped()` - all equipped for a context
   - `get_equipped_tools()` - just tools, with available check

4. ✅ **Exits CRUD** (`src/db/exits.rs`)
   - `create_exit()`, `delete_exit()`, `get_exits()`
   - Cycle detection

5. ✅ **Bootstrap world structure** (`src/db/things.rs`)
   - Create world container and top-level containers
   - Register internal tools as things
   - Set up defaults with equipped relationships

6. ✅ **Slash commands** (`src/commands.rs`)
   - `/inv`, `/equip`, `/unequip`, `/portal`
   - Help text updated with Inventory section

7. ✅ **Tool call rows** (`src/db/rows.rs`, `src/llm.rs`, `src/ssh/streaming.rs`)
   - Added `Row::tool_call()` and `Row::tool_result()` constructors
   - Integrated with LLM streaming (StreamChunk::ToolCall includes arguments)
   - Tool rows linked to parent model message via `parent_row_id`
   - Content_meta stores qualified name and success status

8. ✅ **Tool naming** (`src/internal_tools.rs`)
   - Bootstrap registers tools as `sshwarma:look` format
   - Qualified names stored in `things.qualified_name`

9. ✅ **Lua callbacks** (`src/lua/tools.rs`)
   - `things_get()`, `things_children()`, `things_find()`, `things_by_kind()`
   - `equipped_list()`, `equipped_tools()`, `equipped_merged_tools()`
   - `equip()`, `unequip()`, `exits_list()`, `bootstrap_world()`

10. ✅ **MCP tools** (`src/mcp_server.rs`)
    - `inventory_list` - List equipped tools in a room
    - `inventory_equip` - Equip a tool by qualified name
    - `inventory_unequip` - Unequip a tool from a room

### Phase B: wrap() Integration ✅

1. ✅ **wrap.lua equipped layer** (`src/embedded/wrap.lua`)
   - Added `format_equipped_layer()` - shows all equipped tools
   - Added `format_internal_tools_layer()` - sshwarma:* tools only
   - Added `format_external_tools_layer()` - MCP tools only
   - Added `:equipped()`, `:internal_tools()`, `:external_tools()` builder methods
   - Updated `default_wrap()` to include `:equipped()` layer

2. ✅ **Rig tool server filtering** (`src/ssh/handler.rs`)
   - Added `get_equipped_tool_names()` helper
   - MCP tools filtered by equipped status before registration
   - Qualified name matching: `holler__sample` → `holler:sample`
   - Falls back to defaults if room has no equipped tools

### Phase C: History + Stats ✅

1. ✅ **Database queries** (`src/db/rows.rs`)
   - `list_tool_calls(buffer_id, limit)` - Get tool.call/tool.result rows
   - `count_tool_calls(buffer_id)` - Aggregate counts by tool name

2. ✅ **Slash commands** (`src/commands.rs`)
   - `/history --tools [n]` - Show recent tool calls with timestamps
   - `/history --stats` - Show tool usage statistics with percentages

### Phase D: Garbage Collection

Manual tools:
- `/gc orphans` — Things with no parent (except world)
- `/gc stale` — Soft-deleted items older than N days
- `/gc purge` — Hard-delete old soft-deleted items

## Files to Modify

| File | Change |
|------|--------|
| `src/db/schema.rs` | Add things, equipped, exits tables |
| `src/db/things.rs` | **New**: Things CRUD |
| `src/db/equipped.rs` | **New**: Equipped CRUD |
| `src/db/exits.rs` | **New**: Exits CRUD |
| `src/db/rows.rs` | Tool call content_methods |
| `src/db/mod.rs` | Export new modules |
| `src/llm.rs` | StreamChunk tool call metadata |
| `src/internal_tools.rs` | Rename to sshwarma:* format |
| `src/commands.rs` | Inventory commands, /portal |
| `src/interp/commands.rs` | Wire up new commands |
| `src/lua/tools.rs` | Things and equipped Lua callbacks |
| `src/mcp_server.rs` | sshwarma:inventory_* MCP tools |
| `src/embedded/wrap.lua` | Query equipped tools |

## Example Session

```
workshop> /inv
Equipped:
  ✓ sshwarma:look
  ✓ sshwarma:say
  ✓ holler:sample     [holler, available]

Room contents:
  · prompt:code-style

workshop> /inv all
Equipped:
  [as above]

Room contents:
  · prompt:code-style

Available to equip:
  ○ holler:play       [holler]
  ○ exa:web_search    [exa]
  ○ sshwarma:history  [internal]

workshop> /equip room holler:play
Equipped holler:play

workshop> /examine holler:sample
holler:sample - Generate audio samples
Kind: tool
Location: holler (mcp)
Status: available

Recent calls:
  2m ago   qwen-8b   {prompt: "jazz drums"}     ✓ 2.1s
  15m ago  qwen-8b   {prompt: "bass groove"}    ✓ 2.4s

Stats: 42 calls, 2 errors, avg 2.3s

workshop> /portal north studio
Created exit: north → studio

workshop> /exits
Exits from workshop:
  north → studio
```

## Decisions Made

1. **Bags + Equipped** — Simpler than full DAG. Tree containment + separate equipped relationship.

2. **No /take** — No intermediate "in inventory but not equipped" state. Just equip/unequip.

3. **Soft deletes** — Everything uses deleted_at. Garbage collection later.

4. **No kind constraints** — Any thing can parent any thing. Conventions, not enforcement.

5. **No copies in defaults** — Defaults just has equipped rows pointing to internal tools. New rooms copy the equipped relationships, not the tools themselves.

6. **MUD interface** — Users see friendly commands. Bags/equipped hidden.

7. **Qualified name is UNIQUE** — No duplicates. Lookup by qualified_name returns exactly one thing.

8. **Home room for shared** — `home` is a special room containing shared resources. Rooms can equip from home.

9. **Agent+room merge** — wrap() merges agent equipped with room equipped. Room wins conflicts. Both tools and data are merged.
