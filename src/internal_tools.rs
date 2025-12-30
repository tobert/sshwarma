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

use crate::ops;
use crate::state::SharedState;

/// Convert anyhow::Error to ToolError
fn anyhow_to_tool_error(e: anyhow::Error) -> ToolError {
    ToolError::ToolCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        e.to_string(),
    )))
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

        // Profile tools (always available in rooms)
        handle.add_tool(SshwarmaListProfiles { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaAddProfile { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaRemoveProfile { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaAssignRole { ctx: ctx.clone() }).await?;
        handle.add_tool(SshwarmaUnassignRole { ctx: ctx.clone() }).await?;
        count += 5;
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
            let summary = ops::look(&self.ctx.state, &self.ctx.room)
                .await
                .map_err(anyhow_to_tool_error)?;

            serde_json::to_string(&summary).map_err(ToolError::JsonError)
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

// ============================================================================
// Profile Tools
// ============================================================================

/// List profiles in room
#[derive(Clone)]
struct SshwarmaListProfiles {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaListProfiles {
    fn name(&self) -> String {
        "sshwarma_list_profiles".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_list_profiles".to_string(),
                description: "List all profiles and role assignments in the current room".to_string(),
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
            let profiles = self.ctx.state.db.get_profiles(&self.ctx.room)
                .map_err(anyhow_to_tool_error)?;
            let role_assignments = self.ctx.state.db.get_role_assignments(&self.ctx.room)
                .map_err(anyhow_to_tool_error)?;

            let result = json!({
                "profiles": profiles.iter().map(|p| {
                    json!({
                        "name": p.name,
                        "target": match &p.target {
                            crate::world::ProfileTarget::Room => json!({"type": "room"}),
                            crate::world::ProfileTarget::Model(m) => json!({"type": "model", "value": m}),
                            crate::world::ProfileTarget::Role(r) => json!({"type": "role", "value": r}),
                        },
                        "priority": p.priority,
                        "system_prompt": p.system_prompt,
                        "context_prefix": p.context_prefix,
                        "context_suffix": p.context_suffix,
                    })
                }).collect::<Vec<_>>(),
                "role_assignments": role_assignments,
            });

            serde_json::to_string(&result).map_err(ToolError::JsonError)
        })
    }
}

/// Add or update a profile
#[derive(Clone)]
struct SshwarmaAddProfile {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct AddProfileArgs {
    name: String,
    target_type: String,
    target_value: Option<String>,
    system_prompt: Option<String>,
    context_prefix: Option<String>,
    context_suffix: Option<String>,
    priority: Option<i32>,
}

impl ToolDyn for SshwarmaAddProfile {
    fn name(&self) -> String {
        "sshwarma_add_profile".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_add_profile".to_string(),
                description: "Add or update a profile that customizes model behavior in this room".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Profile name (unique within room)"
                        },
                        "target_type": {
                            "type": "string",
                            "enum": ["room", "model", "role"],
                            "description": "What this profile targets: 'room' (all models), 'model' (specific model), 'role' (models with assigned role)"
                        },
                        "target_value": {
                            "type": "string",
                            "description": "For model: model handle (e.g., 'qwen-4b'). For role: role name. Not needed for room."
                        },
                        "system_prompt": {
                            "type": "string",
                            "description": "Additional system prompt text"
                        },
                        "context_prefix": {
                            "type": "string",
                            "description": "Text prepended to dynamic context"
                        },
                        "context_suffix": {
                            "type": "string",
                            "description": "Text appended to dynamic context"
                        },
                        "priority": {
                            "type": "integer",
                            "description": "Stacking priority (lower = applied first)"
                        }
                    },
                    "required": ["name", "target_type"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: AddProfileArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;

            let target = match parsed.target_type.as_str() {
                "room" => crate::world::ProfileTarget::Room,
                "model" => {
                    let value = parsed.target_value.ok_or_else(|| {
                        ToolError::ToolCallError(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "target_value required for model target",
                        )))
                    })?;
                    crate::world::ProfileTarget::Model(value)
                }
                "role" => {
                    let value = parsed.target_value.ok_or_else(|| {
                        ToolError::ToolCallError(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "target_value required for role target",
                        )))
                    })?;
                    crate::world::ProfileTarget::Role(value)
                }
                _ => {
                    return Err(ToolError::ToolCallError(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "target_type must be 'room', 'model', or 'role'",
                    ))));
                }
            };

            let profile = crate::world::Profile {
                name: parsed.name.clone(),
                target,
                system_prompt: parsed.system_prompt,
                context_prefix: parsed.context_prefix,
                context_suffix: parsed.context_suffix,
                priority: parsed.priority.unwrap_or(0),
            };

            // Save to DB
            self.ctx.state.db.add_profile(&self.ctx.room, &profile)
                .map_err(anyhow_to_tool_error)?;

            // Update in-memory
            {
                let mut world = tokio::task::block_in_place(|| self.ctx.state.world.blocking_write());
                if let Some(room) = world.rooms.get_mut(&self.ctx.room) {
                    room.context.profiles.retain(|p| p.name != parsed.name);
                    room.context.profiles.push(profile);
                    room.context.profiles.sort_by_key(|p| p.priority);
                }
            }

            Ok(json!({"status": "ok", "profile": parsed.name}).to_string())
        })
    }
}

