//! Database schema definitions
//!
//! All CREATE TABLE statements for the sshwarma database.
//! Uses UUIDv7 for primary keys (time-sortable) and fractional REAL for ordering.

/// Schema version for migrations
pub const SCHEMA_VERSION: i32 = 102; // 102: Lua scripts with CoW versioning, user UI config

/// Complete schema SQL
pub const SCHEMA: &str = r#"
--------------------------------------------------------------------------------
-- AGENTS
-- Unified model for humans, models, MCP clients, and bots
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    name TEXT UNIQUE NOT NULL,              -- unique identifier
    display_name TEXT,                      -- human-readable
    agent_kind TEXT NOT NULL,               -- 'human', 'model', 'mcp_client', 'bot'
    capabilities TEXT DEFAULT '[]',         -- JSON array: ['chat', 'tool:sample', 'navigation']
    created_at INTEGER NOT NULL,            -- Unix timestamp ms

    -- Lua code storage
    hud_script TEXT,                        -- custom HUD Lua (NULL = use default)
    wrap_script TEXT,                       -- custom wrap() formatter (NULL = use default)
    context_format TEXT DEFAULT 'markdown', -- preferred context format for this agent

    -- Model backend (NULL for humans/bots)
    backend_kind TEXT,                      -- 'ollama', 'openai', 'anthropic', 'llamacpp', NULL
    backend_model_id TEXT,                  -- e.g., "qwen3:8b", "claude-3-opus"
    backend_endpoint TEXT,                  -- URL or NULL for default
    backend_config TEXT,                    -- JSON for additional config
    system_prompt TEXT                      -- default system prompt for this agent
);

CREATE INDEX IF NOT EXISTS idx_agents_kind ON agents(agent_kind);
CREATE INDEX IF NOT EXISTS idx_agents_name ON agents(name);

