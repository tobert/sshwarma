//! MCP server for exposing sshwarma tools to Claude Code
//!
//! This module provides an MCP server that allows Claude Code to interact
//! with sshwarma rooms - listing rooms, viewing history, sending messages.

use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
    ServerHandler,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

use crate::db::rules::{ActionSlot, RoomRule, TriggerKind};
use crate::db::scripts::{LuaScript, ScriptKind};
use crate::db::Database;
use crate::llm::LlmClient;
use crate::lua::{LuaRuntime, WrapState};
use crate::model::{ModelBackend, ModelHandle, ModelRegistry};
use crate::state::SharedState;
use crate::world::{JournalKind, World};
use tokio::sync::{Mutex, RwLock};

/// Shared state for the MCP server
pub struct McpServerState {
    pub world: Arc<RwLock<World>>,
    pub db: Arc<Database>,
    pub llm: Arc<LlmClient>,
    pub models: Arc<ModelRegistry>,
    pub lua_runtime: Arc<Mutex<LuaRuntime>>,
    pub shared_state: Arc<SharedState>,
}

/// MCP server for sshwarma
#[derive(Clone)]
pub struct SshwarmaMcpServer {
    state: Arc<McpServerState>,
    tool_router: ToolRouter<Self>,
}

/// Parameters for list_rooms
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListRoomsParams {}

/// Parameters for get_history
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetHistoryParams {
    #[schemars(description = "Name of the room to get history from")]
    pub room: String,
    #[schemars(description = "Number of messages to retrieve (default 50, max 200)")]
    pub limit: Option<usize>,
}

/// Parameters for say
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SayParams {
    #[schemars(description = "Name of the room to send message to")]
    pub room: String,
    #[schemars(description = "Message content to send")]
    pub message: String,
    #[schemars(description = "Sender name (defaults to 'claude')")]
    pub sender: Option<String>,
}

/// Parameters for ask_model
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AskModelParams {
    #[schemars(description = "Short name of the model to ask (e.g. 'qwen-8b')")]
    pub model: String,
    #[schemars(description = "Message to send to the model")]
    pub message: String,
    #[schemars(description = "Optional room context - if provided, includes recent history")]
    pub room: Option<String>,
}

/// Parameters for list_models
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListModelsParams {}

/// Parameters for create_room
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateRoomParams {
    #[schemars(description = "Name for the new room (alphanumeric, dashes, underscores)")]
    pub name: String,
    #[schemars(description = "Optional description for the room")]
    pub description: Option<String>,
}

/// Parameters for room_context
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomContextParams {
    #[schemars(description = "Name of the room to get context for")]
    pub room: String,
}

/// Parameters for journal_read
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct JournalReadParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Optional filter by kind: note, decision, milestone, idea, question")]
    pub kind: Option<String>,
    #[schemars(description = "Maximum entries to return (default 20)")]
    pub limit: Option<usize>,
}

/// Parameters for journal_write
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct JournalWriteParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Kind of entry: note, decision, milestone, idea, question")]
    pub kind: String,
    #[schemars(description = "Content of the journal entry")]
    pub content: String,
    #[schemars(description = "Author name (defaults to 'claude')")]
    pub author: Option<String>,
}

/// Parameters for asset_bind
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssetBindParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Artifact ID to bind (e.g. CAS hash or external ID)")]
    pub artifact_id: String,
    #[schemars(
        description = "Semantic role for the asset (e.g. 'drums', 'main_theme', 'reference')"
    )]
    pub role: String,
    #[schemars(description = "Optional notes about this binding")]
    pub notes: Option<String>,
    #[schemars(description = "Who is binding (defaults to 'claude')")]
    pub bound_by: Option<String>,
}

/// Parameters for asset_unbind
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssetUnbindParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Role to unbind")]
    pub role: String,
}

/// Parameters for asset_lookup
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssetLookupParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Role to look up")]
    pub role: String,
}

/// Parameters for set_vibe
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetVibeParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Vibe/atmosphere description for the room")]
    pub vibe: String,
}

/// Parameters for add_exit
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddExitParams {
    #[schemars(description = "Name of the source room")]
    pub room: String,
    #[schemars(description = "Direction (e.g. 'north', 'studio', 'archive')")]
    pub direction: String,
    #[schemars(description = "Target room name")]
    pub target: String,
    #[schemars(description = "Create bidirectional exit (default true)")]
    pub bidirectional: Option<bool>,
}

/// Parameters for fork_room
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ForkRoomParams {
    #[schemars(description = "Name of the source room to fork from")]
    pub source: String,
    #[schemars(description = "Name for the new forked room")]
    pub new_name: String,
}

/// Parameters for preview_wrap
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewWrapParams {
    #[schemars(
        description = "Model short name (e.g. 'qwen-8b'). If not specified, uses a preview model."
    )]
    pub model: Option<String>,
    #[schemars(description = "Room to preview context for (optional)")]
    pub room: Option<String>,
    #[schemars(description = "Username to simulate (defaults to 'claude')")]
    pub username: Option<String>,
}

