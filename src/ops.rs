//! Core operations shared between slash commands and internal tools
//!
//! Pure async functions that take state + args and return Result<T>.
//! No formatting - callers decide how to present results.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rig::tool::server::ToolServer;
use serde::Serialize;
use tokio::sync::{mpsc, Mutex};

use crate::db::rows::Row;
use crate::internal_tools::{InternalToolConfig, ToolContext};
use crate::llm::StreamChunk;
use crate::lua::{LuaRuntime, WrapState};
use crate::model::ModelHandle;
use crate::ssh::RowUpdate;
use crate::state::SharedState;

// =============================================================================
// MentionSession trait - shared abstraction for @mention handling
// =============================================================================

/// Result of initiating a @mention
#[derive(Debug, Clone)]
pub struct MentionResult {
    /// Row ID of the user's message
    pub message_row_id: String,
    /// Row ID of the placeholder for model response
    pub response_row_id: String,
}

/// Trait for session context needed by @mention operations
///
/// This trait abstracts over SSH sessions and MCP sessions, allowing
/// the same @mention handling logic to work for both.
pub trait MentionSession: Send + Sync {
    /// Get the session's agent ID (UUID from agents table)
    fn agent_id(&self) -> &str;

    /// Get the display name/username for this session
    fn username(&self) -> &str;

    /// Get the current room name (None if not in a room)
    fn current_room(&self) -> Option<String>;
}

/// Handle an @mention - create rows in buffer
///
/// Creates user message row and placeholder for model response.
/// Does NOT spawn the model task - caller is responsible for that.
///
/// # Arguments
/// * `state` - Shared application state
/// * `session` - Session context implementing MentionSession
/// * `model_name` - Short name of the model to mention (e.g., "qwen-8b")
/// * `message` - The message content to send to the model
///
/// # Returns
/// A tuple of (MentionResult, ModelHandle) on success
pub async fn handle_mention_create_rows(
    state: &SharedState,
    session: &dyn MentionSession,
    model_name: &str,
    message: &str,
) -> Result<(MentionResult, ModelHandle)> {
    let room_name = session
        .current_room()
        .ok_or_else(|| anyhow!("Not in a room"))?;

    if message.trim().is_empty() {
        return Err(anyhow!("Message cannot be empty"));
    }

    // Look up model
    let model = state
        .models
        .get(model_name)
        .ok_or_else(|| {
            let available: Vec<_> = state
                .models
                .available()
                .iter()
                .map(|m| m.short_name.as_str())
                .collect();
            anyhow!(
                "Unknown model '{}'. Available: {}",
                model_name,
                available.join(", ")
            )
        })?
        .clone();

    // Add user's message to buffer
    let buffer = state.db.get_or_create_room_buffer(&room_name)?;
    let agent = state.db.get_or_create_human_agent(session.username())?;
    let mut user_row = Row::message(
        &buffer.id,
        &agent.id,
        format!("@{}: {}", model_name, message),
        false,
    );
    state.db.append_row(&mut user_row)?;

    // Create placeholder for model response
    let model_agent = state.db.get_or_create_model_agent(&model.short_name)?;
    let mut thinking_row = Row::thinking(&buffer.id, &model_agent.id);
    state.db.append_row(&mut thinking_row)?;

    Ok((
        MentionResult {
            message_row_id: user_row.id,
            response_row_id: thinking_row.id,
        },
        model,
    ))
}

// =============================================================================
// Model Response Spawning - shared between SSH and MCP
// =============================================================================

/// Configuration for spawning a model response
#[derive(Clone)]
pub struct ModelResponseConfig {
    /// The model to use for generation
    pub model: ModelHandle,
    /// The user's message to the model
    pub message: String,
    /// The username of who sent the message
    pub username: String,
    /// The room name (for context and tool filtering)
    pub room_name: Option<String>,
    /// Row ID of the placeholder thinking row
    pub placeholder_row_id: Option<String>,
}

