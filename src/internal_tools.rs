//! Internal sshwarma tools for LLM agents
//!
//! These tools give models the same capabilities as humans have via slash commands.
//! They're always available, ensuring models always have tools to call.

use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::{ToolDyn, ToolError};
use rig::wasm_compat::WasmBoxedFuture;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;
use tracing::debug;

use crate::lua::LuaRuntime;
use crate::ops;
use crate::state::SharedState;

/// Convert anyhow::Error to ToolError
fn anyhow_to_tool_error(e: anyhow::Error) -> ToolError {
    ToolError::ToolCallError(Box::new(std::io::Error::other(e.to_string())))
}

/// Configuration for internal tools
#[derive(Debug, Clone)]
pub struct InternalToolConfig {
    /// Enable navigation tools (join, leave, go, create, fork)
    pub enable_navigation: bool,
}

impl InternalToolConfig {
    /// Create config for a specific room, reading per-room settings from database
    pub async fn for_room(state: &SharedState, room: &str) -> Self {
        let enable_navigation = state.db.get_room_navigation(room).unwrap_or(true);
        Self { enable_navigation }
    }
}

impl Default for InternalToolConfig {
    fn default() -> Self {
        Self {
            enable_navigation: true,
        }
    }
}

/// Context needed by internal tools
#[derive(Clone)]
pub struct ToolContext {
    pub state: Arc<SharedState>,
    pub room: String,
    pub username: String,
    pub lua_runtime: Arc<Mutex<LuaRuntime>>,
}

/// Register all internal sshwarma tools with a ToolServerHandle
///
/// Note: Must use add_tool (not append_toolset) because rig's append_toolset
/// doesn't add tools to static_tool_names, making them invisible to get_tool_defs.
pub async fn register_tools(
    handle: &rig::tool::server::ToolServerHandle,
    ctx: ToolContext,
    config: &InternalToolConfig,
    include_write_tools: bool,
) -> anyhow::Result<usize> {
    let mut count = 0;

    // Read-only tools (always available)
    handle.add_tool(SshwarmaLook { ctx: ctx.clone() }).await?;
    handle.add_tool(SshwarmaWho { ctx: ctx.clone() }).await?;
    handle.add_tool(SshwarmaRooms { ctx: ctx.clone() }).await?;
    handle.add_tool(SshwarmaHistory { ctx: ctx.clone() }).await?;
    handle.add_tool(SshwarmaExits { ctx: ctx.clone() }).await?;
    handle.add_tool(SshwarmaJournal { ctx: ctx.clone() }).await?;
    handle.add_tool(SshwarmaTools { ctx: ctx.clone() }).await?;
    count += 7;

    // Write tools (only when in a room)
    if include_write_tools {
        handle.add_tool(SshwarmaSay { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaVibe { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaNote { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaDecide { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaIdea { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaMilestone { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaInspire { ctx: ctx.clone() }).await?;
        count += 7;

        // Navigation tools (toggleable per-room)
        if config.enable_navigation {
            handle.add_tool(SshwarmaJoin { ctx: ctx.clone() }).await?;
            handle.add_tool(SshwarmaLeave { ctx: ctx.clone() }).await?;
            handle.add_tool(SshwarmaGo { ctx: ctx.clone() }).await?;
            handle.add_tool(SshwarmaCreate { ctx: ctx.clone() }).await?;
            handle.add_tool(SshwarmaFork { ctx: ctx.clone() }).await?;
            count += 5;
        }
    }

    Ok(count)
}

// ============================================================================
// Read-only Tools
// ============================================================================

/// Get current room info
#[derive(Clone)]
struct SshwarmaLook {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaLook {
    fn name(&self) -> String {
        "sshwarma_look".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_look".to_string(),
                description: "Get current room info: name, description, users, models, artifacts, vibe, exits".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            }
        })
    }

    fn call(&self, _args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            use crate::lua::WrapState;

            let wrap_state = WrapState {
                room_name: Some(self.ctx.room.clone()),
                username: self.ctx.username.clone(),
                model: crate::model::ModelHandle::default(),
                shared_state: self.ctx.state.clone(),
            };

            let lua = self.ctx.lua_runtime.lock().await;
            lua.render_look_markdown(wrap_state)
                .map_err(anyhow_to_tool_error)
        })
    }
}

/// Get users in room
#[derive(Clone)]
struct SshwarmaWho {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaWho {
    fn name(&self) -> String {
        "sshwarma_who".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_who".to_string(),
                description: "Get list of users in the current room".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            }
        })
    }

    fn call(&self, _args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let users = ops::who(&self.ctx.state, &self.ctx.room)
                .await
                .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&users).map_err(ToolError::JsonError)
        })
    }
}