/// Parameters for list_rules
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListRulesParams {
    #[schemars(description = "Name of the room to list rules for")]
    pub room: String,
}

/// Parameters for create_rule
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateRuleParams {
    #[schemars(description = "Name of the room to add the rule to")]
    pub room: String,
    #[schemars(description = "Trigger type: 'tick', 'interval', or 'row'")]
    pub trigger_kind: String,
    #[schemars(description = "For tick triggers: run every N ticks (500ms each)")]
    pub tick_divisor: Option<i32>,
    #[schemars(description = "For interval triggers: milliseconds between runs")]
    pub interval_ms: Option<i64>,
    #[schemars(
        description = "For row triggers: glob pattern to match content_method (e.g. 'message.*')"
    )]
    pub match_pattern: Option<String>,
    #[schemars(description = "Name of the script to execute (must exist in database)")]
    pub script_name: String,
    #[schemars(description = "Optional human-readable name for the rule")]
    pub name: Option<String>,
}

/// Parameters for delete_rule
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteRuleParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Rule ID or prefix to delete")]
    pub rule_id: String,
}

/// Parameters for toggle_rule
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ToggleRuleParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Rule ID or prefix")]
    pub rule_id: String,
    #[schemars(description = "Enable (true) or disable (false) the rule")]
    pub enabled: bool,
}

/// Parameters for list_scripts
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListScriptsParams {}

/// Parameters for create_script
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateScriptParams {
    #[schemars(description = "Unique name for the script")]
    pub name: String,
    #[schemars(description = "Script kind: 'handler', 'renderer', or 'transformer'")]
    pub kind: String,
    #[schemars(description = "Lua source code. Must define handle(tick, state) for handlers.")]
    pub code: String,
    #[schemars(description = "Optional description of what the script does")]
    pub description: Option<String>,
}

/// Parameters for inventory_list
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InventoryListParams {
    #[schemars(description = "Name of the room to list inventory for")]
    pub room: String,
    #[schemars(description = "Include available (unequipped) tools in list")]
    pub include_available: Option<bool>,
}

/// Parameters for inventory_equip
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InventoryEquipParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Qualified name of the thing to equip (e.g. 'holler:sample')")]
    pub qualified_name: String,
    #[schemars(description = "Priority for ordering (lower = first)")]
    pub priority: Option<f64>,
}

/// Parameters for inventory_unequip
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InventoryUnequipParams {
    #[schemars(description = "Name of the room")]
    pub room: String,
    #[schemars(description = "Qualified name of the thing to unequip")]
    pub qualified_name: String,
}