/// Get set of equipped tool qualified names for a room
///
/// Returns empty set if room has no equipped tools or things system not initialized.
/// This is used to filter which MCP/internal tools are available during @mention.
pub fn get_equipped_tool_names(state: &SharedState, room_name: &str) -> HashSet<String> {
    // Look up room by name to get its UUID
    let room_id = match state.db.get_room_by_name(room_name) {
        Ok(Some(room)) => room.id,
        _ => return HashSet::new(),
    };

    // Get equipped tools for this room
    let equipped = state
        .db
        .get_room_equipment_tools(&room_id)
        .unwrap_or_default();

    // Build set of qualified names
    equipped
        .into_iter()
        .filter_map(|eq| eq.thing.qualified_name)
        .collect()
}

/// Spawn a model response task
///
/// Creates the streaming task that handles tool calls and updates.
/// Returns a JoinHandle that can be awaited or detached.
///
/// The `update_tx` channel receives RowUpdate messages as the model responds.
/// If None, updates are discarded (useful for fire-and-forget).
///
/// # Arguments
/// * `state` - Shared application state
/// * `config` - Configuration including model, message, username, room
/// * `lua_runtime` - Optional Lua runtime for context composition (if None, uses basic prompt)
/// * `update_tx` - Optional channel for receiving streaming updates
pub async fn spawn_model_response(
    state: Arc<SharedState>,
    config: ModelResponseConfig,
    lua_runtime: Option<Arc<Mutex<LuaRuntime>>>,
    update_tx: Option<mpsc::Sender<RowUpdate>>,
) -> Result<tokio::task::JoinHandle<()>> {
    let llm = state.llm.clone();

    // Get MCP tools for rig agent
    let mcp_context = state.mcp.rig_tools().await;

    // Get equipped tools for this room to filter available tools
    let room_for_tools = config
        .room_name
        .clone()
        .unwrap_or_else(|| "lobby".to_string());
    let equipped_tools = get_equipped_tool_names(&state, &room_for_tools);

    // Build ToolServer with MCP + internal tools (filtered by equipped)
    let tool_server_handle = {
        let mut server = ToolServer::new();

        // Add MCP tools if available (only if equipped to room)
        if let Some(ref ctx) = mcp_context {
            for (tool, peer) in ctx.tools.iter() {
                // Convert MCP tool name to qualified format: server__tool -> server:tool
                let qualified = tool.name.replace("__", ":");

                // Only include tools that are equipped to this room
                if equipped_tools.contains(&qualified) {
                    server = server.rmcp_tool(tool.clone(), peer.clone());
                } else {
                    tracing::debug!("skipping MCP tool {} (not equipped)", qualified);
                }
            }
        }

        server.run()
    };

    // Register internal sshwarma tools (filtered by equipment status)
    let in_room = config.room_name.is_some();

    if let Some(ref lua_rt) = lua_runtime {
        let tool_ctx = ToolContext {
            state: state.clone(),
            room: room_for_tools.clone(),
            username: config.username.clone(),
            lua_runtime: lua_rt.clone(),
        };
        let internal_config = InternalToolConfig::for_room(&state, &room_for_tools).await;
        match crate::internal_tools::register_tools(
            &tool_server_handle,
            tool_ctx,
            &internal_config,
            in_room,
            &equipped_tools,
        )
        .await
        {
            Ok(count) => tracing::info!("registered {} internal tools for @mention", count),
            Err(e) => tracing::error!("failed to register internal tools: {}", e),
        }
    }

    // Build tool guide for system prompt
    let tool_guide = match tool_server_handle.get_tool_defs(None).await {
        Ok(tool_defs) if !tool_defs.is_empty() => {
            let mut guide = String::from("\n\n## Your Functions\n");
            guide.push_str("You have these built-in functions:\n\n");
            for tool in &tool_defs {
                let display_name = tool.name.strip_prefix("sshwarma_").unwrap_or(&tool.name);
                guide.push_str(&format!("- **{}**: {}\n", display_name, tool.description));
            }
            tracing::info!("injecting {} tool definitions into prompt", tool_defs.len());
            guide
        }
        Ok(_) => {
            tracing::warn!("no tools available for @mention");
            String::new()
        }
        Err(e) => {
            tracing::error!("failed to get tool definitions: {}", e);
            String::new()
        }
    };

    // Build context via wrap() system
    let model = config.model.clone();
    let target_tokens = model.context_window.unwrap_or(8000);
    let (system_prompt, full_message) = if let Some(ref lua_rt) = lua_runtime {
        let wrap_state = WrapState {
            room_name: config.room_name.clone(),
            username: config.username.clone(),
            model: model.clone(),
            shared_state: state.clone(),
        };

        let lua = lua_rt.lock().await;
        match lua.wrap(wrap_state, target_tokens) {
            Ok(result) => {
                // Log token counts before moving values
                let system_tokens = result.system_prompt.len() / 4;
                let context_tokens = result.context.len() / 4;

                // Combine wrap system_prompt with tool guide
                let prompt = if tool_guide.is_empty() {
                    result.system_prompt
                } else {
                    format!("{}{}", result.system_prompt, tool_guide)
                };

                // Prepend context to user message
                let msg = if result.context.is_empty() {
                    config.message.clone()
                } else {
                    format!("{}\n\n---\n\n{}", result.context, config.message)
                };

                tracing::info!(
                    "wrap() composed {} system tokens, {} context tokens",
                    system_tokens,
                    context_tokens
                );

                (prompt, msg)
            }
            Err(e) => {
                // Fail visibly - notify user and abort
                tracing::error!("wrap() failed: {}", e);
                lua.tool_state().push_notification_with_level(
                    format!("Context composition failed: {}", e),
                    5000,
                    crate::lua::NotificationLevel::Error,
                );
                return Err(anyhow!("wrap() failed: {}", e));
            }
        }
    } else {
        // No Lua runtime - use basic fallback
        let prompt = format!(
            "You are {} in a collaborative chat room. Be helpful and concise.{}",
            model.display_name, tool_guide
        );
        (prompt, config.message.clone())
    };

    let model_short = model.short_name.clone();
    let row_id = config.placeholder_row_id.clone();
    let room_for_tracking = config.room_name.clone();

    let handle = tokio::spawn(async move {
        tracing::info!("spawn_model_response: background task started");

        // Get buffer_id and agent_id for tool call tracking
        let (buffer_id, agent_id) = if let Some(ref room) = room_for_tracking {
            let buf_id = state
                .db
                .get_or_create_room_buffer(room)
                .map(|b| b.id)
                .unwrap_or_default();
            let agt_id = state
                .db
                .get_or_create_model_agent(&model_short)
                .map(|a| a.id)
                .unwrap_or_default();
            (buf_id, agt_id)
        } else {
            (String::new(), String::new())
        };

        // Track last tool name for result matching
        let mut last_tool_name = String::new();

        // Create channel for streaming chunks
        let (chunk_tx, mut chunk_rx) = mpsc::channel::<StreamChunk>(32);

        // Spawn the streaming LLM call
        tracing::info!("spawn_model_response: starting LLM stream");
        let stream_handle = tokio::spawn({
            let llm = llm.clone();
            let model = model.clone();
            let system_prompt = system_prompt.clone();
            let full_message = full_message.clone();
            async move {
                tracing::info!("spawn_model_response: calling stream_with_tool_server");
                let result = llm
                    .stream_with_tool_server(
                        &model,
                        &system_prompt,
                        &full_message,
                        tool_server_handle,
                        chunk_tx,
                        100, // max tool turns
                    )
                    .await;
                tracing::info!(
                    "spawn_model_response: stream_with_tool_server returned: {:?}",
                    result.is_ok()
                );
                result
            }
        });

        // Process streaming chunks
        let mut _full_response = String::new();
        tracing::info!("spawn_model_response: waiting for chunks");

        while let Some(chunk) = chunk_rx.recv().await {
            tracing::info!(
                "spawn_model_response: received chunk: {:?}",
                std::mem::discriminant(&chunk)
            );
            match chunk {
                StreamChunk::Text(text) => {
                    tracing::info!("spawn_model_response: text chunk len={}", text.len());
                    _full_response.push_str(&text);
                    if let Some(ref row_id) = row_id {
                        if let Some(ref tx) = update_tx {
                            let _ = tx
                                .send(RowUpdate::Chunk {
                                    row_id: row_id.clone(),
                                    text,
                                })
                                .await;
                        }
                    }
                }
                StreamChunk::ToolCall { name, arguments } => {
                    last_tool_name = name.clone();
                    if let Some(ref row_id) = row_id {
                        if let Some(ref tx) = update_tx {
                            let _ = tx
                                .send(RowUpdate::ToolCall {
                                    row_id: row_id.clone(),
                                    tool_name: name,
                                    tool_args: arguments,
                                    model_name: model_short.clone(),
                                    buffer_id: buffer_id.clone(),
                                    agent_id: agent_id.clone(),
                                })
                                .await;
                        }
                    }
                }
                StreamChunk::ToolResult(summary) => {
                    if let Some(ref row_id) = row_id {
                        if let Some(ref tx) = update_tx {
                            let _ = tx
                                .send(RowUpdate::ToolResult {
                                    row_id: row_id.clone(),
                                    tool_name: last_tool_name.clone(),
                                    summary,
                                    success: true, // rig tool calls that reach here succeeded
                                    buffer_id: buffer_id.clone(),
                                })
                                .await;
                        }
                    }
                }
                StreamChunk::Done => {
                    break;
                }
                StreamChunk::Error(e) => {
                    tracing::error!("stream error: {}", e);
                    break;
                }
            }
        }

        // Wait for stream task to complete
        let _ = stream_handle.await;

        // Send completion to finalize the row
        if let Some(row_id) = row_id {
            if let Some(ref tx) = update_tx {
                let _ = tx
                    .send(RowUpdate::Complete {
                        row_id,
                        model_name: model_short,
                    })
                    .await;
            }
        }
    });

    Ok(handle)
}