/// List all rooms
#[derive(Clone)]
struct SshwarmaRooms {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaRooms {
    fn name(&self) -> String {
        "sshwarma_rooms".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_rooms".to_string(),
                description: "List all available rooms with user counts".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            }
        })
    }

    fn call(&self, _args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let rooms = ops::rooms(&self.ctx.state)
                .await
                .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&rooms).map_err(ToolError::JsonError)
        })
    }
}

/// Get room history
#[derive(Clone)]
struct SshwarmaHistory {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct HistoryArgs {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

impl ToolDyn for SshwarmaHistory {
    fn name(&self) -> String {
        "sshwarma_history".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_history".to_string(),
                description: "Get recent messages from the room".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Number of messages to retrieve (default: 20, max: 100)"
                        }
                    },
                    "required": []
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: HistoryArgs =
                serde_json::from_str(&args).unwrap_or(HistoryArgs { limit: 20 });
            let limit = parsed.limit.min(100);

            let history = ops::history(&self.ctx.state, &self.ctx.room, limit)
                .await
                .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&history).map_err(ToolError::JsonError)
        })
    }
}

/// Get room exits
#[derive(Clone)]
struct SshwarmaExits {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaExits {
    fn name(&self) -> String {
        "sshwarma_exits".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_exits".to_string(),
                description: "List exits from the current room (directions and destinations)"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            }
        })
    }

    fn call(&self, _args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let exits = ops::exits(&self.ctx.state, &self.ctx.room)
                .await
                .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&exits).map_err(ToolError::JsonError)
        })
    }
}

/// Get journal entries
#[derive(Clone)]
struct SshwarmaJournal {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct JournalArgs {
    kind: Option<String>,
    #[serde(default = "default_journal_limit")]
    limit: usize,
}

fn default_journal_limit() -> usize {
    20
}

impl ToolDyn for SshwarmaJournal {
    fn name(&self) -> String {
        "sshwarma_journal".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_journal".to_string(),
                description: "Get journal entries (notes, decisions, ideas, milestones)".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["note", "decision", "idea", "milestone"],
                            "description": "Filter by entry type (optional)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Number of entries to retrieve (default: 20)"
                        }
                    },
                    "required": []
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: JournalArgs = serde_json::from_str(&args).unwrap_or(JournalArgs {
                kind: None,
                limit: 20,
            });

            let entries = ops::get_journal(
                &self.ctx.state,
                &self.ctx.room,
                parsed.kind.as_deref(),
                parsed.limit,
            )
            .await
            .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&entries).map_err(ToolError::JsonError)
        })
    }
}

/// List available MCP tools
#[derive(Clone)]
struct SshwarmaTools {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaTools {
    fn name(&self) -> String {
        "sshwarma_tools".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_tools".to_string(),
                description: "List available MCP tools from connected servers".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            }
        })
    }

    fn call(&self, _args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let tools = ops::tools(&self.ctx.state)
                .await
                .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&tools).map_err(ToolError::JsonError)
        })
    }
}

// ============================================================================
// Write Tools (Room Context)
// ============================================================================

/// Say something to the room
#[derive(Clone)]
struct SshwarmaSay {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct SayArgs {
    message: String,
}

impl ToolDyn for SshwarmaSay {
    fn name(&self) -> String {
        "sshwarma_say".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_say".to_string(),
                description: "Say something to the room (visible to all users)".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "The message to say"
                        }
                    },
                    "required": ["message"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            debug!(tool = "sshwarma_say", room = %self.ctx.room, "tool call");
            let parsed: SayArgs =
                serde_json::from_str(&args).map_err(ToolError::JsonError)?;

            ops::say(&self.ctx.state, &self.ctx.room, &self.ctx.username, &parsed.message)
                .await
                .map_err(anyhow_to_tool_error)?;

            Ok(r#"{"status": "ok"}"#.to_string())
        })
    }
}

