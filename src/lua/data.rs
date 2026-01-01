//! Lua bindings for Buffer and Row data access
//!
//! Provides userdata types for querying and manipulating buffers and rows
//! from Lua scripts. Implements a dataframe-like API for row operations.

use crate::db::buffers::{Buffer, BufferType};
use crate::db::rows::Row;
use crate::db::Database;
use mlua::{Function, Lua, Result as LuaResult, Table, UserData, UserDataMethods, Value};
use std::sync::Arc;

/// Lua userdata wrapper for a Buffer
///
/// Provides access to buffer metadata and row queries.
#[derive(Clone)]
pub struct LuaBuffer {
    /// The underlying buffer
    pub buffer: Buffer,
    /// Database reference for queries
    pub db: Arc<Database>,
}

impl UserData for LuaBuffer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // buf.id -> string
        methods.add_meta_method(
            mlua::MetaMethod::Index,
            |lua, this, key: String| match key.as_str() {
                "id" => Ok(Value::String(lua.create_string(&this.buffer.id)?)),
                "room_id" => match &this.buffer.room_id {
                    Some(id) => Ok(Value::String(lua.create_string(id)?)),
                    None => Ok(Value::Nil),
                },
                "owner_agent_id" => match &this.buffer.owner_agent_id {
                    Some(id) => Ok(Value::String(lua.create_string(id)?)),
                    None => Ok(Value::Nil),
                },
                "buffer_type" => Ok(Value::String(
                    lua.create_string(this.buffer.buffer_type.as_str())?,
                )),
                "created_at" => Ok(Value::Number(this.buffer.created_at as f64)),
                "tombstoned" => Ok(Value::Boolean(this.buffer.tombstoned)),
                "tombstone_summary" => match &this.buffer.tombstone_summary {
                    Some(s) => Ok(Value::String(lua.create_string(s)?)),
                    None => Ok(Value::Nil),
                },
                "include_in_wrap" => Ok(Value::Boolean(this.buffer.include_in_wrap)),
                "wrap_priority" => Ok(Value::Integer(this.buffer.wrap_priority)),
                _ => Ok(Value::Nil),
            },
        );

        // buf:rows() -> LuaRowSet
        methods.add_method("rows", |_lua, this, ()| {
            let rows = this
                .db
                .list_buffer_rows(&this.buffer.id)
                .map_err(mlua::Error::external)?;

            Ok(LuaRowSet {
                rows,
                db: this.db.clone(),
            })
        });

        // buf:is_room_chat() -> bool
        methods.add_method("is_room_chat", |_lua, this, ()| {
            Ok(this.buffer.buffer_type == BufferType::RoomChat)
        });

        // buf:is_thinking() -> bool
        methods.add_method("is_thinking", |_lua, this, ()| {
            Ok(this.buffer.buffer_type == BufferType::Thinking)
        });

        // buf:is_tool_output() -> bool
        methods.add_method("is_tool_output", |_lua, this, ()| {
            Ok(this.buffer.buffer_type == BufferType::ToolOutput)
        });
    }
}

/// Lua userdata for a collection of rows with query methods
///
/// Provides dataframe-like operations for filtering and transforming rows.
#[derive(Clone)]
pub struct LuaRowSet {
    /// The rows in this set
    pub rows: Vec<Row>,
    /// Database reference for child queries
    pub db: Arc<Database>,
}

impl UserData for LuaRowSet {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Allow iteration with ipairs
        methods.add_meta_method(mlua::MetaMethod::Len, |_lua, this, ()| Ok(this.rows.len()));

        // rowset[i] -> LuaRow
        methods.add_meta_method(mlua::MetaMethod::Index, |_lua, this, key: i64| {
            let idx = (key - 1) as usize; // Lua is 1-indexed
            if idx < this.rows.len() {
                Ok(Some(LuaRow {
                    row: this.rows[idx].clone(),
                    db: this.db.clone(),
                }))
            } else {
                Ok(None)
            }
        });

        // rowset:count() -> number
        methods.add_method("count", |_lua, this, ()| Ok(this.rows.len()));

        // rowset:is_empty() -> bool
        methods.add_method("is_empty", |_lua, this, ()| Ok(this.rows.is_empty()));

