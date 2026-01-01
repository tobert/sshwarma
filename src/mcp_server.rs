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

use crate::config::Config;
use crate::db::Database;
use crate::llm::LlmClient;
use crate::lua::{LuaRuntime, WrapState};
use crate::model::{ModelBackend, ModelHandle, ModelRegistry};
use crate::state::SharedState;
use crate::world::{JournalKind, World};
use tokio::sync::RwLock;

/// Shared state for the MCP server
pub struct McpServerState {
    pub world: Arc<RwLock<World>>,
    pub db: Arc<Database>,
    pub llm: Arc<LlmClient>,
    pub models: Arc<ModelRegistry>,
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
        use crate::display::{EntryContent, EntrySource};

        let limit = params.limit.unwrap_or(50).min(200);

        match self.state.db.recent_entries(&params.room, limit) {
            Ok(entries) => {
                // Filter to only chat messages
                let messages: Vec<_> = entries
                    .iter()
                    .filter(|e| !e.ephemeral)
                    .filter_map(|entry| {
                        if let EntryContent::Chat(text) = &entry.content {
                            let sender = match &entry.source {
                                EntrySource::User(name) => name.clone(),
                                EntrySource::Model { name, .. } => name.clone(),
                                EntrySource::System => "system".to_string(),
                                EntrySource::Command { command } => format!("/{}", command),
                            };
                            let ts = entry.timestamp.format("%H:%M").to_string();
                            Some((ts, sender, text.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();

                if messages.is_empty() {
                    return format!("No messages in room '{}'.", params.room);
                }

                let mut output = format!(
                    "--- History for {} ({} messages) ---\n",
                    params.room,
                    messages.len()
                );
                for (ts, sender, content) in messages {
                    output.push_str(&format!("[{}] {}: {}\n", ts, sender, content));
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

        // Add message to in-memory room
        use crate::display::{EntryContent, EntryId, EntrySource, LedgerEntry};
        use chrono::Utc;

        let entry = LedgerEntry {
            id: EntryId(0),
            timestamp: Utc::now(),
            source: EntrySource::User(sender.clone()),
            content: EntryContent::Chat(params.message.clone()),
            mutable: false,
            ephemeral: false,
            collapsible: true,
        };

        {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(&params.room) {
                room.add_entry(entry.source.clone(), entry.content.clone());
            }
        }

        // Persist to database
        match self.state.db.add_ledger_entry(&params.room, &entry) {
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

        // Build context from room ledger if provided
        use crate::display::{EntryContent, EntryId, EntrySource, LedgerEntry};
        use chrono::Utc;

        let history = if let Some(ref room_name) = params.room {
            let world = self.state.world.read().await;
            if let Some(room) = world.get_room(room_name) {
                room.ledger
                    .recent(10)
                    .iter()
                    .filter(|e| !e.ephemeral)
                    .filter_map(|entry| match &entry.content {
                        EntryContent::Chat(text) => {
                            let role = match &entry.source {
                                EntrySource::User(_) => "user",
                                EntrySource::Model { .. } => "assistant",
                                EntrySource::System | EntrySource::Command { .. } => return None,
                            };
                            Some((role.to_string(), text.clone()))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
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
                    let entry = LedgerEntry {
                        id: EntryId(0),
                        timestamp: Utc::now(),
                        source: EntrySource::Model {
                            name: model.short_name.clone(),
                            is_streaming: false,
                        },
                        content: EntryContent::Chat(response.clone()),
                        mutable: false,
                        ephemeral: false,
                        collapsible: true,
                    };

                    {
                        let mut world = self.state.world.write().await;
                        if let Some(room) = world.get_room_mut(room_name) {
                            room.add_entry(entry.source.clone(), entry.content.clone());
                        }
                    }
                    let _ = self.state.db.add_ledger_entry(room_name, &entry);
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

        // Create a temporary LuaRuntime for the preview
        let lua_runtime = match LuaRuntime::new() {
            Ok(rt) => rt,
            Err(e) => return format!("Error creating Lua runtime: {}", e),
        };

        // Build SharedState from McpServerState components
        // Note: MCP server doesn't have MCP clients, so we create minimal state
        let shared_state = Arc::new(SharedState {
            world: self.state.world.clone(),
            db: self.state.db.clone(),
            config: Config::default(),
            llm: self.state.llm.clone(),
            models: self.state.models.clone(),
            mcp: Arc::new(crate::mcp::McpManager::new()),
        });

        let wrap_state = WrapState {
            room_name: params.room,
            username,
            model: model.clone(),
            shared_state,
        };

        match lua_runtime.compose_context(wrap_state, target_tokens) {
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