/// Set or get room vibe
#[derive(Clone)]
struct SshwarmaVibe {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct VibeArgs {
    vibe: Option<String>,
}

impl ToolDyn for SshwarmaVibe {
    fn name(&self) -> String {
        "sshwarma_vibe".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_vibe".to_string(),
                description: "Get or set the room vibe (creative direction/mood)".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "vibe": {
                            "type": "string",
                            "description": "The vibe to set (omit to get current vibe)"
                        }
                    },
                    "required": []
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: VibeArgs = serde_json::from_str(&args).unwrap_or(VibeArgs { vibe: None });

            if let Some(vibe) = parsed.vibe {
                ops::set_vibe(&self.ctx.state, &self.ctx.room, &vibe)
                    .await
                    .map_err(anyhow_to_tool_error)?;
                Ok(json!({"status": "ok", "vibe": vibe}).to_string())
            } else {
                let vibe = ops::get_vibe(&self.ctx.state, &self.ctx.room)
                    .await
                    .map_err(anyhow_to_tool_error)?;
                Ok(json!({"vibe": vibe}).to_string())
            }
        })
    }
}

// Journal entry tools

#[derive(Deserialize)]
struct JournalEntryArgs {
    content: String,
}

macro_rules! journal_tool {
    ($name:ident, $tool_name:expr, $kind:expr, $description:expr) => {
        #[derive(Clone)]
        struct $name {
            ctx: ToolContext,
        }

        impl ToolDyn for $name {
            fn name(&self) -> String {
                $tool_name.to_string()
            }

            fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
                Box::pin(async move {
                    ToolDefinition {
                        name: $tool_name.to_string(),
                        description: $description.to_string(),
                        parameters: json!({
                            "type": "object",
                            "properties": {
                                "content": {
                                    "type": "string",
                                    "description": "The content to add"
                                }
                            },
                            "required": ["content"]
                        }),
                    }
                })
            }

            fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
                Box::pin(async move {
                    debug!(tool = $tool_name, room = %self.ctx.room, "tool call");
                    let parsed: JournalEntryArgs =
                        serde_json::from_str(&args).map_err(ToolError::JsonError)?;

                    ops::add_journal(
                        &self.ctx.state,
                        &self.ctx.room,
                        &self.ctx.username,
                        &parsed.content,
                        $kind,
                    )
                    .await
                    .map_err(anyhow_to_tool_error)?;

                    Ok(r#"{"status": "ok"}"#.to_string())
                })
            }
        }
    };
}

journal_tool!(
    SshwarmaNote,
    "sshwarma_note",
    ops::JournalKind::Note,
    "Add a note to the room journal"
);

journal_tool!(
    SshwarmaDecide,
    "sshwarma_decide",
    ops::JournalKind::Decision,
    "Record a decision in the room journal"
);

journal_tool!(
    SshwarmaIdea,
    "sshwarma_idea",
    ops::JournalKind::Idea,
    "Capture an idea in the room journal"
);

journal_tool!(
    SshwarmaMilestone,
    "sshwarma_milestone",
    ops::JournalKind::Milestone,
    "Mark a milestone in the room journal"
);

/// Add inspiration
#[derive(Clone)]
struct SshwarmaInspire {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct InspireArgs {
    content: Option<String>,
}

impl ToolDyn for SshwarmaInspire {
    fn name(&self) -> String {
        "sshwarma_inspire".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_inspire".to_string(),
                description: "Add inspiration or get existing inspirations".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "Inspiration to add (omit to list existing)"
                        }
                    },
                    "required": []
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: InspireArgs =
                serde_json::from_str(&args).unwrap_or(InspireArgs { content: None });