#[tool_router]
impl SshwarmaMcpServer {
    pub fn new(state: Arc<McpServerState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List all available rooms")]
    async fn list_rooms(&self, Parameters(_params): Parameters<ListRoomsParams>) -> String {
        let world = self.state.world.read().await;
        let rooms = world.list_rooms();

        if rooms.is_empty() {
            return "No rooms exist yet.".to_string();
        }

        let mut output = String::new();
        for room in rooms {
            output.push_str(&format!("- {} ({} users)\n", room.name, room.user_count));
        }
        output
    }

    #[tool(description = "Get recent message history from a room")]
    async fn get_history(&self, Parameters(params): Parameters<GetHistoryParams>) -> String {
        let limit = params.limit.unwrap_or(50).min(200);

        match self.state.db.recent_messages(&params.room, limit) {
            Ok(messages) => {
                // Filter to only actual messages
                let filtered: Vec<_> = messages
                    .iter()
                    .filter(|m| {
                        m.message_type.starts_with("message.") && !m.content.is_empty() && !m.hidden
                    })
                    .collect();

                if filtered.is_empty() {
                    return format!("No messages in room '{}'.", params.room);
                }

                let mut output = format!(
                    "--- History for {} ({} messages) ---\n",
                    params.room,
                    filtered.len()
                );
                for msg in filtered {
                    output.push_str(&format!(
                        "[{}] {}: {}\n",
                        msg.timestamp, msg.sender_name, msg.content
                    ));
                }
                output
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Send a message to a room")]
    async fn say(&self, Parameters(params): Parameters<SayParams>) -> String {
        let sender = params.sender.unwrap_or_else(|| "claude".to_string());

        // Check if room exists
        {
            let world = self.state.world.read().await;
            if world.get_room(&params.room).is_none() {
                return format!("Room '{}' does not exist.", params.room);
            }
        }

        // Add message using new Row/Buffer system
        use crate::db::rows::Row;

        // Get or create the room's buffer
        let buffer = match self.state.db.get_or_create_room_buffer(&params.room) {
            Ok(b) => b,
            Err(e) => return format!("Error getting room buffer: {}", e),
        };

        // Get or create agent for sender
        let agent = match self.state.db.get_or_create_human_agent(&sender) {
            Ok(a) => a,
            Err(e) => return format!("Error getting agent: {}", e),
        };

        // Create and add the row
        let mut row = Row::message(&buffer.id, &agent.id, &params.message, false);
        match self.state.db.append_row(&mut row) {
            Ok(_) => format!("{}: {}", sender, params.message),
            Err(e) => format!("Error saving message: {}", e),
        }
    }

    #[tool(description = "Ask a model a question, optionally with room context")]
    async fn ask_model(&self, Parameters(params): Parameters<AskModelParams>) -> String {
        // Look up the model
        let model = match self.state.models.get(&params.model) {
            Some(m) => m.clone(),
            None => {
                let available: Vec<_> = self
                    .state
                    .models
                    .available()
                    .iter()
                    .map(|m| m.short_name.as_str())
                    .collect();
                return format!(
                    "Unknown model '{}'. Available: {}",
                    params.model,
                    available.join(", ")
                );
            }
        };

        // Build context from room buffer if provided
        use crate::db::rows::Row;

        let history = if let Some(ref room_name) = params.room {
            // Get room buffer
            if let Ok(buffer) = self.state.db.get_or_create_room_buffer(room_name) {
                // Get recent message rows
                if let Ok(rows) = self.state.db.list_recent_buffer_rows(&buffer.id, 10) {
                    rows.into_iter()
                        .filter(|r| !r.ephemeral)
                        .filter_map(|row| {
                            let content = row.content.as_deref()?;
                            let role = if row.content_method == "message.user" {
                                "user"
                            } else if row.content_method == "message.model" {
                                "assistant"
                            } else {
                                return None;
                            };
                            Some((role.to_string(), content.to_string()))
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let system_prompt = format!(
            "You are {} in sshwarma collaborative chat. Be helpful, concise, and friendly.",
            model.display_name
        );

        match self
            .state
            .llm
            .chat_with_context(&model, &system_prompt, &history, &params.message)
            .await
        {
            Ok(response) => {
                // Record in room if specified
                if let Some(ref room_name) = params.room {
                    // Get buffer
                    if let Ok(buffer) = self.state.db.get_or_create_room_buffer(room_name) {
                        // Get or create model agent
                        if let Ok(agent) =
                            self.state.db.get_or_create_model_agent(&model.short_name)
                        {
                            // Add model response row
                            let mut row = Row::new(&buffer.id, "message.model");
                            row.source_agent_id = Some(agent.id);
                            row.content = Some(response.clone());
                            let _ = self.state.db.append_row(&mut row);
                        }
                    }
                }

                format!("{}: {}", model.short_name, response)
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "List available AI models")]
    async fn list_models(&self, Parameters(_params): Parameters<ListModelsParams>) -> String {
        let models = self.state.models.available();

        if models.is_empty() {
            return "No models available.".to_string();
        }

        let mut output = String::new();
        for model in models {
            output.push_str(&format!(
                "- {} ({})\n",
                model.short_name, model.display_name
            ));
        }
        output
    }

    #[tool(description = "Create a new room")]
    async fn create_room(&self, Parameters(params): Parameters<CreateRoomParams>) -> String {
        // Validate room name
        if !params
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return "Room name can only contain letters, numbers, dashes, and underscores."
                .to_string();
        }

        // Check if room exists
        {
            let world = self.state.world.read().await;
            if world.get_room(&params.name).is_some() {
                return format!("Room '{}' already exists.", params.name);
            }
        }

        // Create in memory
        {
            let mut world = self.state.world.write().await;
            world.create_room(params.name.clone());
            if let Some(room) = world.get_room_mut(&params.name) {
                room.description = params.description.clone();
            }
        }

        // Persist
        match self
            .state
            .db
            .create_room(&params.name, params.description.as_deref())
        {
            Ok(_) => format!("Created room '{}'.", params.name),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get full room context for agent onboarding - vibe, assets, journal, exits"
    )]
    async fn room_context(&self, Parameters(params): Parameters<RoomContextParams>) -> String {
        let mut output = String::new();

        // Get vibe
        let vibe = self.state.db.get_vibe(&params.room).ok().flatten();
        output.push_str(&format!("# Room: {}\n\n", params.room));

        if let Some(v) = vibe {
            output.push_str(&format!("## Vibe\n{}\n\n", v));
        }

        // Get parent (fork lineage)
        if let Ok(Some(parent)) = self.state.db.get_parent(&params.room) {
            output.push_str(&format!("## Parent\nForked from: {}\n\n", parent));
        }

        // Get tags
        if let Ok(tags) = self.state.db.get_tags(&params.room) {
            if !tags.is_empty() {
                let tags_vec: Vec<_> = tags.into_iter().collect();
                output.push_str(&format!("## Tags\n{}\n\n", tags_vec.join(", ")));
            }
        }

        // Get assets
        if let Ok(assets) = self.state.db.list_asset_bindings(&params.room) {
            if !assets.is_empty() {
                output.push_str("## Bound Assets\n");
                for asset in assets {
                    output.push_str(&format!("- **{}**: `{}`", asset.role, asset.artifact_id));
                    if let Some(notes) = &asset.notes {
                        output.push_str(&format!(" - {}", notes));
                    }
                    output.push('\n');
                }
                output.push('\n');
            }
        }

        // Get exits
        if let Ok(exits) = self.state.db.get_exits(&params.room) {
            if !exits.is_empty() {
                output.push_str("## Exits\n");
                for (direction, target) in &exits {
                    output.push_str(&format!("- {} → {}\n", direction, target));
                }
                output.push('\n');
            }
        }

        // Get recent journal entries
        if let Ok(entries) = self.state.db.get_journal_entries(&params.room, None, 5) {
            if !entries.is_empty() {
                output.push_str("## Recent Journal\n");
                for entry in entries {
                    output.push_str(&format!(
                        "- [{}] {}: {}\n",
                        entry.kind, entry.author, entry.content
                    ));
                }
                output.push('\n');
            }
        }

        // Get inspirations
        if let Ok(inspirations) = self.state.db.get_inspirations(&params.room) {
            if !inspirations.is_empty() {
                output.push_str("## Inspirations\n");
                for insp in inspirations {
                    output.push_str(&format!("- {}\n", insp.content));
                }
            }
        }

        if output.trim().is_empty() {
            format!("Room '{}' has no context set.", params.room)
        } else {
            output
        }
    }

    #[tool(description = "Read journal entries from a room")]
    async fn journal_read(&self, Parameters(params): Parameters<JournalReadParams>) -> String {
        let kind = params.kind.as_ref().and_then(|k| JournalKind::parse(k));
        let limit = params.limit.unwrap_or(20);

        match self.state.db.get_journal_entries(&params.room, kind, limit) {
            Ok(entries) => {
                if entries.is_empty() {
                    return format!("No journal entries in room '{}'.", params.room);
                }

                let mut output = format!(
                    "## Journal for {} ({} entries)\n\n",
                    params.room,
                    entries.len()
                );
                for entry in entries {
                    let timestamp = entry.timestamp.format("%Y-%m-%d %H:%M");
                    output.push_str(&format!(
                        "[{}] **{}** ({})\n{}\n\n",
                        timestamp, entry.kind, entry.author, entry.content
                    ));
                }
                output
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Write a journal entry to a room")]
    async fn journal_write(&self, Parameters(params): Parameters<JournalWriteParams>) -> String {
        let kind = match JournalKind::parse(&params.kind) {
            Some(k) => k,
            None => {
                return format!(
                    "Invalid kind '{}'. Use: note, decision, milestone, idea, question",
                    params.kind
                )
            }
        };

        let author = params.author.unwrap_or_else(|| "claude".to_string());

        match self
            .state
            .db
            .add_journal_entry(&params.room, &author, &params.content, kind)
        {
            Ok(_) => format!("Added {} to journal: {}", params.kind, params.content),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Bind an artifact to a room with a semantic role")]
    async fn asset_bind(&self, Parameters(params): Parameters<AssetBindParams>) -> String {
        let bound_by = params.bound_by.unwrap_or_else(|| "claude".to_string());

        match self.state.db.bind_asset(
            &params.room,
            &params.role,
            &params.artifact_id,
            params.notes.as_deref(),
            &bound_by,
        ) {
            Ok(_) => format!(
                "Bound '{}' as '{}' in room '{}'.",
                params.artifact_id, params.role, params.room
            ),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Unbind an asset from a room by role")]
    async fn asset_unbind(&self, Parameters(params): Parameters<AssetUnbindParams>) -> String {
        match self.state.db.unbind_asset(&params.room, &params.role) {
            Ok(_) => format!("Unbound '{}' from room '{}'.", params.role, params.room),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Look up an asset by role in a room")]
    async fn asset_lookup(&self, Parameters(params): Parameters<AssetLookupParams>) -> String {
        match self.state.db.get_asset_binding(&params.room, &params.role) {
            Ok(Some(binding)) => {
                let mut output = format!("**{}**: `{}`\n", binding.role, binding.artifact_id);
                if let Some(notes) = &binding.notes {
                    output.push_str(&format!("Notes: {}\n", notes));
                }
                output.push_str(&format!(
                    "Bound by {} at {}",
                    binding.bound_by, binding.bound_at
                ));
                output
            }
            Ok(None) => format!(
                "No asset bound as '{}' in room '{}'.",
                params.role, params.room
            ),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Set the vibe/atmosphere for a room")]
    async fn set_vibe(&self, Parameters(params): Parameters<SetVibeParams>) -> String {
        match self.state.db.set_vibe(&params.room, Some(&params.vibe)) {
            Ok(_) => format!("Set vibe for '{}': {}", params.room, params.vibe),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Create an exit from one room to another")]
    async fn add_exit(&self, Parameters(params): Parameters<AddExitParams>) -> String {
        let bidirectional = params.bidirectional.unwrap_or(true);

        // Add forward exit
        if let Err(e) = self
            .state
            .db
            .add_exit(&params.room, &params.direction, &params.target)
        {
            return format!("Error: {}", e);
        }

        // Add reverse exit if bidirectional
        if bidirectional {
            let reverse_dir = match params.direction.as_str() {
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

            if let Err(e) = self
                .state
                .db
                .add_exit(&params.target, reverse_dir, &params.room)
            {
                return format!(
                    "Created exit {} → {} but failed to create reverse: {}",
                    params.direction, params.target, e
                );
            }

            format!(
                "Created exits: {} ({} → {}) and {} ({} → {})",
                params.direction,
                params.room,
                params.target,
                reverse_dir,
                params.target,
                params.room
            )
        } else {
            format!(
                "Created exit: {} ({} → {})",
                params.direction, params.room, params.target
            )
        }
    }

    #[tool(description = "Fork a room, inheriting its context")]
    async fn fork_room(&self, Parameters(params): Parameters<ForkRoomParams>) -> String {
        // Validate new room name
        if !params
            .new_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return "Room name can only contain letters, numbers, dashes, and underscores."
                .to_string();
        }

        // Check source exists
        {
            let world = self.state.world.read().await;
            if world.get_room(&params.source).is_none() {
                return format!("Source room '{}' does not exist.", params.source);
            }
            if world.get_room(&params.new_name).is_some() {
                return format!("Room '{}' already exists.", params.new_name);
            }
        }

        // Fork creates both the room and copies context in db
        match self.state.db.fork_room(&params.source, &params.new_name) {
            Ok(_) => {
                // Also create in memory
                let mut world = self.state.world.write().await;
                world.create_room(params.new_name.clone());
            }
            Err(e) => return format!("Error forking: {}", e),
        }

        // Report success
        match self.state.db.get_parent(&params.new_name) {
            Ok(_) => format!(
                "Forked '{}' from '{}'. Inherited: vibe, tags, assets, inspirations.",
                params.new_name, params.source
            ),
            Err(e) => format!("Error forking context: {}", e),
        }
    }

    #[tool(description = "Preview what context would be composed for an LLM interaction")]
    async fn preview_wrap(&self, Parameters(params): Parameters<PreviewWrapParams>) -> String {
        let username = params.username.unwrap_or_else(|| "claude".to_string());

        // Get model from params or use a preview mock
        let model = if let Some(model_name) = &params.model {
            match self.state.models.get(model_name) {
                Some(m) => m.clone(),
                None => {
                    let available: Vec<_> = self
                        .state
                        .models
                        .available()
                        .iter()
                        .map(|m| m.short_name.as_str())
                        .collect();
                    return format!(
                        "Unknown model '{}'. Available: {}",
                        model_name,
                        available.join(", ")
                    );
                }
            }
        } else {
            // Mock model for preview
            ModelHandle {
                short_name: "preview".to_string(),
                display_name: "Preview Model".to_string(),
                backend: ModelBackend::Mock {
                    prefix: "[preview]".to_string(),
                },
                available: true,
                system_prompt: Some("This is a preview of context composition.".to_string()),
                context_window: Some(30000),
            }
        };

        let target_tokens = model.context_window.unwrap_or(30000);

        // Use the persistent LuaRuntime with full SharedState
        let wrap_state = WrapState {
            room_name: params.room.clone(),
            username: username.clone(),
            model: model.clone(),
            shared_state: self.state.shared_state.clone(),
        };

        let lua_runtime = self.state.lua_runtime.lock().await;

        // Set session context so tools.history() etc. work
        lua_runtime
            .tool_state()
            .set_session_context(Some(crate::lua::SessionContext {
                username,
                model: Some(model.clone()),
                room_name: params.room,
            }));
        lua_runtime
            .tool_state()
            .set_shared_state(Some(self.state.shared_state.clone()));

        match lua_runtime.wrap(wrap_state, target_tokens) {
            Ok(result) => {
                let system_tokens = result.system_prompt.len() / 4;
                let context_tokens = result.context.len() / 4;

                format!(
                    "=== wrap() preview for @{} ===\n\n\
                     --- SYSTEM PROMPT ({} tokens, cacheable) ---\n{}\n\n\
                     --- CONTEXT ({} tokens, dynamic) ---\n{}\n\n\
                     Total: ~{} tokens of {} budget",
                    model.short_name,
                    system_tokens,
                    result.system_prompt,
                    context_tokens,
                    if result.context.is_empty() {
                        "(empty)"
                    } else {
                        &result.context
                    },
                    system_tokens + context_tokens,
                    target_tokens
                )
            }
            Err(e) => format!("Error composing context: {}", e),
        }
    }

    #[tool(description = "List rules in a room")]
    async fn list_rules(&self, Parameters(params): Parameters<ListRulesParams>) -> String {
        match self.state.db.list_room_rules(&params.room) {
            Ok(rules) => {
                if rules.is_empty() {
                    return format!("No rules in room '{}'.", params.room);
                }

                let mut output = format!("## Rules for {}\n\n", params.room);
                for rule in rules {
                    let status = if rule.enabled { "✓" } else { "✗" };
                    let trigger = match rule.trigger_kind {
                        TriggerKind::Tick => format!("tick:{}", rule.tick_divisor.unwrap_or(1)),
                        TriggerKind::Interval => {
                            format!("interval:{}ms", rule.interval_ms.unwrap_or(0))
                        }
                        TriggerKind::Row => {
                            format!(
                                "row:{}",
                                rule.match_content_method.as_deref().unwrap_or("*")
                            )
                        }
                    };
                    let name = rule.name.as_deref().unwrap_or("(unnamed)");
                    output.push_str(&format!(
                        "[{}] {} `{}` → {} ({})\n",
                        status,
                        &rule.id[..8],
                        trigger,
                        rule.script_id,
                        name
                    ));
                }
                output
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Create a new rule for a room")]
    async fn create_rule(&self, Parameters(params): Parameters<CreateRuleParams>) -> String {
        // Parse trigger kind
        let trigger_kind = match params.trigger_kind.as_str() {
            "tick" => TriggerKind::Tick,
            "interval" => TriggerKind::Interval,
            "row" => TriggerKind::Row,
            _ => {
                return format!(
                    "Invalid trigger_kind '{}'. Use: tick, interval, row",
                    params.trigger_kind
                )
            }
        };

        // Validate trigger parameters
        match trigger_kind {
            TriggerKind::Tick => {
                if params.tick_divisor.is_none() {
                    return "tick trigger requires tick_divisor parameter".to_string();
                }
            }
            TriggerKind::Interval => {
                if params.interval_ms.is_none() {
                    return "interval trigger requires interval_ms parameter".to_string();
                }
            }
            TriggerKind::Row => {
                if params.match_pattern.is_none() {
                    return "row trigger requires match_pattern parameter".to_string();
                }
            }
        }

        // Look up script by name
        let script = match self.state.db.get_script_by_name(&params.script_name) {
            Ok(Some(s)) => s,
            Ok(None) => return format!("Script '{}' not found.", params.script_name),
            Err(e) => return format!("Error looking up script: {}", e),
        };

        // Create the rule
        let rule = RoomRule {
            id: uuid::Uuid::now_v7().to_string(),
            room_id: params.room.clone(),
            name: params.name,
            enabled: true,
            priority: 0.0,
            trigger_kind,
            match_content_method: params.match_pattern,
            match_source_agent: None,
            match_tag: None,
            match_buffer_type: None,
            interval_ms: params.interval_ms,
            tick_divisor: params.tick_divisor,
            script_id: script.id.clone(),
            action_slot: ActionSlot::Background,
            created_at: chrono::Utc::now().timestamp(),
        };

        match self.state.db.insert_rule(&rule) {
            Ok(_) => format!(
                "Created rule {} in room '{}': {} → {}",
                &rule.id[..8],
                params.room,
                params.trigger_kind,
                params.script_name
            ),
            Err(e) => format!("Error creating rule: {}", e),
        }
    }

    #[tool(description = "Delete a rule from a room")]
    async fn delete_rule(&self, Parameters(params): Parameters<DeleteRuleParams>) -> String {
        // Find rule by ID prefix
        let rules = match self.state.db.list_room_rules(&params.room) {
            Ok(r) => r,
            Err(e) => return format!("Error listing rules: {}", e),
        };

        let matching: Vec<_> = rules
            .iter()
            .filter(|r| r.id.starts_with(&params.rule_id))
            .collect();

        match matching.len() {
            0 => format!(
                "No rule matching '{}' in room '{}'.",
                params.rule_id, params.room
            ),
            1 => {
                let rule = matching[0];
                match self.state.db.delete_rule(&rule.id) {
                    Ok(_) => format!("Deleted rule {}.", &rule.id[..8]),
                    Err(e) => format!("Error deleting rule: {}", e),
                }
            }
            n => format!(
                "Ambiguous: {} rules match '{}'. Be more specific.",
                n, params.rule_id
            ),
        }
    }

    #[tool(description = "Enable or disable a rule")]
    async fn toggle_rule(&self, Parameters(params): Parameters<ToggleRuleParams>) -> String {
        // Find rule by ID prefix
        let rules = match self.state.db.list_room_rules(&params.room) {
            Ok(r) => r,
            Err(e) => return format!("Error listing rules: {}", e),
        };

        let matching: Vec<_> = rules
            .iter()
            .filter(|r| r.id.starts_with(&params.rule_id))
            .collect();

        match matching.len() {
            0 => format!(
                "No rule matching '{}' in room '{}'.",
                params.rule_id, params.room
            ),
            1 => {
                let rule = matching[0];
                match self.state.db.set_rule_enabled(&rule.id, params.enabled) {
                    Ok(_) => {
                        let status = if params.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        };
                        format!("Rule {} {}.", &rule.id[..8], status)
                    }
                    Err(e) => format!("Error updating rule: {}", e),
                }
            }
            n => format!(
                "Ambiguous: {} rules match '{}'. Be more specific.",
                n, params.rule_id
            ),
        }
    }

    #[tool(description = "List available Lua scripts")]
    async fn list_scripts(&self, Parameters(_params): Parameters<ListScriptsParams>) -> String {
        match self.state.db.list_scripts(None) {
            Ok(scripts) => {
                if scripts.is_empty() {
                    return "No scripts found.".to_string();
                }

                let mut output = "## Available Scripts\n\n".to_string();
                for script in scripts {
                    let name = script.name.as_deref().unwrap_or("(anonymous)");
                    let desc = script.description.as_deref().unwrap_or("No description");
                    output.push_str(&format!("- **{}** ({:?}): {}\n", name, script.kind, desc));
                }
                output
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Create a new Lua script")]
    async fn create_script(&self, Parameters(params): Parameters<CreateScriptParams>) -> String {
        // Parse script kind
        let kind = match params.kind.as_str() {
            "handler" => ScriptKind::Handler,
            "renderer" => ScriptKind::Renderer,
            "transformer" => ScriptKind::Transformer,
            _ => {
                return format!(
                    "Invalid kind '{}'. Use: handler, renderer, transformer",
                    params.kind
                )
            }
        };

        let script = LuaScript {
            id: uuid::Uuid::now_v7().to_string(),
            name: Some(params.name.clone()),
            kind,
            code: params.code,
            description: params.description,
            created_at: chrono::Utc::now().timestamp(),
            updated_at: chrono::Utc::now().timestamp(),
        };

        match self.state.db.insert_script(&script) {
            Ok(_) => format!("Created script '{}'.", params.name),
            Err(e) => format!("Error creating script: {}", e),
        }
    }

    // =========================================================================
    // Inventory tools (things system)
    // =========================================================================

    #[tool(description = "List equipped tools in a room's inventory")]
    async fn inventory_list(&self, Parameters(params): Parameters<InventoryListParams>) -> String {
        use crate::db::things::ThingKind;

        // Ensure world is bootstrapped
        if let Err(e) = self.state.db.bootstrap_world() {
            return format!("Error bootstrapping: {}", e);
        }

        // Find room thing by name
        let room_thing = match self.state.db.find_things_by_name(&params.room) {
            Ok(things) => things.into_iter().find(|t| t.kind == ThingKind::Room),
            Err(e) => return format!("Error: {}", e),
        };

        let room_thing = match room_thing {
            Some(t) => t,
            None => {
                // Room might exist in old system but not in things
                let world = self.state.world.read().await;
                if world.get_room(&params.room).is_some() {
                    drop(world);
                    // Create thing for it
                    let mut new_room =
                        crate::db::things::Thing::room(&params.room).with_parent("rooms");
                    new_room.id = format!("room_{}", params.room);
                    if let Err(e) = self.state.db.insert_thing(&new_room) {
                        return format!("Error creating room thing: {}", e);
                    }
                    if let Err(e) = self.state.db.copy_equipped("defaults", &new_room.id) {
                        return format!("Error copying defaults: {}", e);
                    }
                    new_room
                } else {
                    return format!("Room '{}' does not exist.", params.room);
                }
            }
        };

        // Get equipped tools
        let equipped = match self.state.db.get_equipped_tools(&room_thing.id) {
            Ok(e) => e,
            Err(e) => return format!("Error: {}", e),
        };

        let mut output = format!("Inventory for '{}':\n\nEquipped:\n", params.room);
        if equipped.is_empty() {
            output.push_str("  (none)\n");
        } else {
            for eq in &equipped {
                let status = if eq.thing.available { "✓" } else { "○" };
                let qname = eq.thing.qualified_name.as_deref().unwrap_or(&eq.thing.name);
                output.push_str(&format!("  {} {}\n", status, qname));
            }
        }

        // Show available if requested
        if params.include_available.unwrap_or(false) {
            let all_tools = match self.state.db.list_things_by_kind(ThingKind::Tool) {
                Ok(t) => t,
                Err(e) => return format!("Error: {}", e),
            };

            let equipped_ids: std::collections::HashSet<_> =
                equipped.iter().map(|e| e.thing.id.as_str()).collect();

            let available: Vec<_> = all_tools
                .iter()
                .filter(|t| t.available && !equipped_ids.contains(t.id.as_str()))
                .collect();

            if !available.is_empty() {
                output.push_str("\nAvailable to equip:\n");
                for tool in available {
                    let qname = tool.qualified_name.as_deref().unwrap_or(&tool.name);
                    output.push_str(&format!("  ○ {}\n", qname));
                }
            }
        }

        output
    }

    #[tool(description = "Equip a tool in a room")]
    async fn inventory_equip(
        &self,
        Parameters(params): Parameters<InventoryEquipParams>,
    ) -> String {
        use crate::db::things::ThingKind;

        // Ensure world is bootstrapped
        if let Err(e) = self.state.db.bootstrap_world() {
            return format!("Error: {}", e);
        }

        // Find room thing
        let room_thing = match self.state.db.find_things_by_name(&params.room) {
            Ok(things) => things.into_iter().find(|t| t.kind == ThingKind::Room),
            Err(e) => return format!("Error: {}", e),
        };

        let room_thing = match room_thing {
            Some(t) => t,
            None => return format!("Room '{}' not found in things system.", params.room),
        };

        // Find thing by qualified name
        let things = if params.qualified_name.contains('*') {
            match self
                .state
                .db
                .find_things_by_qualified_name(&params.qualified_name)
            {
                Ok(t) => t,
                Err(e) => return format!("Error: {}", e),
            }
        } else {
            match self
                .state
                .db
                .get_thing_by_qualified_name(&params.qualified_name)
            {
                Ok(Some(t)) => vec![t],
                Ok(None) => return format!("Thing '{}' not found.", params.qualified_name),
                Err(e) => return format!("Error: {}", e),
            }
        };

        if things.is_empty() {
            return format!("No things matching '{}'", params.qualified_name);
        }

        // Equip each thing
        let priority = params.priority.unwrap_or(0.0);
        let mut equipped_count = 0;
        for thing in &things {
            if let Err(e) = self.state.db.equip(&room_thing.id, &thing.id, priority) {
                return format!("Error equipping {}: {}", thing.name, e);
            }
            equipped_count += 1;
        }

        if equipped_count == 1 {
            let qname = things[0]
                .qualified_name
                .as_deref()
                .unwrap_or(&things[0].name);
            format!("Equipped {} in {}", qname, params.room)
        } else {
            format!(
                "Equipped {} things matching '{}' in {}",
                equipped_count, params.qualified_name, params.room
            )
        }
    }

    #[tool(description = "Unequip a tool from a room")]
    async fn inventory_unequip(
        &self,
        Parameters(params): Parameters<InventoryUnequipParams>,
    ) -> String {
        use crate::db::things::ThingKind;

        // Find room thing
        let room_thing = match self.state.db.find_things_by_name(&params.room) {
            Ok(things) => things.into_iter().find(|t| t.kind == ThingKind::Room),
            Err(e) => return format!("Error: {}", e),
        };

        let room_thing = match room_thing {
            Some(t) => t,
            None => return format!("Room '{}' not found in things system.", params.room),
        };

        // Find thing by qualified name
        let things = if params.qualified_name.contains('*') {
            match self
                .state
                .db
                .find_things_by_qualified_name(&params.qualified_name)
            {
                Ok(t) => t,
                Err(e) => return format!("Error: {}", e),
            }
        } else {
            match self
                .state
                .db
                .get_thing_by_qualified_name(&params.qualified_name)
            {
                Ok(Some(t)) => vec![t],
                Ok(None) => return format!("Thing '{}' not found.", params.qualified_name),
                Err(e) => return format!("Error: {}", e),
            }
        };

        if things.is_empty() {
            return format!("No things matching '{}'", params.qualified_name);
        }

        // Unequip each thing
        let mut unequipped_count = 0;
        for thing in &things {
            if let Err(e) = self.state.db.unequip(&room_thing.id, &thing.id) {
                return format!("Error unequipping {}: {}", thing.name, e);
            }
            unequipped_count += 1;
        }

        if unequipped_count == 1 {
            let qname = things[0]
                .qualified_name
                .as_deref()
                .unwrap_or(&things[0].name);
            format!("Unequipped {} from {}", qname, params.room)
        } else {
            format!(
                "Unequipped {} things matching '{}' from {}",
                unequipped_count, params.qualified_name, params.room
            )
        }
    }
}

#[tool_handler]
impl ServerHandler for SshwarmaMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "sshwarma MCP server - interact with collaborative rooms. \
                 Use list_rooms to see rooms, get_history to see conversations, \
                 say to send messages, and ask_model to chat with AI models."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Start the MCP server on the given port
pub async fn start_mcp_server(
    port: u16,
    state: Arc<McpServerState>,
) -> Result<tokio::task::JoinHandle<()>> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    info!(port, "MCP server listening");

    let service = StreamableHttpService::new(
        move || Ok(SshwarmaMcpServer::new(state.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!("MCP server error: {}", e);
        }
    });

    Ok(handle)
}