/// Room summary for /look
#[derive(Debug, Clone, Serialize)]
pub struct RoomSummary {
    pub name: String,
    pub description: Option<String>,
    pub users: Vec<String>,
    pub models: Vec<String>,
    pub artifact_count: usize,
    pub vibe: Option<String>,
    pub exits: HashMap<String, String>,
}

/// Room list entry for /rooms
#[derive(Debug, Clone, Serialize)]
pub struct RoomInfo {
    pub name: String,
    pub user_count: usize,
}

/// Get room summary
pub async fn look(state: &SharedState, room_name: &str) -> Result<RoomSummary> {
    let world = state.world.read().await;
    let room = world
        .get_room(room_name)
        .ok_or_else(|| anyhow!("Room '{}' not found", room_name))?;

    let vibe = state.db.get_vibe(room_name).ok().flatten();
    let exits = state.db.get_exits(room_name).unwrap_or_default();

    Ok(RoomSummary {
        name: room.name.clone(),
        description: room.description.clone(),
        users: room.users.clone(),
        models: room.models.iter().map(|m| m.short_name.clone()).collect(),
        artifact_count: room.artifacts.len(),
        vibe,
        exits,
    })
}

/// Get users in room
pub async fn who(state: &SharedState, room_name: &str) -> Result<Vec<String>> {
    let world = state.world.read().await;
    let room = world
        .get_room(room_name)
        .ok_or_else(|| anyhow!("Room '{}' not found", room_name))?;

    Ok(room.users.clone())
}