CREATE TABLE IF NOT EXISTS agent_sessions (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    agent_id TEXT NOT NULL,
    session_kind TEXT NOT NULL,             -- 'ssh', 'mcp', 'api', 'internal'
    connected_at INTEGER NOT NULL,          -- Unix timestamp ms
    disconnected_at INTEGER,                -- NULL if still connected
    metadata TEXT,                          -- JSON (IP, client info, etc.)
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

CREATE INDEX IF NOT EXISTS idx_sessions_agent ON agent_sessions(agent_id);
CREATE INDEX IF NOT EXISTS idx_sessions_active ON agent_sessions(disconnected_at) WHERE disconnected_at IS NULL;

CREATE TABLE IF NOT EXISTS agent_auth (
    agent_id TEXT NOT NULL,
    auth_kind TEXT NOT NULL,                -- 'pubkey', 'api_key', 'mcp_token', 'local'
    auth_data TEXT NOT NULL,                -- key fingerprint, hashed token, etc.
    created_at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, auth_kind),
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

--------------------------------------------------------------------------------
-- ROOMS
-- Simplified: just identity. Metadata lives in room_kv.
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS rooms (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    name TEXT UNIQUE NOT NULL,              -- URL-safe identifier
    created_at INTEGER NOT NULL             -- Unix timestamp ms
);

CREATE TABLE IF NOT EXISTS room_kv (
    room_id TEXT NOT NULL,
    key TEXT NOT NULL,                      -- 'vibe', 'description', 'exit.north', etc.
    value TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (room_id, key),
    FOREIGN KEY (room_id) REFERENCES rooms(id)
);

CREATE INDEX IF NOT EXISTS idx_room_kv_room ON room_kv(room_id);

--------------------------------------------------------------------------------
-- BUFFERS
-- Containers for rows. Can be room chat, thinking, tool output, scratch.
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS buffers (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    room_id TEXT,                           -- NULL for orphaned/global
    owner_agent_id TEXT,                    -- NULL for room-public
    buffer_type TEXT NOT NULL,              -- 'room_chat', 'thinking', 'tool_output', 'scratch'
    created_at INTEGER NOT NULL,

    -- Tombstoning
    tombstoned INTEGER DEFAULT 0,
    tombstone_status TEXT,                  -- 'success', 'failure', 'cancelled'
    tombstone_summary TEXT,
    tombstoned_at INTEGER,

    -- Forking
    parent_buffer_id TEXT,

    -- Wrap behavior
    include_in_wrap INTEGER DEFAULT 1,
    wrap_priority INTEGER DEFAULT 100,

    FOREIGN KEY (room_id) REFERENCES rooms(id),
    FOREIGN KEY (owner_agent_id) REFERENCES agents(id),
    FOREIGN KEY (parent_buffer_id) REFERENCES buffers(id)
);

CREATE INDEX IF NOT EXISTS idx_buffers_room ON buffers(room_id);
CREATE INDEX IF NOT EXISTS idx_buffers_type ON buffers(buffer_type);

--------------------------------------------------------------------------------
-- ROWS
-- Atomic units of content. Can nest via parent_row_id.
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS rows (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    buffer_id TEXT NOT NULL,
    parent_row_id TEXT,                     -- NULL = top-level
    position REAL NOT NULL,                 -- fractional for ordering

    -- Source
    source_agent_id TEXT,                   -- FK to agents (NULL for system)
    source_session_id TEXT,                 -- which session created this

    -- Content
    content_method TEXT NOT NULL,           -- 'message.user', 'thinking.stream', 'tool.call', etc.
    content_format TEXT DEFAULT 'text',     -- 'text', 'markdown', 'json', 'ansi'
    content_meta TEXT,                      -- JSON type-specific metadata
    content TEXT,

    -- Display state
    collapsed INTEGER DEFAULT 0,
    ephemeral INTEGER DEFAULT 0,
    mutable INTEGER DEFAULT 0,
    pinned INTEGER DEFAULT 0,
    hidden INTEGER DEFAULT 0,

    -- Metrics
    token_count INTEGER,
    cost_usd REAL,
    latency_ms INTEGER,

    -- Timestamps
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    finalized_at INTEGER,

    FOREIGN KEY (buffer_id) REFERENCES buffers(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_row_id) REFERENCES rows(id) ON DELETE CASCADE,
    FOREIGN KEY (source_agent_id) REFERENCES agents(id)
);

-- Primary access patterns
CREATE INDEX IF NOT EXISTS idx_rows_buffer_position ON rows(buffer_id, position) WHERE parent_row_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_rows_parent_position ON rows(parent_row_id, position) WHERE parent_row_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_rows_buffer_created ON rows(buffer_id, created_at);
CREATE INDEX IF NOT EXISTS idx_rows_content_method ON rows(buffer_id, content_method);
CREATE INDEX IF NOT EXISTS idx_rows_source ON rows(buffer_id, source_agent_id);
CREATE INDEX IF NOT EXISTS idx_rows_mutable ON rows(buffer_id, mutable) WHERE mutable = 1;

CREATE TABLE IF NOT EXISTS row_tags (
    row_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (row_id, tag),
    FOREIGN KEY (row_id) REFERENCES rows(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_row_tags_tag ON row_tags(tag);

CREATE TABLE IF NOT EXISTS row_reactions (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    row_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    reaction TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (row_id) REFERENCES rows(id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id) REFERENCES agents(id),
    UNIQUE (row_id, agent_id, reaction)
);

CREATE INDEX IF NOT EXISTS idx_row_reactions_row ON row_reactions(row_id);

CREATE TABLE IF NOT EXISTS row_links (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    from_row_id TEXT NOT NULL,
    to_row_id TEXT NOT NULL,
    link_type TEXT NOT NULL,                -- 'reply', 'quote', 'relates', 'continues'
    created_at INTEGER NOT NULL,
    FOREIGN KEY (from_row_id) REFERENCES rows(id) ON DELETE CASCADE,
    FOREIGN KEY (to_row_id) REFERENCES rows(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_row_links_from ON row_links(from_row_id);
CREATE INDEX IF NOT EXISTS idx_row_links_to ON row_links(to_row_id);

--------------------------------------------------------------------------------
-- VIEW STATE
-- Per-agent UI state (view stack, scroll positions)
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS view_stack (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    agent_id TEXT NOT NULL,
    region_name TEXT NOT NULL,
    layers TEXT NOT NULL DEFAULT '[]',      -- JSON array of layer objects
    active_layer INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    UNIQUE (agent_id, region_name),
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

CREATE TABLE IF NOT EXISTS buffer_scroll (
    agent_id TEXT NOT NULL,
    buffer_id TEXT NOT NULL,
    scroll_row_id TEXT,                     -- row at top of viewport
    scroll_offset INTEGER DEFAULT 0,
    mode TEXT DEFAULT 'tail',               -- 'tail' or 'pinned'
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, buffer_id),
    FOREIGN KEY (agent_id) REFERENCES agents(id),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id) ON DELETE CASCADE
);

--------------------------------------------------------------------------------
-- LUA SCRIPTS
-- User/room Lua modules with copy-on-write versioning
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS lua_scripts (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    scope TEXT NOT NULL,                    -- 'system', 'user', 'room'
    scope_id TEXT,                          -- username or room_name, NULL for system
    module_path TEXT NOT NULL,              -- 'screen', 'ui.status', etc.
    code TEXT NOT NULL,                     -- Lua source
    parent_id TEXT,                         -- previous version (CoW)
    description TEXT,
    created_at INTEGER NOT NULL,
    created_by TEXT                         -- who made this version
);

-- Primary lookup: current version of a module in a scope
CREATE INDEX IF NOT EXISTS idx_scripts_lookup
    ON lua_scripts(scope, scope_id, module_path, created_at DESC);

-- Find all versions of a script (for history/rollback)
CREATE INDEX IF NOT EXISTS idx_scripts_parent ON lua_scripts(parent_id);

--------------------------------------------------------------------------------
-- USER UI CONFIG
-- Per-user UI entrypoint configuration
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS user_ui_config (
    username TEXT PRIMARY KEY,
    entrypoint_module TEXT,                 -- NULL = use embedded default
    updated_at INTEGER NOT NULL
);

--------------------------------------------------------------------------------
-- ROOM RULES
-- Match-action system for triggering Lua scripts
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS room_rules (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    room_id TEXT NOT NULL,
    name TEXT,                              -- human-readable, optional
    enabled INTEGER DEFAULT 1,
    priority REAL DEFAULT 0.0,              -- execution order (lower = first)

    -- Trigger kind
    trigger_kind TEXT NOT NULL,             -- 'row', 'interval', 'tick'

    -- Row trigger conditions (for trigger_kind = 'row')
    match_content_method TEXT,              -- glob: 'message.*', 'tool.call'
    match_source_agent TEXT,                -- glob: 'claude', 'human:*', '*'
    match_tag TEXT,                         -- exact: '#decision'
    match_buffer_type TEXT,                 -- exact: 'room_chat', 'thinking'

    -- Time trigger conditions
    interval_ms INTEGER,                    -- for 'interval': run every N ms
    tick_divisor INTEGER,                   -- for 'tick': run every N ticks (tick = 500ms)

    -- Action
    script_id TEXT NOT NULL,
    action_slot TEXT NOT NULL,              -- 'render', 'wrap', 'notify', 'transform', 'background'

    created_at INTEGER NOT NULL,
    FOREIGN KEY (room_id) REFERENCES rooms(id),
    FOREIGN KEY (script_id) REFERENCES lua_scripts(id)
);

CREATE INDEX IF NOT EXISTS idx_room_rules_room ON room_rules(room_id, enabled, priority);
CREATE INDEX IF NOT EXISTS idx_room_rules_trigger ON room_rules(trigger_kind);

--------------------------------------------------------------------------------
-- THINGS
-- Tree of everything: rooms, agents, MCPs, tools, data, references
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS things (
    id TEXT PRIMARY KEY,                    -- UUIDv7
    parent_id TEXT REFERENCES things(id),   -- NULL = root (world)
    kind TEXT NOT NULL,                     -- 'container', 'room', 'agent', 'mcp', 'tool', 'data', 'reference'
    name TEXT NOT NULL,                     -- display name
    qualified_name TEXT,                    -- unique: 'holler:sample', 'sshwarma:look'
    description TEXT,

    -- Kind-specific content
    content TEXT,                           -- For 'data' kind: inline content
    uri TEXT,                               -- For 'reference' kind: external URI
    metadata TEXT,                          -- JSON: vibe, config, schema, etc.

    -- Status
    available INTEGER DEFAULT 1,            -- MCP connected? Tool working?

    -- Lifecycle
    created_at INTEGER NOT NULL,            -- Unix timestamp ms
    updated_at INTEGER NOT NULL,            -- Unix timestamp ms
    deleted_at INTEGER                      -- NULL = not deleted (soft delete)
);

CREATE INDEX IF NOT EXISTS idx_things_parent ON things(parent_id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_things_kind ON things(kind) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_things_qualified ON things(qualified_name)
    WHERE deleted_at IS NULL AND qualified_name IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_things_name ON things(name) WHERE deleted_at IS NULL;

--------------------------------------------------------------------------------
-- EQUIPPED
-- What's active in a context (room or agent equips tools/data)
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS equipped (
    context_id TEXT NOT NULL REFERENCES things(id),  -- room or agent thing
    thing_id TEXT NOT NULL REFERENCES things(id),    -- tool or data thing
    priority REAL DEFAULT 0.0,                       -- ordering (lower = first)
    created_at INTEGER NOT NULL,
    deleted_at INTEGER,                              -- NULL = equipped (soft delete)
    PRIMARY KEY (context_id, thing_id)
);

CREATE INDEX IF NOT EXISTS idx_equipped_context ON equipped(context_id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_equipped_thing ON equipped(thing_id) WHERE deleted_at IS NULL;

--------------------------------------------------------------------------------
-- EXITS
-- Room navigation connections
--------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS exits (
    from_thing_id TEXT NOT NULL REFERENCES things(id),  -- source room thing
    direction TEXT NOT NULL,                            -- 'north', 'studio', 'archive'
    to_thing_id TEXT NOT NULL REFERENCES things(id),    -- target room thing
    created_at INTEGER NOT NULL,
    deleted_at INTEGER,                                 -- NULL = active (soft delete)
    PRIMARY KEY (from_thing_id, direction)
);

CREATE INDEX IF NOT EXISTS idx_exits_from ON exits(from_thing_id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_exits_to ON exits(to_thing_id) WHERE deleted_at IS NULL;
"#;

/// CTE for computing row depth (nesting level)
pub const ROW_DEPTH_CTE: &str = r#"
WITH RECURSIVE row_depth AS (
    SELECT id, 0 as depth FROM rows WHERE parent_row_id IS NULL
    UNION ALL
    SELECT r.id, rd.depth + 1 FROM rows r JOIN row_depth rd ON r.parent_row_id = rd.id
)
"#;

/// Query to get current room presence (agents whose last presence row is 'join')
pub const PRESENCE_QUERY: &str = r#"
WITH latest_presence AS (
    SELECT
        source_agent_id,
        content_method,
        ROW_NUMBER() OVER (PARTITION BY source_agent_id ORDER BY created_at DESC) as rn
    FROM rows
    WHERE buffer_id = ?1
      AND content_method LIKE 'presence.%'
)
SELECT source_agent_id
FROM latest_presence
WHERE rn = 1 AND content_method = 'presence.join'
"#;