            if let Some(content) = parsed.content {
                ops::add_inspiration(&self.ctx.state, &self.ctx.room, &content, &self.ctx.username)
                    .await
                    .map_err(anyhow_to_tool_error)?;
                Ok(r#"{"status": "ok"}"#.to_string())
            } else {
                let inspirations = ops::get_inspirations(&self.ctx.state, &self.ctx.room)
                    .await
                    .map_err(anyhow_to_tool_error)?;
                serde_json::to_string(&inspirations).map_err(ToolError::JsonError)
            }
        })
    }
}

// ============================================================================
// Navigation Tools
// ============================================================================

/// Join a room
#[derive(Clone)]
struct SshwarmaJoin {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct JoinArgs {
    room: String,
}

impl ToolDyn for SshwarmaJoin {
    fn name(&self) -> String {
        "sshwarma_join".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_join".to_string(),
                description: "Join a different room".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "room": {
                            "type": "string",
                            "description": "Name of the room to join"
                        }
                    },
                    "required": ["room"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: JoinArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;
            debug!(tool = "sshwarma_join", from = %self.ctx.room, to = %parsed.room, "navigation");

            let summary = ops::join(
                &self.ctx.state,
                &self.ctx.username,
                Some(&self.ctx.room),
                &parsed.room,
            )
            .await
            .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&summary).map_err(ToolError::JsonError)
        })
    }
}

/// Leave room (return to lobby)
#[derive(Clone)]
struct SshwarmaLeave {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaLeave {
    fn name(&self) -> String {
        "sshwarma_leave".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_leave".to_string(),
                description: "Leave the current room and return to lobby".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            }
        })
    }

    fn call(&self, _args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            debug!(tool = "sshwarma_leave", from = %self.ctx.room, "navigation");
            ops::leave(&self.ctx.state, &self.ctx.username, &self.ctx.room)
                .await
                .map_err(anyhow_to_tool_error)?;

            Ok(r#"{"status": "ok", "location": "lobby"}"#.to_string())
        })
    }
}

/// Navigate via exit
#[derive(Clone)]
struct SshwarmaGo {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct GoArgs {
    direction: String,
}

impl ToolDyn for SshwarmaGo {
    fn name(&self) -> String {
        "sshwarma_go".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_go".to_string(),
                description: "Navigate through an exit (north, south, east, west, up, down, etc.)"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "direction": {
                            "type": "string",
                            "description": "Direction to go"
                        }
                    },
                    "required": ["direction"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: GoArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;
            debug!(tool = "sshwarma_go", from = %self.ctx.room, direction = %parsed.direction, "navigation");

            let summary = ops::go(
                &self.ctx.state,
                &self.ctx.username,
                &self.ctx.room,
                &parsed.direction,
            )
            .await
            .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&summary).map_err(ToolError::JsonError)
        })
    }
}

/// Create a new room
#[derive(Clone)]
struct SshwarmaCreate {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct CreateArgs {
    name: String,
}

impl ToolDyn for SshwarmaCreate {
    fn name(&self) -> String {
        "sshwarma_create".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_create".to_string(),
                description: "Create a new room and join it".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name for the new room (alphanumeric, dashes, underscores)"
                        }
                    },
                    "required": ["name"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: CreateArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;
            debug!(tool = "sshwarma_create", room = %parsed.name, "room creation");

            let summary = ops::create_room(
                &self.ctx.state,
                &self.ctx.username,
                &parsed.name,
                Some(&self.ctx.room),
            )
            .await
            .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&summary).map_err(ToolError::JsonError)
        })
    }
}

/// Fork a room
#[derive(Clone)]
struct SshwarmaFork {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct ForkArgs {
    name: String,
}

impl ToolDyn for SshwarmaFork {
    fn name(&self) -> String {
        "sshwarma_fork".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_fork".to_string(),
                description: "Fork the current room (copies vibe, assets, inspirations) and join it"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name for the new forked room"
                        }
                    },
                    "required": ["name"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: ForkArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;
            debug!(tool = "sshwarma_fork", from = %self.ctx.room, to = %parsed.name, "room fork");

            let summary = ops::fork_room(
                &self.ctx.state,
                &self.ctx.username,
                &self.ctx.room,
                &parsed.name,
            )
            .await
            .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&summary).map_err(ToolError::JsonError)
        })
    }
}