/// List all rooms
pub async fn rooms(state: &SharedState) -> Result<Vec<RoomInfo>> {
    let world = state.world.read().await;
    let room_list = world.list_rooms();

    Ok(room_list
        .into_iter()
        .map(|r| RoomInfo {
            name: r.name,
            user_count: r.user_count,
        })
        .collect())
}

/// Get room history
pub async fn history(
    state: &SharedState,
    room_name: &str,
    limit: usize,
) -> Result<Vec<HistoryEntry>> {
    let messages = state.db.recent_messages(room_name, limit)?;

    Ok(messages
        .into_iter()
        .map(|m| HistoryEntry {
            timestamp: m.timestamp[11..16].to_string(), // HH:MM
            sender: m.sender_name,
            content: m.content,
        })
        .collect())
}

/// History entry
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub sender: String,
    pub content: String,
}

/// Get room exits
pub async fn exits(state: &SharedState, room_name: &str) -> Result<HashMap<String, String>> {
    state.db.get_exits(room_name)
}

/// List available MCP tools
pub async fn tools(state: &SharedState) -> Result<Vec<ToolInfo>> {
    let tool_list = state.mcp.list_tools().await;

    Ok(tool_list
        .into_iter()
        .map(|t| ToolInfo {
            name: t.name,
            source: t.source,
            description: t.description,
        })
        .collect())
}