/// Remove a profile
#[derive(Clone)]
struct SshwarmaRemoveProfile {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct RemoveProfileArgs {
    name: String,
}

impl ToolDyn for SshwarmaRemoveProfile {
    fn name(&self) -> String {
        "sshwarma_remove_profile".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_remove_profile".to_string(),
                description: "Remove a profile from the room".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name of profile to remove"
                        }
                    },
                    "required": ["name"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: RemoveProfileArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;

            let removed = self.ctx.state.db.remove_profile(&self.ctx.room, &parsed.name)
                .map_err(anyhow_to_tool_error)?;

            if removed {
                // Update in-memory
                let mut world = tokio::task::block_in_place(|| self.ctx.state.world.blocking_write());
                if let Some(room) = world.rooms.get_mut(&self.ctx.room) {
                    room.context.profiles.retain(|p| p.name != parsed.name);
                }
            }

            Ok(json!({"status": if removed { "ok" } else { "not_found" }}).to_string())
        })
    }
}

/// Assign a role to a model
#[derive(Clone)]
struct SshwarmaAssignRole {
    ctx: ToolContext,
}

#[derive(Deserialize)]
struct AssignRoleArgs {
    model: String,
    role: String,
}

impl ToolDyn for SshwarmaAssignRole {
    fn name(&self) -> String {
        "sshwarma_assign_role".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_assign_role".to_string(),
                description: "Assign a role to a model in this room".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "model": {
                            "type": "string",
                            "description": "Model handle (e.g., 'qwen-4b')"
                        },
                        "role": {
                            "type": "string",
                            "description": "Role name to assign"
                        }
                    },
                    "required": ["model", "role"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: AssignRoleArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;

            self.ctx.state.db.assign_role(&self.ctx.room, &parsed.model, &parsed.role)
                .map_err(anyhow_to_tool_error)?;

            // Update in-memory
            {
                let mut world = tokio::task::block_in_place(|| self.ctx.state.world.blocking_write());
                if let Some(room) = world.rooms.get_mut(&self.ctx.room) {
                    room.context.role_assignments
                        .entry(parsed.model.clone())
                        .or_default()
                        .push(parsed.role.clone());
                }
            }

            Ok(json!({"status": "ok", "model": parsed.model, "role": parsed.role}).to_string())
        })
    }
}

/// Unassign a role from a model
#[derive(Clone)]
struct SshwarmaUnassignRole {
    ctx: ToolContext,
}

impl ToolDyn for SshwarmaUnassignRole {
    fn name(&self) -> String {
        "sshwarma_unassign_role".to_string()
    }

    fn definition(&self, _prompt: String) -> WasmBoxedFuture<'_, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: "sshwarma_unassign_role".to_string(),
                description: "Unassign a role from a model in this room".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "model": {
                            "type": "string",
                            "description": "Model handle (e.g., 'qwen-4b')"
                        },
                        "role": {
                            "type": "string",
                            "description": "Role name to unassign"
                        }
                    },
                    "required": ["model", "role"]
                }),
            }
        })
    }

    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let parsed: AssignRoleArgs = serde_json::from_str(&args).map_err(ToolError::JsonError)?;

            let removed = self.ctx.state.db.unassign_role(&self.ctx.room, &parsed.model, &parsed.role)
                .map_err(anyhow_to_tool_error)?;

            if removed {
                // Update in-memory
                let mut world = tokio::task::block_in_place(|| self.ctx.state.world.blocking_write());
                if let Some(room) = world.rooms.get_mut(&self.ctx.room) {
                    if let Some(roles) = room.context.role_assignments.get_mut(&parsed.model) {
                        roles.retain(|r| r != &parsed.role);
                    }
                }
            }

            Ok(json!({"status": if removed { "ok" } else { "not_found" }}).to_string())
        })
    }
}