        // rowset:first() -> LuaRow or nil
        methods.add_method("first", |_lua, this, ()| {
            if let Some(row) = this.rows.first() {
                Ok(Some(LuaRow {
                    row: row.clone(),
                    db: this.db.clone(),
                }))
            } else {
                Ok(None)
            }
        });

        // rowset:last_row() -> LuaRow or nil (renamed to avoid conflict with last(n))
        methods.add_method("last_row", |_lua, this, ()| {
            if let Some(row) = this.rows.last() {
                Ok(Some(LuaRow {
                    row: row.clone(),
                    db: this.db.clone(),
                }))
            } else {
                Ok(None)
            }
        });

        // rowset:last(n) -> LuaRowSet (last n rows)
        methods.add_method("last", |_lua, this, n: usize| {
            let start = this.rows.len().saturating_sub(n);
            Ok(LuaRowSet {
                rows: this.rows[start..].to_vec(),
                db: this.db.clone(),
            })
        });

        // rowset:slice(offset, limit) -> LuaRowSet
        methods.add_method("slice", |_lua, this, (offset, limit): (usize, usize)| {
            let start = offset.min(this.rows.len());
            let end = (start + limit).min(this.rows.len());
            Ok(LuaRowSet {
                rows: this.rows[start..end].to_vec(),
                db: this.db.clone(),
            })
        });