/// Tool info
#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub source: String,
    pub description: String,
}

/// Get vibe for room
pub async fn get_vibe(state: &SharedState, room_name: &str) -> Result<Option<String>> {
    state.db.get_vibe(room_name)
}

/// Set vibe for room
pub async fn set_vibe(state: &SharedState, room_name: &str, vibe: &str) -> Result<()> {
    state.db.set_vibe(room_name, Some(vibe))?;

    // Update in-memory state
    let mut world = state.world.write().await;
    if let Some(room) = world.get_room_mut(room_name) {
        room.context.vibe = Some(vibe.to_string());
    }

    Ok(())
}

/// Get navigation enabled for room (defaults to true)
pub async fn get_room_navigation(state: &SharedState, room_name: &str) -> Result<bool> {
    state.db.get_room_navigation(room_name)
}

/// Set navigation enabled for room
pub async fn set_room_navigation(
    state: &SharedState,
    room_name: &str,
    enabled: bool,
) -> Result<()> {
    state.db.set_room_navigation(room_name, enabled)?;
    Ok(())
}

/// Say something to the room
pub async fn say(state: &SharedState, room_name: &str, sender: &str, message: &str) -> Result<()> {
    use crate::db::rows::Row;

    // Get or create the room's buffer
    let buffer = state.db.get_or_create_room_buffer(room_name)?;

    // Get agent ID for sender (create if needed)
    let agent_id = state.db.get_or_create_human_agent(sender)?.id;

    // Create and add the row
    let mut row = Row::message(&buffer.id, &agent_id, message, false);
    state.db.append_row(&mut row)?;

    Ok(())
}

// Navigation operations

/// Join a room
pub async fn join(
    state: &SharedState,
    username: &str,
    current_room: Option<&str>,
    target_room: &str,
) -> Result<RoomSummary> {
    tracing::info!("ops::join: entering, target={}", target_room);

    // Leave current room if in one
    if let Some(current) = current_room {
        tracing::info!("ops::join: leaving current room {}", current);
        let mut world = state.world.write().await;
        tracing::info!("ops::join: got write lock for leave");
        if let Some(room) = world.get_room_mut(current) {
            room.remove_user(username);
        }
    }

    // Check target exists
    tracing::info!("ops::join: checking target exists");
    {
        tracing::info!("ops::join: acquiring read lock");
        let world = state.world.read().await;
        tracing::info!("ops::join: got read lock");
        if world.get_room(target_room).is_none() {
            return Err(anyhow!(
                "No room named '{}'. Use /create {} to make one.",
                target_room,
                target_room
            ));
        }
    }
    tracing::info!("ops::join: target exists");

    // Ensure room buffer exists in database
    tracing::info!("ops::join: getting room buffer");
    let buffer = state.db.get_or_create_room_buffer(target_room)?;
    tracing::info!("ops::join: got room buffer");

    // Join target room
    tracing::info!("ops::join: acquiring write lock for join");
    {
        let mut world = state.world.write().await;
        tracing::info!("ops::join: got write lock for join");
        if let Some(room) = world.get_room_mut(target_room) {
            room.add_user(username.to_string());
            // Set buffer ID if not already set
            if room.buffer_id.is_none() {
                room.set_buffer_id(buffer.id.clone());
            }
        }
    }

    // Return room info
    look(state, target_room).await
}

