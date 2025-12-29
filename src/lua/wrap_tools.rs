//! Wrap tools - Lua callbacks for context composition
//!
//! Provides `tools.wrap.*` functions that Lua scripts call to fetch
//! context layers for LLM prompts. Each layer returns `{content, tokens}`
//! for token budgeting.

use crate::lua::wrap::WrapState;
use crate::prompt::SystemPromptBuilder;
use mlua::{Lua, Result as LuaResult, Table};

/// Estimate tokens from text (simple heuristic: ~4 chars per token)
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Create a layer result table with content and token count
fn layer_result(lua: &Lua, content: String) -> LuaResult<Table> {
    let table = lua.create_table()?;
    let tokens = estimate_tokens(&content);
    table.set("content", content)?;
    table.set("tokens", tokens)?;
    Ok(table)
}

/// Register wrap tools in the Lua state
///
/// Creates `tools.wrap` subtable with:
/// - `tools.wrap.system_layer()` - Global sshwarma environment
/// - `tools.wrap.model_layer()` - Model identity and personality
/// - `tools.wrap.room_layer()` - Current room info
/// - `tools.wrap.participants_layer()` - Users and models present
/// - `tools.wrap.history_layer(limit)` - Recent conversation history
/// - `tools.wrap.estimate_tokens(text)` - Token estimation utility
/// - `tools.wrap.truncate(text, max_tokens)` - Truncate to token budget
///
/// Each layer function returns `{content = "...", tokens = N}`.
pub fn register_wrap_tools(lua: &Lua) -> LuaResult<()> {
    // Get or create the tools table
    let globals = lua.globals();
    let tools: Table = globals.get("tools")?;

    // Create wrap subtable
    let wrap = lua.create_table()?;

    // tools.wrap.system_layer() -> {content, tokens}
    // Returns the global sshwarma environment description
    let system_layer_fn = lua.create_function(|lua, ()| {
        let content = SystemPromptBuilder::global_layer();
        layer_result(lua, content)
    })?;
    wrap.set("system_layer", system_layer_fn)?;

    // tools.wrap.model_layer() -> {content, tokens}
    // Returns model identity from WrapState in registry
    let model_layer_fn = lua.create_function(|lua, ()| {
        let wrap_state: Option<WrapState> = lua.named_registry_value("wrap_state").ok();

        let content = match wrap_state {
            Some(state) => SystemPromptBuilder::model_layer(&state.model),
            None => "## Your Identity\nYou are an AI assistant.\n".to_string(),
        };
        layer_result(lua, content)
    })?;
    wrap.set("model_layer", model_layer_fn)?;

    // tools.wrap.room_layer() -> {content, tokens}
    // Returns current room info from WrapState
    let room_layer_fn = lua.create_function(|lua, ()| {
        let wrap_state: Option<WrapState> = lua.named_registry_value("wrap_state").ok();

        let content = match wrap_state {
            Some(state) => {
                if let Some(room_name) = &state.room_name {
                    // Try to get room from shared state
                    let world = state.shared_state.world.blocking_read();
                    if let Some(room) = world.rooms.get(room_name) {
                        SystemPromptBuilder::room_layer(room)
                    } else {
                        format!("**Room:** {}\n", room_name)
                    }
                } else {
                    "**Location:** Lobby\n".to_string()
                }
            }
            None => "**Location:** Unknown\n".to_string(),
        };
        layer_result(lua, content)
    })?;
    wrap.set("room_layer", room_layer_fn)?;

    // tools.wrap.participants_layer() -> {content, tokens}
    // Returns users and models in the current room
    let participants_layer_fn = lua.create_function(|lua, ()| {
        let wrap_state: Option<WrapState> = lua.named_registry_value("wrap_state").ok();

        let content = match wrap_state {
            Some(state) => {
                if let Some(room_name) = &state.room_name {
                    let world = state.shared_state.world.blocking_read();
                    if let Some(room) = world.rooms.get(room_name) {
                        let mut parts = Vec::new();
                        if !room.users.is_empty() {
                            parts.push(format!("**Users:** {}", room.users.join(", ")));
                        }
                        if !room.models.is_empty() {
                            let model_names: Vec<&str> = room.models.iter()
                                .map(|m| m.display_name.as_str())
                                .collect();
                            parts.push(format!("**Models:** {}", model_names.join(", ")));
                        }
                        parts.join("\n")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
            None => String::new(),
        };
        layer_result(lua, content)
    })?;
    wrap.set("participants_layer", participants_layer_fn)?;

    // tools.wrap.user_layer() -> {content, tokens}
    // Returns current user info
    let user_layer_fn = lua.create_function(|lua, ()| {
        let wrap_state: Option<WrapState> = lua.named_registry_value("wrap_state").ok();

        let content = match wrap_state {
            Some(state) => format!("## Current User\nYou are talking with **{}**.\n", state.username),
            None => "## Current User\nUnknown user.\n".to_string(),
        };
        layer_result(lua, content)
    })?;
    wrap.set("user_layer", user_layer_fn)?;

    // tools.wrap.history_layer(limit) -> {content, tokens}
    // Returns recent conversation history
    let history_layer_fn = lua.create_function(|lua, limit: Option<usize>| {
        use crate::display::{EntryContent, EntrySource};

        let limit = limit.unwrap_or(30);
        let wrap_state: Option<WrapState> = lua.named_registry_value("wrap_state").ok();

        let content = match wrap_state {
            Some(state) => {
                if let Some(room_name) = &state.room_name {
                    let world = state.shared_state.world.blocking_read();
                    if let Some(room) = world.rooms.get(room_name) {
                        let messages: Vec<String> = room
                            .ledger
                            .recent(limit)
                            .iter()
                            .filter(|e| !e.ephemeral) // Skip ephemeral entries
                            .filter_map(|entry| {
                                // Extract sender name
                                let sender_name = match &entry.source {
                                    EntrySource::User(name) => name.clone(),
                                    EntrySource::Model { name, .. } => name.clone(),
                                    EntrySource::System | EntrySource::Command { .. } => {
                                        return None // Skip system messages
                                    }
                                };

                                // Extract content text (only Chat messages)
                                let text = match &entry.content {
                                    EntryContent::Chat(text) => text.clone(),
                                    _ => return None, // Skip non-chat messages
                                };

                                Some(format!("{}: {}", sender_name, text))
                            })
                            .collect();

                        if messages.is_empty() {
                            String::new()
                        } else {
                            format!("## Recent History\n{}\n", messages.join("\n"))
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
            None => String::new(),
        };
        layer_result(lua, content)
    })?;
    wrap.set("history_layer", history_layer_fn)?;

    // tools.wrap.journal_layer(kind, limit) -> {content, tokens}
    // Returns recent journal entries (optionally filtered by kind)
    let journal_layer_fn = lua.create_function(|lua, (kind, limit): (Option<String>, Option<usize>)| {
        use crate::world::JournalKind;

        let limit = limit.unwrap_or(5);
        let wrap_state: Option<WrapState> = lua.named_registry_value("wrap_state").ok();

        // Parse kind filter if provided
        let kind_filter = kind.as_ref().and_then(|k| JournalKind::from_str(k));

        let content = match wrap_state {
            Some(state) => {
                if let Some(room_name) = &state.room_name {
                    let world = state.shared_state.world.blocking_read();
                    if let Some(room) = world.rooms.get(room_name) {
                        let entries: Vec<String> = room.context.journal
                            .iter()
                            .filter(|e| kind_filter.map_or(true, |k| e.kind == k))
                            .rev()
                            .take(limit)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .map(|e| format!("[{}] {}", e.kind, e.content))
                            .collect();

                        if entries.is_empty() {
                            String::new()
                        } else {
                            format!("## Journal\n{}\n", entries.join("\n"))
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
            None => String::new(),
        };
        layer_result(lua, content)
    })?;
    wrap.set("journal_layer", journal_layer_fn)?;

    // tools.wrap.inspirations_layer() -> {content, tokens}
    // Returns room inspiration board
    let inspirations_layer_fn = lua.create_function(|lua, ()| {
        let wrap_state: Option<WrapState> = lua.named_registry_value("wrap_state").ok();

        let content = match wrap_state {
            Some(state) => {
                if let Some(room_name) = &state.room_name {
                    let world = state.shared_state.world.blocking_read();
                    if let Some(room) = world.rooms.get(room_name) {
                        if room.context.inspirations.is_empty() {
                            String::new()
                        } else {
                            let items: Vec<String> = room.context.inspirations
                                .iter()
                                .map(|i| format!("- {}", i.content))
                                .collect();
                            format!("## Inspirations\n{}\n", items.join("\n"))
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
            None => String::new(),
        };
        layer_result(lua, content)
    })?;
    wrap.set("inspirations_layer", inspirations_layer_fn)?;

    // tools.wrap.estimate_tokens(text) -> int
    let estimate_fn = lua.create_function(|_, text: String| {
        Ok(estimate_tokens(&text))
    })?;
    wrap.set("estimate_tokens", estimate_fn)?;

    // tools.wrap.truncate(text, max_tokens) -> text
    // Truncates text to approximately max_tokens
    let truncate_fn = lua.create_function(|_, (text, max_tokens): (String, usize)| {
        let max_chars = max_tokens * 4; // Reverse of estimate
        if text.len() <= max_chars {
            Ok(text)
        } else {
            // Find word boundary near truncation point
            let truncated = &text[..max_chars];
            if let Some(last_space) = truncated.rfind(' ') {
                Ok(format!("{}...", &truncated[..last_space]))
            } else {
                Ok(format!("{}...", truncated))
            }
        }
    })?;
    wrap.set("truncate", truncate_fn)?;

    // Register the wrap table under tools
    tools.set("wrap", wrap)?;

    Ok(())
}

/// Set the WrapState in Lua registry for layer callbacks to access
pub fn set_wrap_state(lua: &Lua, state: WrapState) -> LuaResult<()> {
    lua.set_named_registry_value("wrap_state", WrapStateWrapper(state))
}

/// Clear the WrapState from Lua registry
pub fn clear_wrap_state(lua: &Lua) -> LuaResult<()> {
    lua.unset_named_registry_value("wrap_state")
}

/// Wrapper to make WrapState storable in Lua registry
struct WrapStateWrapper(WrapState);

impl mlua::UserData for WrapStateWrapper {}

impl mlua::FromLua for WrapState {
    fn from_lua(value: mlua::Value, _lua: &Lua) -> LuaResult<Self> {
        match value {
            mlua::Value::UserData(ud) => {
                let wrapper = ud.borrow::<WrapStateWrapper>()?;
                Ok(wrapper.0.clone())
            }
            _ => Err(mlua::Error::FromLuaConversionError {
                from: value.type_name(),
                to: "WrapState".to_string(),
                message: Some("expected WrapState userdata".to_string()),
            }),
        }
    }
}