        // rowset:since(timestamp_ms) -> LuaRowSet
        methods.add_method("since", |_lua, this, timestamp: i64| {
            let filtered: Vec<Row> = this
                .rows
                .iter()
                .filter(|r| r.created_at >= timestamp)
                .cloned()
                .collect();
            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:before(timestamp_ms) -> LuaRowSet
        methods.add_method("before", |_lua, this, timestamp: i64| {
            let filtered: Vec<Row> = this
                .rows
                .iter()
                .filter(|r| r.created_at < timestamp)
                .cloned()
                .collect();
            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:where({ field = value, ... }) -> LuaRowSet
        methods.add_method("where", |_lua, this, conditions: Table| {
            let mut filtered = this.rows.clone();

            // Filter by content_method
            if let Ok(method) = conditions.get::<String>("content_method") {
                filtered.retain(|r| r.content_method == method);
            }

            // Filter by content_method prefix (glob-like)
            if let Ok(method_prefix) = conditions.get::<String>("content_method_prefix") {
                filtered.retain(|r| r.content_method.starts_with(&method_prefix));
            }

            // Filter by source_agent_id
            if let Ok(agent) = conditions.get::<String>("source_agent_id") {
                filtered.retain(|r| r.source_agent_id.as_ref() == Some(&agent));
            }
            // Alias: source
            if let Ok(agent) = conditions.get::<String>("source") {
                filtered.retain(|r| r.source_agent_id.as_ref() == Some(&agent));
            }

            // Filter by collapsed state
            if let Ok(collapsed) = conditions.get::<bool>("collapsed") {
                filtered.retain(|r| r.collapsed == collapsed);
            }

            // Filter by ephemeral state
            if let Ok(ephemeral) = conditions.get::<bool>("ephemeral") {
                filtered.retain(|r| r.ephemeral == ephemeral);
            }

            // Filter by pinned state
            if let Ok(pinned) = conditions.get::<bool>("pinned") {
                filtered.retain(|r| r.pinned == pinned);
            }

            // Filter by hidden state
            if let Ok(hidden) = conditions.get::<bool>("hidden") {
                filtered.retain(|r| r.hidden == hidden);
            }

            // Filter by finalized (not nil)
            if let Ok(finalized) = conditions.get::<bool>("finalized") {
                if finalized {
                    filtered.retain(|r| r.finalized_at.is_some());
                } else {
                    filtered.retain(|r| r.finalized_at.is_none());
                }
            }

            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:filter(function(row) -> bool) -> LuaRowSet
        methods.add_method("filter", |_lua, this, func: Function| {
            let mut filtered = Vec::new();
            for row in &this.rows {
                let lua_row = LuaRow {
                    row: row.clone(),
                    db: this.db.clone(),
                };
                let keep: bool = func.call(lua_row)?;
                if keep {
                    filtered.push(row.clone());
                }
            }
            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:map(function(row) -> value) -> table (Lua array)
        methods.add_method("map", |lua, this, func: Function| {
            let result = lua.create_table()?;
            for (i, row) in this.rows.iter().enumerate() {
                let lua_row = LuaRow {
                    row: row.clone(),
                    db: this.db.clone(),
                };
                let value: Value = func.call(lua_row)?;
                result.set(i + 1, value)?;
            }
            Ok(result)
        });

        // rowset:reduce(initial, function(acc, row) -> acc) -> value
        methods.add_method(
            "reduce",
            |_lua, this, (initial, func): (Value, Function)| {
                let mut acc = initial;
                for row in &this.rows {
                    let lua_row = LuaRow {
                        row: row.clone(),
                        db: this.db.clone(),
                    };
                    acc = func.call((acc, lua_row))?;
                }
                Ok(acc)
            },
        );

        // rowset:group_by(field) -> table { [value] = LuaRowSet }
        methods.add_method("group_by", |lua, this, field: String| {
            use std::collections::HashMap;
            let mut groups: HashMap<String, Vec<Row>> = HashMap::new();

            for row in &this.rows {
                let key = match field.as_str() {
                    "content_method" => row.content_method.clone(),
                    "source_agent_id" => row
                        .source_agent_id
                        .clone()
                        .unwrap_or_else(|| "nil".to_string()),
                    "content_format" => row.content_format.clone(),
                    _ => continue,
                };
                groups.entry(key).or_default().push(row.clone());
            }

            let result = lua.create_table()?;
            for (key, rows) in groups {
                result.set(
                    key,
                    LuaRowSet {
                        rows,
                        db: this.db.clone(),
                    },
                )?;
            }
            Ok(result)
        });

        // rowset:with_tag(tag) -> LuaRowSet (rows that have the given tag)
        methods.add_method("with_tag", |_lua, this, tag: String| {
            let mut filtered = Vec::new();
            for row in &this.rows {
                // Query tags from database
                if let Ok(tags) = this.db.get_row_tags(&row.id) {
                    if tags.contains(&tag) {
                        filtered.push(row.clone());
                    }
                }
            }
            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:without_tag(tag) -> LuaRowSet (rows without the given tag)
        methods.add_method("without_tag", |_lua, this, tag: String| {
            let mut filtered = Vec::new();
            for row in &this.rows {
                if let Ok(tags) = this.db.get_row_tags(&row.id) {
                    if !tags.contains(&tag) {
                        filtered.push(row.clone());
                    }
                }
            }
            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:messages() -> LuaRowSet (shorthand for content_method starting with "message.")
        methods.add_method("messages", |_lua, this, ()| {
            let filtered: Vec<Row> = this
                .rows
                .iter()
                .filter(|r| r.content_method.starts_with("message."))
                .cloned()
                .collect();
            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:visible() -> LuaRowSet (not hidden, not ephemeral)
        methods.add_method("visible", |_lua, this, ()| {
            let filtered: Vec<Row> = this
                .rows
                .iter()
                .filter(|r| !r.hidden && !r.ephemeral)
                .cloned()
                .collect();
            Ok(LuaRowSet {
                rows: filtered,
                db: this.db.clone(),
            })
        });

        // rowset:to_array() -> table (convert to plain Lua array of LuaRow)
        methods.add_method("to_array", |lua, this, ()| {
            let result = lua.create_table()?;
            for (i, row) in this.rows.iter().enumerate() {
                result.set(
                    i + 1,
                    LuaRow {
                        row: row.clone(),
                        db: this.db.clone(),
                    },
                )?;
            }
            Ok(result)
        });
    }
}

/// Lua userdata for a single Row
///
/// Provides field access and navigation methods.
#[derive(Clone)]
pub struct LuaRow {
    /// The underlying row
    pub row: Row,
    /// Database reference for navigation
    pub db: Arc<Database>,
}

impl UserData for LuaRow {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Field access via __index
        methods.add_meta_method(mlua::MetaMethod::Index, |lua, this, key: String| {
            match key.as_str() {
                "id" => Ok(Value::String(lua.create_string(&this.row.id)?)),
                "buffer_id" => Ok(Value::String(lua.create_string(&this.row.buffer_id)?)),
                "parent_row_id" => match &this.row.parent_row_id {
                    Some(id) => Ok(Value::String(lua.create_string(id)?)),
                    None => Ok(Value::Nil),
                },
                "position" => Ok(Value::Number(this.row.position)),
                "source_agent_id" => match &this.row.source_agent_id {
                    Some(id) => Ok(Value::String(lua.create_string(id)?)),
                    None => Ok(Value::Nil),
                },
                // Alias for convenience
                "source" => match &this.row.source_agent_id {
                    Some(id) => Ok(Value::String(lua.create_string(id)?)),
                    None => Ok(Value::Nil),
                },
                "source_session_id" => match &this.row.source_session_id {
                    Some(id) => Ok(Value::String(lua.create_string(id)?)),
                    None => Ok(Value::Nil),
                },
                "content_method" => Ok(Value::String(lua.create_string(&this.row.content_method)?)),
                "content_format" => Ok(Value::String(lua.create_string(&this.row.content_format)?)),
                "content_meta" => match &this.row.content_meta {
                    Some(meta) => {
                        // Parse JSON and convert to Lua table
                        match serde_json::from_str::<serde_json::Value>(meta) {
                            Ok(json) => json_to_lua_value(lua, &json),
                            Err(_) => Ok(Value::String(lua.create_string(meta)?)),
                        }
                    }
                    None => Ok(Value::Nil),
                },
                "content" => match &this.row.content {
                    Some(c) => Ok(Value::String(lua.create_string(c)?)),
                    None => Ok(Value::Nil),
                },
                "collapsed" => Ok(Value::Boolean(this.row.collapsed)),
                "ephemeral" => Ok(Value::Boolean(this.row.ephemeral)),
                "mutable" => Ok(Value::Boolean(this.row.mutable)),
                "pinned" => Ok(Value::Boolean(this.row.pinned)),
                "hidden" => Ok(Value::Boolean(this.row.hidden)),
                "token_count" => match this.row.token_count {
                    Some(n) => Ok(Value::Integer(n)),
                    None => Ok(Value::Nil),
                },
                "cost_usd" => match this.row.cost_usd {
                    Some(c) => Ok(Value::Number(c)),
                    None => Ok(Value::Nil),
                },
                "latency_ms" => match this.row.latency_ms {
                    Some(l) => Ok(Value::Integer(l)),
                    None => Ok(Value::Nil),
                },
                "created_at" => Ok(Value::Number(this.row.created_at as f64)),
                "updated_at" => Ok(Value::Number(this.row.updated_at as f64)),
                "finalized_at" => match this.row.finalized_at {
                    Some(t) => Ok(Value::Number(t as f64)),
                    None => Ok(Value::Nil),
                },
                _ => Ok(Value::Nil),
            }
        });

        // row:tags() -> array of tag strings
        methods.add_method("tags", |lua, this, ()| {
            let tags = this
                .db
                .get_row_tags(&this.row.id)
                .map_err(mlua::Error::external)?;

            let result = lua.create_table()?;
            for (i, tag) in tags.iter().enumerate() {
                result.set(i + 1, tag.clone())?;
            }
            Ok(result)
        });

        // row:has_tag(tag) -> bool
        methods.add_method("has_tag", |_lua, this, tag: String| {
            let tags = this
                .db
                .get_row_tags(&this.row.id)
                .map_err(mlua::Error::external)?;
            Ok(tags.contains(&tag))
        });

        // row:add_tag(tag) -> self (mutates database)
        methods.add_method("add_tag", |_lua, this, tag: String| {
            this.db
                .add_row_tag(&this.row.id, &tag)
                .map_err(mlua::Error::external)?;
            Ok(())
        });

        // row:remove_tag(tag) -> self
        methods.add_method("remove_tag", |_lua, this, tag: String| {
            this.db
                .remove_row_tag(&this.row.id, &tag)
                .map_err(mlua::Error::external)?;
            Ok(())
        });

        // row:children() -> LuaRowSet (child rows)
        methods.add_method("children", |_lua, this, ()| {
            let children = this
                .db
                .list_child_rows(&this.row.id)
                .map_err(mlua::Error::external)?;
            Ok(LuaRowSet {
                rows: children,
                db: this.db.clone(),
            })
        });

        // row:parent() -> LuaRow or nil
        methods.add_method("parent", |_lua, this, ()| {
            if let Some(ref parent_id) = this.row.parent_row_id {
                if let Ok(Some(parent)) = this.db.get_row(parent_id) {
                    return Ok(Some(LuaRow {
                        row: parent,
                        db: this.db.clone(),
                    }));
                }
            }
            Ok(None)
        });

        // row:reactions() -> array of {agent_id, reaction}
        methods.add_method("reactions", |lua, this, ()| {
            let reactions = this
                .db
                .get_row_reactions(&this.row.id)
                .map_err(mlua::Error::external)?;

            let result = lua.create_table()?;
            for (i, r) in reactions.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("id", r.id.clone())?;
                entry.set("agent_id", r.agent_id.clone())?;
                entry.set("reaction", r.reaction.clone())?;
                entry.set("created_at", r.created_at as f64)?;
                result.set(i + 1, entry)?;
            }
            Ok(result)
        });

        // row:add_reaction(agent_id, emoji) -> nil
        methods.add_method(
            "add_reaction",
            |_lua, this, (agent_id, reaction): (String, String)| {
                this.db
                    .add_row_reaction(&this.row.id, &agent_id, &reaction)
                    .map_err(mlua::Error::external)?;
                Ok(())
            },
        );

        // row:links_from() -> array of {to_row_id, link_type}
        methods.add_method("links_from", |lua, this, ()| {
            let links = this
                .db
                .get_row_links_from(&this.row.id)
                .map_err(mlua::Error::external)?;

            let result = lua.create_table()?;
            for (i, link) in links.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("to_row_id", link.to_row_id.clone())?;
                entry.set("link_type", link.link_type.as_str())?;
                result.set(i + 1, entry)?;
            }
            Ok(result)
        });

        // row:links_to() -> array of {from_row_id, link_type}
        methods.add_method("links_to", |lua, this, ()| {
            let links = this
                .db
                .get_row_links_to(&this.row.id)
                .map_err(mlua::Error::external)?;

            let result = lua.create_table()?;
            for (i, link) in links.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("from_row_id", link.from_row_id.clone())?;
                entry.set("link_type", link.link_type.as_str())?;
                result.set(i + 1, entry)?;
            }
            Ok(result)
        });

        // row:add_link(to_row_id, link_type) -> nil
        methods.add_method(
            "add_link",
            |_lua, this, (to_row_id, link_type): (String, String)| {
                use crate::db::rows::LinkType;
                let lt = LinkType::parse(&link_type).ok_or_else(|| {
                    mlua::Error::external(format!("invalid link type: {}", link_type))
                })?;
                this.db
                    .create_row_link(&this.row.id, &to_row_id, lt)
                    .map_err(mlua::Error::external)?;
                Ok(())
            },
        );

        // row:is_message() -> bool
        methods.add_method("is_message", |_lua, this, ()| {
            Ok(this.row.content_method.starts_with("message."))
        });

        // row:is_thinking() -> bool
        methods.add_method("is_thinking", |_lua, this, ()| {
            Ok(this.row.content_method.starts_with("thinking."))
        });

        // row:is_tool_call() -> bool
        methods.add_method("is_tool_call", |_lua, this, ()| {
            Ok(this.row.content_method == "tool.call")
        });

        // row:is_tool_result() -> bool
        methods.add_method("is_tool_result", |_lua, this, ()| {
            Ok(this.row.content_method == "tool.result")
        });

        // row:is_status() -> bool
        methods.add_method("is_status", |_lua, this, ()| {
            Ok(this.row.content_method.starts_with("status."))
        });

        // row:is_finalized() -> bool
        methods.add_method("is_finalized", |_lua, this, ()| {
            Ok(this.row.finalized_at.is_some())
        });
    }
}

/// Helper to convert serde_json::Value to mlua::Value
fn json_to_lua_value(lua: &Lua, json: &serde_json::Value) -> LuaResult<Value> {
    match json {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            // JSON numbers can be i64 or f64, but mlua's Integer is only i32.
            // f64 can exactly represent integers up to 2^53, which covers timestamps
            // and most practical use cases. For arbitrary JSON, f64 is the safe choice.
            if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua_value(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.clone(), json_to_lua_value(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}

/// Register data types and helper functions in Lua
///
/// Adds:
/// - `sshwarma.buffer(id)` - get buffer by ID
/// - `sshwarma.room_buffer(room_id)` - get main room chat buffer
pub fn register_data_functions(lua: &Lua, db: Arc<Database>) -> LuaResult<()> {
    // Get or create sshwarma table
    let globals = lua.globals();
    let sshwarma: Table = globals.get("sshwarma").unwrap_or_else(|_| {
        let t = lua.create_table().unwrap();
        globals.set("sshwarma", t.clone()).ok();
        t
    });

    // sshwarma.buffer(id) -> LuaBuffer or nil
    {
        let db = db.clone();
        let buffer_fn = lua.create_function(move |_lua, id: String| match db.get_buffer(&id) {
            Ok(Some(buffer)) => Ok(Some(LuaBuffer {
                buffer,
                db: db.clone(),
            })),
            Ok(None) => Ok(None),
            Err(e) => Err(mlua::Error::external(e)),
        })?;
        sshwarma.set("buffer", buffer_fn)?;
    }

    // sshwarma.room_buffer(room_id) -> LuaBuffer
    {
        let db = db.clone();
        let room_buffer_fn = lua.create_function(move |_lua, room_id: String| {
            match db.get_or_create_room_chat_buffer(&room_id) {
                Ok(buffer) => Ok(LuaBuffer {
                    buffer,
                    db: db.clone(),
                }),
                Err(e) => Err(mlua::Error::external(e)),
            }
        })?;
        sshwarma.set("room_buffer", room_buffer_fn)?;
    }

    // sshwarma.buffers_for_room(room_id) -> array of LuaBuffer
    {
        let db = db.clone();
        let buffers_fn = lua.create_function(move |lua, room_id: String| {
            match db.list_room_buffers(&room_id) {
                Ok(buffers) => {
                    let result = lua.create_table()?;
                    for (i, buffer) in buffers.into_iter().enumerate() {
                        result.set(
                            i + 1,
                            LuaBuffer {
                                buffer,
                                db: db.clone(),
                            },
                        )?;
                    }
                    Ok(result)
                }
                Err(e) => Err(mlua::Error::external(e)),
            }
        })?;
        sshwarma.set("buffers_for_room", buffers_fn)?;
    }

    // sshwarma.row(id) -> LuaRow or nil
    {
        let db = db.clone();
        let row_fn = lua.create_function(move |_lua, id: String| match db.get_row(&id) {
            Ok(Some(row)) => Ok(Some(LuaRow {
                row,
                db: db.clone(),
            })),
            Ok(None) => Ok(None),
            Err(e) => Err(mlua::Error::external(e)),
        })?;
        sshwarma.set("row", row_fn)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::agents::{Agent, AgentKind};
    use crate::db::rooms::Room;

    fn setup() -> anyhow::Result<(Arc<Database>, String, String)> {
        let db = Arc::new(Database::in_memory()?);
        let room = Room::new("testroom");
        db.insert_room(&room)?;

        let agent = Agent::new("testagent", AgentKind::Human);
        db.insert_agent(&agent)?;

        let buffer = Buffer::room_chat(&room.id);
        db.insert_buffer(&buffer)?;

        Ok((db, buffer.id, agent.id))
    }

    #[test]
    fn test_lua_buffer_userdata() -> anyhow::Result<()> {
        let (db, buffer_id, _agent_id) = setup()?;

        let lua = Lua::new();
        register_data_functions(&lua, db.clone())?;

        // Get buffer via Lua
        lua.globals().set("buffer_id", buffer_id.clone())?;
        lua.load(
            r#"
            local buf = sshwarma.buffer(buffer_id)
            assert(buf ~= nil, "buffer should exist")
            assert(buf.id == buffer_id, "buffer id should match")
            assert(buf.buffer_type == "room_chat", "should be room_chat type")
        "#,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_rowset_operations() -> anyhow::Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Insert some rows
        let mut row1 = Row::new(&buffer_id, "message.user");
        row1.source_agent_id = Some(agent_id.clone());
        let mut row2 = Row::new(&buffer_id, "message.model");
        row2.source_agent_id = Some(agent_id.clone());
        row2.content = Some("Hello from model".to_string());
        let mut row3 = Row::new(&buffer_id, "thinking.stream");
        row3.source_agent_id = Some(agent_id.clone());

        db.insert_row(&row1)?;
        db.insert_row(&row2)?;
        db.insert_row(&row3)?;

        let lua = Lua::new();
        register_data_functions(&lua, db.clone())?;

        lua.globals().set("buffer_id", buffer_id)?;
        lua.load(
            r#"
            local buf = sshwarma.buffer(buffer_id)
            local rows = buf:rows()

            -- Should have 3 rows
            assert(rows:count() == 3, "should have 3 rows, got " .. rows:count())

            -- Filter to messages only
            local messages = rows:messages()
            assert(messages:count() == 2, "should have 2 messages, got " .. messages:count())

            -- Get last row
            local last = rows:last(1)
            assert(last:count() == 1, "last(1) should return 1 row")

            -- Where clause
            local thinking = rows:where({ content_method = "thinking.stream" })
            assert(thinking:count() == 1, "should have 1 thinking row")
        "#,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_row_tags() -> anyhow::Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        let mut row = Row::new(&buffer_id, "note.user");
        row.source_agent_id = Some(agent_id.clone());
        db.insert_row(&row)?;

        let lua = Lua::new();
        register_data_functions(&lua, db.clone())?;

        lua.globals().set("row_id", row.id.clone())?;
        lua.load(
            r##"
            local row = sshwarma.row(row_id)
            assert(row ~= nil, "row should exist")

            -- Initially no tags
            local tags = row:tags()
            assert(#tags == 0, "should have no tags initially")

            -- Add a tag
            row:add_tag("#decision")
            assert(row:has_tag("#decision"), "should have #decision tag")

            -- Remove tag
            row:remove_tag("#decision")
            assert(not row:has_tag("#decision"), "should not have #decision tag after removal")
        "##,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_row_navigation() -> anyhow::Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Create parent and child rows
        let mut parent = Row::new(&buffer_id, "message.user");
        parent.source_agent_id = Some(agent_id.clone());
        db.insert_row(&parent)?;

        let mut child = Row::new(&buffer_id, "thinking.stream");
        child.source_agent_id = Some(agent_id.clone());
        child.parent_row_id = Some(parent.id.clone());
        db.insert_row(&child)?;

        let lua = Lua::new();
        register_data_functions(&lua, db.clone())?;

        lua.globals().set("parent_id", parent.id.clone())?;
        lua.globals().set("child_id", child.id.clone())?;

        lua.load(
            r#"
            local parent = sshwarma.row(parent_id)
            local child = sshwarma.row(child_id)

            -- Check parent has children
            local children = parent:children()
            assert(children:count() == 1, "parent should have 1 child")

            -- Check child has parent
            local found_parent = child:parent()
            assert(found_parent ~= nil, "child should have parent")
            assert(found_parent.id == parent_id, "parent id should match")
        "#,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_rowset_map_reduce() -> anyhow::Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Insert rows with token counts
        for i in 0..3 {
            let mut row = Row::new(&buffer_id, "message.user");
            row.source_agent_id = Some(agent_id.clone());
            row.token_count = Some((i + 1) * 100);
            db.insert_row(&row)?;
        }

        let lua = Lua::new();
        register_data_functions(&lua, db.clone())?;

        lua.globals().set("buffer_id", buffer_id)?;
        lua.load(
            r#"
            local buf = sshwarma.buffer(buffer_id)
            local rows = buf:rows()

            -- Map: extract token counts
            local counts = rows:map(function(r)
                return r.token_count or 0
            end)
            assert(#counts == 3, "should have 3 counts")

            -- Reduce: sum token counts
            local total = rows:reduce(0, function(acc, r)
                return acc + (r.token_count or 0)
            end)
            assert(total == 600, "total should be 600 (100+200+300), got " .. total)
        "#,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_rowset_filter_function() -> anyhow::Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        let mut row1 = Row::new(&buffer_id, "message.user");
        row1.source_agent_id = Some(agent_id.clone());
        let mut row2 = Row::new(&buffer_id, "message.model");
        row2.source_agent_id = Some(agent_id.clone());
        let mut row3 = Row::new(&buffer_id, "status.thinking");
        row3.source_agent_id = Some(agent_id.clone());

        db.insert_row(&row1)?;
        db.insert_row(&row2)?;
        db.insert_row(&row3)?;

        let lua = Lua::new();
        register_data_functions(&lua, db.clone())?;

        lua.globals().set("buffer_id", buffer_id)?;
        lua.load(
            r#"
            local buf = sshwarma.buffer(buffer_id)
            local rows = buf:rows()

            -- Filter with custom function
            local model_messages = rows:filter(function(r)
                return r.content_method == "message.model"
            end)
            assert(model_messages:count() == 1, "should have 1 model message")

            -- First row should be message.model
            local first = model_messages:first()
            assert(first.content_method == "message.model", "first should be message.model")
        "#,
        )
        .exec()?;

        Ok(())
    }
}