/// Leave room (return to lobby)
pub async fn leave(state: &SharedState, username: &str, room_name: &str) -> Result<()> {
    let mut world = state.world.write().await;
    if let Some(room) = world.get_room_mut(room_name) {
        room.remove_user(username);
    }
    Ok(())
}

/// Create a new room
pub async fn create_room(
    state: &SharedState,
    username: &str,
    room_name: &str,
    current_room: Option<&str>,
) -> Result<RoomSummary> {
    // Validate name
    if !room_name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!(
            "Room name can only contain letters, numbers, dashes, and underscores."
        ));
    }

    // Check if exists
    {
        let world = state.world.read().await;
        if world.get_room(room_name).is_some() {
            return Err(anyhow!(
                "Room '{}' already exists. Use /join {} to enter.",
                room_name,
                room_name
            ));
        }
    }

    // Leave current room
    if let Some(current) = current_room {
        let mut world = state.world.write().await;
        if let Some(room) = world.get_room_mut(current) {
            room.remove_user(username);
        }
    }

    // Create room
    {
        let mut world = state.world.write().await;
        world.create_room(room_name.to_string());
        if let Some(room) = world.get_room_mut(room_name) {
            room.add_user(username.to_string());
        }
    }

    state.db.create_room(room_name, None)?;

    look(state, room_name).await
}

/// Fork a room (copy context)
pub async fn fork_room(
    state: &SharedState,
    username: &str,
    source_room: &str,
    new_room: &str,
) -> Result<RoomSummary> {
    // Validate name
    if !new_room
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!(
            "Room name can only contain letters, numbers, dashes, and underscores."
        ));
    }

    // Check target doesn't exist
    {
        let world = state.world.read().await;
        if world.get_room(new_room).is_some() {
            return Err(anyhow!("Room '{}' already exists.", new_room));
        }
    }

    // Fork in database
    state.db.fork_room(source_room, new_room)?;

    // Create in memory and join
    {
        let mut world = state.world.write().await;
        world.create_room(new_room.to_string());

        // Leave source room
        if let Some(room) = world.get_room_mut(source_room) {
            room.remove_user(username);
        }

        // Join new room
        if let Some(room) = world.get_room_mut(new_room) {
            room.add_user(username.to_string());
        }
    }

    look(state, new_room).await
}

/// Navigate via exit
pub async fn go(
    state: &SharedState,
    username: &str,
    current_room: &str,
    direction: &str,
) -> Result<RoomSummary> {
    let exits = state.db.get_exits(current_room)?;

    match exits.get(direction) {
        Some(target) => join(state, username, Some(current_room), target).await,
        None => {
            if exits.is_empty() {
                Err(anyhow!("No exits from this room."))
            } else {
                let available: Vec<_> = exits.keys().collect();
                Err(anyhow!(
                    "No exit '{}'. Available: {}",
                    direction,
                    available
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }
}

/// Dig an exit
pub async fn dig(
    state: &SharedState,
    from_room: &str,
    direction: &str,
    to_room: &str,
) -> Result<String> {
    // Create exit
    state.db.add_exit(from_room, direction, to_room)?;

    // Create reverse exit
    let reverse = match direction {
        "north" => "south",
        "south" => "north",
        "east" => "west",
        "west" => "east",
        "up" => "down",
        "down" => "up",
        "in" => "out",
        "out" => "in",
        _ => "back",
    };

    state.db.add_exit(to_room, reverse, from_room)?;

    Ok(reverse.to_string())
}
