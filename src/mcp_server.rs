//! MCP server for exposing sshwarma tools to Claude Code
//!
//! This module provides an MCP server that allows Claude Code to interact
//! with sshwarma rooms - listing rooms, viewing history, sending messages.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rmcp::{
    ErrorData as McpError,
    model::{
        CallToolRequestParam, CallToolResult, Content, ListToolsResult, PaginatedRequestParam,
        ServerCapabilities, ServerInfo, Tool,
    },
    schemars,
    service::{RequestContext, RoleServer},
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
    ServerHandler,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{debug, info, warn};
use uuid::Uuid;

// ScriptScope is imported locally where needed
use crate::db::Database;
use crate::llm::LlmClient;
use crate::lua::{LuaRuntime, WrapState};
use crate::model::{ModelBackend, ModelHandle, ModelRegistry};
use crate::state::SharedState;
use crate::world::World;
use tokio::sync::{Mutex, RwLock};

use rmcp::model::JsonObject;

// =============================================================================
// Lua Tool Registry for MCP Server
// =============================================================================

/// Metadata for a Lua-defined MCP tool
///
/// The handler function is NOT stored here since each MCP session has its own
/// Lua runtime. Instead, we store the module_path and call the handler via:
/// `require(module_path).handler(params)`
#[derive(Debug, Clone)]
pub struct LuaTool {
    /// Tool name exposed via MCP
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// JSON Schema for the tool's parameters
    pub schema: Arc<JsonObject>,
    /// Module path to require (e.g., "mcp.echo_test")
    pub module_path: String,
}

/// Registry of Lua-defined MCP tools
///
/// Thread-safe registry that stores tool metadata. When dispatching,
/// the handler is loaded from the per-session Lua runtime.
pub struct McpToolRegistry {
    lua_tools: std::sync::RwLock<HashMap<String, LuaTool>>,
}

impl McpToolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            lua_tools: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a Lua tool
    pub fn register(&self, tool: LuaTool) {
        let name = tool.name.clone();
        if let Ok(mut tools) = self.lua_tools.write() {
            debug!(name = %name, module_path = %tool.module_path, "Registering Lua MCP tool");
            tools.insert(name, tool);
        }
    }

    /// Check if a tool is registered
    pub fn has(&self, name: &str) -> bool {
        self.lua_tools
            .read()
            .map(|tools| tools.contains_key(name))
            .unwrap_or(false)
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<LuaTool> {
        self.lua_tools.read().ok()?.get(name).cloned()
    }

    /// List all registered Lua tools
    pub fn list(&self) -> Vec<LuaTool> {
        self.lua_tools
            .read()
            .map(|tools| tools.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Dispatch a tool call to Lua
    ///
    /// This loads the handler from the per-session Lua runtime:
    /// `require(module_path).handler(params)`
    pub fn dispatch(
        &self,
        name: &str,
        params: Value,
        lua_runtime: &LuaRuntime,
    ) -> Result<String> {
        use crate::lua::tools::{json_to_lua, lua_to_json};

        let tool = self
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found in registry", name))?;

        let lua = lua_runtime.lua();

        // Load the module: require(module_path)
        let require: mlua::Function = lua
            .globals()
            .get("require")
            .context("Failed to get require function")?;

        let module: mlua::Table = require
            .call(tool.module_path.as_str())
            .with_context(|| format!("Failed to require module '{}'", tool.module_path))?;

        // Get the handler function
        let handler: mlua::Function = module
            .get("handler")
            .with_context(|| format!("Module '{}' does not have a handler function", tool.module_path))?;

        // Convert params to Lua
        let lua_params = json_to_lua(lua, &params)
            .context("Failed to convert params to Lua")?;

        // Call the handler
        let result: mlua::Value = handler
            .call(lua_params)
            .with_context(|| format!("Handler for '{}' failed", name))?;

        // Convert result back to JSON
        let json_result = lua_to_json(&result)
            .context("Failed to convert Lua result to JSON")?;

        // Return as JSON string
        Ok(serde_json::to_string_pretty(&json_result)?)
    }
}

impl Default for McpToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to generate schema without the $schema field
pub fn generate_schema<T: schemars::JsonSchema>() -> Arc<JsonObject> {
    let root = rmcp::schemars::schema_for!(T);
    let mut value = serde_json::to_value(root).unwrap_or(Value::Null);
    if let Some(obj) = value.as_object_mut() {
        obj.remove("$schema");
        Arc::new(obj.clone())
    } else {
        Arc::new(JsonObject::new())
    }
}

/// Get all tool definitions for introspection/Lua access
///
/// Returns the full list of MCP tools with their schemas.
/// Useful for:
/// - Exposing to Lua for context composition
/// - Schema introspection and composition
/// - Building meta-tools that wrap other tools
pub fn get_tool_definitions() -> Vec<Tool> {
    vec![
        Tool::new("list_rooms", "List all available rooms", generate_schema::<ListRoomsParams>()),
        Tool::new("get_history", "Get recent message history from a room", generate_schema::<GetHistoryParams>()),
        Tool::new("say", "Send a message to a room", generate_schema::<SayParams>()),
        Tool::new("ask_model", "Ask a model a question (simple completion, no tools - for full @mention-style interaction with tools, SSH users should type @model in chat)", generate_schema::<AskModelParams>()),
        Tool::new("list_models", "List available AI models", generate_schema::<ListModelsParams>()),
        Tool::new("help", "Get help docs. No topic = list available.", generate_schema::<HelpParams>()),
        Tool::new("create_room", "Create a new room", generate_schema::<CreateRoomParams>()),
        Tool::new("room_context", "Get full room context for agent onboarding - vibe, assets, exits", generate_schema::<RoomContextParams>()),
        Tool::new("set_vibe", "Set the vibe/atmosphere for a room", generate_schema::<SetVibeParams>()),
        Tool::new("add_exit", "Create an exit from one room to another", generate_schema::<AddExitParams>()),
        Tool::new("fork_room", "Fork a room, inheriting its context", generate_schema::<ForkRoomParams>()),
        Tool::new("preview_wrap", "Preview what context would be composed for an LLM interaction", generate_schema::<PreviewWrapParams>()),
        Tool::new("list_scripts", "List available Lua scripts", generate_schema::<ListScriptsParams>()),
        Tool::new("create_script", "Create a new Lua script", generate_schema::<CreateScriptParams>()),
        Tool::new("read_script", "Read a user's Lua UI script by module path", generate_schema::<ReadScriptParams>()),
        Tool::new("update_script", "Update a user's Lua UI script (creates new version via copy-on-write)", generate_schema::<UpdateScriptParams>()),
        Tool::new("delete_script", "Delete a user's Lua UI script (removes all versions)", generate_schema::<DeleteScriptParams>()),
        Tool::new("set_entrypoint", "Set the main UI script entrypoint for a user", generate_schema::<SetEntrypointParams>()),
        Tool::new("inventory_list", "List equipped tools in a room's inventory", generate_schema::<InventoryListParams>()),
        Tool::new("inventory_equip", "Equip a tool in a room", generate_schema::<InventoryEquipParams>()),
        Tool::new("inventory_unequip", "Unequip a tool from a room", generate_schema::<InventoryUnequipParams>()),
        Tool::new("thing_contents", "List contents of a container (things inside rooms, agents, or shared)", generate_schema::<ThingContentsParams>()),
        Tool::new("thing_take", "Copy a thing into your inventory (copy-on-write)", generate_schema::<ThingTakeParams>()),
        Tool::new("thing_drop", "Move a thing from your inventory to a room", generate_schema::<ThingDropParams>()),
        Tool::new("thing_create", "Create a new thing in a container", generate_schema::<ThingCreateParams>()),
        Tool::new("thing_destroy", "Delete a thing (must specify owner:name)", generate_schema::<ThingDestroyParams>()),
        // Session identity tools
        Tool::new("identify", "Set your display name for this session. Auto-joins matching room if it exists.", generate_schema::<IdentifyParams>()),
        Tool::new("whoami", "Get current session identity and status", generate_schema::<WhoamiParams>()),
    ]
}

/// Shared state for the MCP server
pub struct McpServerState {
    pub world: Arc<RwLock<World>>,
    pub db: Arc<Database>,
    pub llm: Arc<LlmClient>,
    pub models: Arc<ModelRegistry>,
    pub lua_runtime: Arc<Mutex<LuaRuntime>>,
    pub shared_state: Arc<SharedState>,
    /// Registry of Lua-defined MCP tools
    pub tool_registry: Arc<McpToolRegistry>,
}

/// Per-connection MCP session state
pub struct McpSession {
    /// Session UUID
    pub id: String,
    /// DB agent record ID
    pub agent_id: String,
    /// Display name - "claude" by default, or set via identify()
    pub display_name: String,
    /// Current room - auto-joins room matching display_name on identify()
    pub current_room: Option<String>,
    /// Per-session Lua runtime
    pub lua_runtime: Arc<Mutex<LuaRuntime>>,
    /// When this session was created
    pub created_at: DateTime<Utc>,
}

impl McpSession {
    /// Create a new MCP session with default "claude" identity
    pub fn new(db: &Database, shared_state: Arc<SharedState>) -> Result<Self> {
        let session_id = Uuid::now_v7().to_string();
        let display_name = "claude".to_string();

        // Get or create the agent record for this session
        let agent = db
            .get_or_create_human_agent(&display_name)
            .context("Failed to get or create agent for MCP session")?;

        // Create a per-session Lua runtime
        let lua_runtime = LuaRuntime::new()
            .context("Failed to create Lua runtime for MCP session")?;

        // Set up the shared state in the Lua tool state
        lua_runtime.tool_state().set_shared_state(Some(shared_state));

        Ok(Self {
            id: session_id,
            agent_id: agent.id,
            display_name,
            current_room: None,
            lua_runtime: Arc::new(Mutex::new(lua_runtime)),
            created_at: Utc::now(),
        })
    }

    /// Update the session identity - returns the old name
    pub fn update_identity(&mut self, db: &Database, new_name: &str) -> Result<String> {
        let old_name = std::mem::replace(&mut self.display_name, new_name.to_string());

        // Get or create agent for the new identity
        let agent = db
            .get_or_create_human_agent(new_name)
            .context("Failed to get or create agent for new identity")?;
        self.agent_id = agent.id;

        Ok(old_name)
    }

    /// Join a room by name
    pub fn join_room(&mut self, db: &Database, room_name: &str) -> Result<()> {
        // Verify the room exists (or could be created)
        let _room = db
            .get_room_by_name(room_name)
            .context("Failed to look up room")?
            .ok_or_else(|| anyhow::anyhow!("Room '{}' does not exist", room_name))?;

        self.current_room = Some(room_name.to_string());
        Ok(())
    }
}

impl crate::ops::MentionSession for McpSession {
    fn agent_id(&self) -> &str {
        &self.agent_id
    }

    fn username(&self) -> &str {
        &self.display_name
    }

    fn current_room(&self) -> Option<String> {
        self.current_room.clone()
    }
}

/// MCP server for sshwarma
#[derive(Clone)]
pub struct SshwarmaMcpServer {
    state: Arc<McpServerState>,
    session: Arc<RwLock<McpSession>>,
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

/// Parameters for help
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct HelpParams {
    #[schemars(description = "Topic (fun, str, inspect, tools, room). Omit for list.")]
    pub topic: Option<String>,
}

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

/// Parameters for read_script (user UI scripts)
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadScriptParams {
    #[schemars(description = "Module path of the script to read (e.g., 'screen', 'ui.status')")]
    pub module_path: String,
}

/// Parameters for update_script (user UI scripts)
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateScriptParams {
    #[schemars(description = "Module path of the script to update")]
    pub module_path: String,
    #[schemars(description = "New Lua source code")]
    pub code: String,
}

/// Parameters for delete_script (user UI scripts)
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteScriptParams {
    #[schemars(description = "Module path of the script to delete")]
    pub module_path: String,
}

/// Parameters for set_entrypoint
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetEntrypointParams {
    #[schemars(description = "Module path to use as main UI entrypoint, or null/empty for default")]
    pub module_path: Option<String>,
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

/// Parameters for thing_contents
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ThingContentsParams {
    #[schemars(description = "Target container: 'shared', room name, or @agent_name")]
    pub target: String,
}

/// Parameters for thing_take
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ThingTakeParams {
    #[schemars(description = "Qualified name or pattern of thing to copy (e.g., 'holler:sample')")]
    pub thing: String,
}

/// Parameters for thing_drop
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ThingDropParams {
    #[schemars(description = "Name of thing in your inventory to drop")]
    pub thing: String,
    #[schemars(description = "Room to drop into (defaults to 'lobby')")]
    pub room: Option<String>,
}

/// Parameters for thing_create
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ThingCreateParams {
    #[schemars(description = "Target container: 'me', room name, 'shared', or @agent_name")]
    pub target: String,
    #[schemars(description = "Name for the new thing")]
    pub name: String,
    #[schemars(description = "Kind: 'data', 'container', or 'tool' (default: 'data')")]
    pub kind: Option<String>,
    #[schemars(description = "Content for data things")]
    pub content: Option<String>,
    #[schemars(description = "Lua code for tool things")]
    pub code: Option<String>,
    #[schemars(description = "Description of the thing")]
    pub description: Option<String>,
}

/// Parameters for thing_destroy
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ThingDestroyParams {
    #[schemars(description = "Owner and thing name: 'me:thing', 'room:thing', '@agent:thing'")]
    pub target: String,
}

/// Parameters for identify - set the session's display name
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IdentifyParams {
    #[schemars(description = "Display name for this session (e.g., 'claude-code', 'research-agent')")]
    pub name: String,
    #[schemars(description = "Optional context about this agent's purpose or capabilities")]
    pub context: Option<String>,
}

/// Parameters for whoami - get current session identity
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WhoamiParams {}

impl SshwarmaMcpServer {
    pub fn new(state: Arc<McpServerState>) -> Result<Self> {
        // Create a per-connection session
        let session = McpSession::new(&state.db, state.shared_state.clone())
            .context("Failed to create MCP session")?;

        Ok(Self {
            state,
            session: Arc::new(RwLock::new(session)),
        })
    }

    async fn list_rooms(&self, _params: ListRoomsParams) -> String {
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

    async fn get_history(&self, params: GetHistoryParams) -> String {
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

    async fn say(&self, params: SayParams) -> String {
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

    async fn ask_model(&self, params: AskModelParams) -> String {
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

    async fn list_models(&self, _params: ListModelsParams) -> String {
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

    async fn help(&self, params: HelpParams) -> String {
        use crate::lua::EmbeddedModules;

        let embedded = EmbeddedModules::new();

        match params.topic {
            Some(topic) => {
                // Look up specific topic
                let doc_name = format!("help.{}", topic);
                match embedded.get(&doc_name) {
                    Some(content) => content.to_string(),
                    None => {
                        // List available topics as fallback
                        let topics: Vec<&str> = embedded
                            .list()
                            .into_iter()
                            .filter(|name| name.starts_with("help."))
                            .map(|name| name.strip_prefix("help.").unwrap_or(name))
                            .collect();
                        format!(
                            "Unknown topic: '{}'. Available: {}",
                            topic,
                            topics.join(", ")
                        )
                    }
                }
            }
            None => {
                // List all topics
                let mut lines = vec!["Available help topics:".to_string(), String::new()];

                let topics = [
                    ("fun", "Functional programming, lazy iterators"),
                    ("str", "String utilities (split, strip, join)"),
                    ("inspect", "Pretty-print tables for debugging"),
                    ("tools", "MCP tool reference and patterns"),
                    ("room", "Room navigation, vibes, exits"),
                ];

                for (name, desc) in topics {
                    lines.push(format!("  {:<10}  {}", name, desc));
                }

                lines.push(String::new());
                lines.push("Usage: help(topic: '<name>')".to_string());
                lines.join("\n")
            }
        }
    }

    async fn create_room(&self, params: CreateRoomParams) -> String {
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

    async fn room_context(&self, params: RoomContextParams) -> String {
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

        if output.trim().is_empty() {
            format!("Room '{}' has no context set.", params.room)
        } else {
            output
        }
    }

    async fn set_vibe(&self, params: SetVibeParams) -> String {
        match self.state.db.set_vibe(&params.room, Some(&params.vibe)) {
            Ok(_) => format!("Set vibe for '{}': {}", params.room, params.vibe),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn add_exit(&self, params: AddExitParams) -> String {
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

    async fn fork_room(&self, params: ForkRoomParams) -> String {
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
                "Forked '{}' from '{}'. Inherited: vibe, tags, assets.",
                params.new_name, params.source
            ),
            Err(e) => format!("Error forking context: {}", e),
        }
    }

    async fn preview_wrap(&self, params: PreviewWrapParams) -> String {
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

        // Look up room_id from room_name
        let room_id = params.room.as_ref().and_then(|name| {
            self.state.db.get_room_by_name(name).ok().flatten().map(|r| r.id)
        });

        // Look up agent_id from username
        let agent_id = match self.state.db.get_or_create_human_agent(&username) {
            Ok(agent) => agent.id,
            Err(e) => return format!("Error getting agent: {}", e),
        };

        // Set session context so tools.history() etc. work
        lua_runtime
            .tool_state()
            .set_session_context(Some(crate::lua::SessionContext {
                agent_id,
                model: Some(model.clone()),
                room_id,
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

    async fn list_scripts(&self, _params: ListScriptsParams) -> String {
        use crate::db::scripts::ScriptScope;

        // List system scripts for now (room scripts require a room context)
        match self.state.db.list_scripts(ScriptScope::System, None) {
            Ok(scripts) => {
                if scripts.is_empty() {
                    return "No system scripts found.".to_string();
                }

                let mut output = "## Available Scripts\n\n".to_string();
                for script in scripts {
                    let desc = script.description.as_deref().unwrap_or("No description");
                    output.push_str(&format!("- **{}** ({}): {}\n", script.module_path, script.scope.as_str(), desc));
                }
                output
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn create_script(&self, params: CreateScriptParams) -> String {
        use crate::db::scripts::ScriptScope;

        // Parse kind to determine scope (for backwards compatibility)
        // handler/renderer/transformer all become system scope scripts with the module_path as name
        let scope = match params.kind.as_str() {
            "handler" | "renderer" | "transformer" => ScriptScope::System,
            _ => {
                return format!(
                    "Invalid kind '{}'. Use: handler, renderer, transformer",
                    params.kind
                )
            }
        };

        match self.state.db.create_script(
            scope,
            None,  // scope_id - None for system scope
            &params.name,  // module_path
            &params.code,
            "mcp",  // created_by
        ) {
            Ok(id) => format!("Created script '{}' with id {}.", params.name, &id[..8]),
            Err(e) => format!("Error creating script: {}", e),
        }
    }

    // =========================================================================
    // User UI Script Management
    // These tools operate on user-scoped scripts for UI customization
    // =========================================================================

    async fn read_script(&self, params: ReadScriptParams) -> String {
        use crate::db::scripts::ScriptScope;

        // For now, read scripts from the "claude" user scope
        // In a real implementation, this would use the authenticated user
        let username = "claude";

        match self.state.db.get_current_script(ScriptScope::User, Some(username), &params.module_path) {
            Ok(Some(script)) => {
                format!(
                    "## Script: {}\n\nModule: `{}`\nScope: user:{}\nVersion: {}\n\n```lua\n{}\n```",
                    params.module_path,
                    script.module_path,
                    username,
                    &script.id[..8],
                    script.code
                )
            }
            Ok(None) => format!("No script found at module path '{}'.", params.module_path),
            Err(e) => format!("Error reading script: {}", e),
        }
    }

    async fn update_script(&self, params: UpdateScriptParams) -> String {
        use crate::db::scripts::ScriptScope;

        let username = "claude";

        // Check if script exists
        let existing = match self.state.db.get_current_script(ScriptScope::User, Some(username), &params.module_path) {
            Ok(Some(s)) => s,
            Ok(None) => {
                // Script doesn't exist, create it
                match self.state.db.create_script(
                    ScriptScope::User,
                    Some(username),
                    &params.module_path,
                    &params.code,
                    "claude",
                ) {
                    Ok(id) => return format!("Created new script '{}' (id: {}).", params.module_path, &id[..8]),
                    Err(e) => return format!("Error creating script: {}", e),
                }
            }
            Err(e) => return format!("Error checking for existing script: {}", e),
        };

        // Update existing script (CoW)
        match self.state.db.update_script(&existing.id, &params.code, "claude") {
            Ok(new_id) => format!(
                "Updated script '{}' (new version: {}, previous: {}).",
                params.module_path,
                &new_id[..8],
                &existing.id[..8]
            ),
            Err(e) => format!("Error updating script: {}", e),
        }
    }

    async fn delete_script(&self, params: DeleteScriptParams) -> String {
        use crate::db::scripts::ScriptScope;

        let username = "claude";

        match self.state.db.delete_script(ScriptScope::User, Some(username), &params.module_path) {
            Ok(count) if count > 0 => format!(
                "Deleted script '{}' ({} version{}).",
                params.module_path,
                count,
                if count == 1 { "" } else { "s" }
            ),
            Ok(_) => format!("No script found at module path '{}'.", params.module_path),
            Err(e) => format!("Error deleting script: {}", e),
        }
    }

    async fn set_entrypoint(&self, params: SetEntrypointParams) -> String {
        let username = "claude";

        // Empty string treated as None (reset to default)
        let entrypoint = params.module_path.as_deref().filter(|s| !s.is_empty());

        match self.state.db.set_user_entrypoint(username, entrypoint) {
            Ok(()) => {
                if let Some(ep) = entrypoint {
                    format!("Set UI entrypoint to '{}'. Use /reload to apply.", ep)
                } else {
                    "Reset to default UI. Use /reload to apply.".to_string()
                }
            }
            Err(e) => format!("Error setting entrypoint: {}", e),
        }
    }

    // =========================================================================
    // Inventory tools (things system)
    // =========================================================================

    async fn inventory_list(&self, params: InventoryListParams) -> String {
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
                // Look up the actual Room to get its ID
                let room = match self.state.db.get_room_by_name(&params.room) {
                    Ok(Some(r)) => r,
                    Ok(None) => {
                        // Also check World (in-memory)
                        let world = self.state.world.read().await;
                        if world.get_room(&params.room).is_none() {
                            return format!("Room '{}' does not exist.", params.room);
                        }
                        drop(world);
                        // Create room in DB if only in World
                        if let Err(e) = self.state.db.create_room(&params.room, None) {
                            return format!("Error creating room: {}", e);
                        }
                        match self.state.db.get_room_by_name(&params.room) {
                            Ok(Some(r)) => r,
                            _ => return format!("Failed to get room '{}'", params.room),
                        }
                    }
                    Err(e) => return format!("Error: {}", e),
                };

                // Create thing for it with same ID as the Room
                let mut new_room =
                    crate::db::things::Thing::room(&params.room).with_parent("rooms");
                new_room.id = room.id.clone();
                if let Err(e) = self.state.db.insert_thing(&new_room) {
                    // Might already exist, which is fine
                    if !e.to_string().contains("UNIQUE constraint") {
                        return format!("Error creating room thing: {}", e);
                    }
                }
                // Copy equipment from lobby (which has internal tools)
                if let Ok(Some(lobby)) = self.state.db.get_room_by_name("lobby") {
                    if let Err(e) = self.state.db.copy_room_equipment(&lobby.id, &room.id) {
                        return format!("Error copying equipment: {}", e);
                    }
                }
                new_room
            }
        };

        // Get equipped tools
        let equipped = match self.state.db.get_room_equipment_tools(&room_thing.id) {
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

    async fn inventory_equip(
        &self,
        params: InventoryEquipParams,
    ) -> String {
        // Ensure world is bootstrapped
        if let Err(e) = self.state.db.bootstrap_world() {
            return format!("Error: {}", e);
        }

        // Get the room ID from the rooms table
        let room = match self.state.db.get_room_by_name(&params.room) {
            Ok(Some(r)) => r,
            Ok(None) => return format!("Room '{}' does not exist.", params.room),
            Err(e) => return format!("Error: {}", e),
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
            if let Err(e) = self.state.db.room_equip(&room.id, &thing.id, None, None, priority) {
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

    async fn inventory_unequip(
        &self,
        params: InventoryUnequipParams,
    ) -> String {
        // Get the room ID from the rooms table
        let room = match self.state.db.get_room_by_name(&params.room) {
            Ok(Some(r)) => r,
            Ok(None) => return format!("Room '{}' does not exist.", params.room),
            Err(e) => return format!("Error: {}", e),
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
            if let Err(e) = self.state.db.room_unequip(&room.id, &thing.id, None) {
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

    // =========================================================================
    // Containment tools (things hierarchy)
    // =========================================================================

    /// Resolve target string to parent_id for containment operations
    fn resolve_containment_target(&self, target: &str) -> Result<String, String> {
        use crate::db::things::ids;

        if target == "me" {
            // For MCP, "me" = agent_claude - ensure it exists
            self.state
                .db
                .ensure_agent_thing("claude")
                .map_err(|e| format!("Failed to ensure agent: {}", e))?;
            Ok("agent_claude".to_string())
        } else if target == "shared" || target == "world" {
            Ok(ids::SHARED.to_string())
        } else if let Some(agent_name) = target.strip_prefix('@') {
            // Ensure agent thing exists
            self.state
                .db
                .ensure_agent_thing(agent_name)
                .map_err(|e| format!("Failed to ensure agent: {}", e))?;
            Ok(format!("agent_{}", agent_name))
        } else {
            // Assume it's a room name - look up the room
            match self.state.db.get_room_by_name(target) {
                Ok(Some(room)) => Ok(room.id),
                Ok(None) => Err(format!("Room '{}' not found", target)),
                Err(e) => Err(format!("Error looking up room: {}", e)),
            }
        }
    }

    async fn thing_contents(&self, params: ThingContentsParams) -> String {
        let parent_id = match self.resolve_containment_target(&params.target) {
            Ok(id) => id,
            Err(e) => return e,
        };

        let children = match self.state.db.get_thing_children(&parent_id) {
            Ok(c) => c,
            Err(e) => return format!("Error: {}", e),
        };

        let title = if params.target == "me" {
            "Your Inventory".to_string()
        } else if params.target == "shared" || params.target == "world" {
            "Shared Resources".to_string()
        } else if params.target.starts_with('@') {
            format!("{}'s Inventory", params.target)
        } else {
            format!("Contents of room '{}'", params.target)
        };

        let mut output = format!("{}:\n", title);

        if children.is_empty() {
            output.push_str("  (empty)\n");
        } else {
            for thing in children {
                let icon = match thing.kind {
                    crate::db::things::ThingKind::Container => "[+]",
                    crate::db::things::ThingKind::Tool => " ⚙ ",
                    _ => " - ",
                };
                let name = thing.qualified_name.as_deref().unwrap_or(&thing.name);
                output.push_str(&format!("  {} {}\n", icon, name));
            }
        }

        output
    }

    async fn thing_take(&self, params: ThingTakeParams) -> String {
        // MCP agent is always "claude"
        let agent_thing_id = "agent_claude".to_string();

        // Ensure agent thing exists
        if let Err(e) = self.state.db.ensure_agent_thing("claude") {
            return format!("Error ensuring agent: {}", e);
        }

        // Find the thing to take
        let thing = match self.state.db.get_thing_by_qualified_name(&params.thing) {
            Ok(Some(t)) => t,
            Ok(None) => {
                // Try pattern search
                match self.state.db.find_things_by_qualified_name(&params.thing) {
                    Ok(things) if things.len() == 1 => things.into_iter().next().unwrap(),
                    Ok(things) if things.len() > 1 => {
                        return format!("Ambiguous: {} matches for '{}'", things.len(), params.thing)
                    }
                    _ => return format!("Thing '{}' not found", params.thing),
                }
            }
            Err(e) => return format!("Error: {}", e),
        };

        // Copy the thing
        match self.state.db.copy_thing(&thing.id, &agent_thing_id) {
            Ok(copy) => {
                let name = thing.qualified_name.as_deref().unwrap_or(&thing.name);
                format!("Took {} (copy id: {})", name, &copy.id[..8])
            }
            Err(e) => format!("Error taking thing: {}", e),
        }
    }

    async fn thing_drop(&self, params: ThingDropParams) -> String {
        let agent_thing_id = "agent_claude".to_string();
        let room_name = params.room.as_deref().unwrap_or("lobby");

        // Get room ID
        let room_id = match self.state.db.get_room_by_name(room_name) {
            Ok(Some(r)) => r.id,
            Ok(None) => return format!("Room '{}' not found", room_name),
            Err(e) => return format!("Error: {}", e),
        };

        // Find thing in agent's inventory
        let children = match self.state.db.get_thing_children(&agent_thing_id) {
            Ok(c) => c,
            Err(e) => return format!("Error listing inventory: {}", e),
        };

        let thing = children.into_iter().find(|t| {
            t.name == params.thing
                || t.qualified_name.as_deref() == Some(&params.thing)
        });

        let thing = match thing {
            Some(t) => t,
            None => return format!("'{}' not in your inventory", params.thing),
        };

        // Move it
        match self.state.db.move_thing(&thing.id, &room_id) {
            Ok(()) => {
                let name = thing.qualified_name.as_deref().unwrap_or(&thing.name);
                format!("Dropped {} into {}", name, room_name)
            }
            Err(e) => format!("Error dropping: {}", e),
        }
    }

    async fn thing_create(&self, params: ThingCreateParams) -> String {
        use crate::db::things::{Thing, ThingKind};

        // Resolve target to parent_id
        let parent_id = match self.resolve_containment_target(&params.target) {
            Ok(id) => id,
            Err(e) => return e,
        };

        // Parse kind
        let kind = match params.kind.as_deref().unwrap_or("data") {
            "data" => ThingKind::Data,
            "container" => ThingKind::Container,
            "tool" => ThingKind::Tool,
            other => return format!("Invalid kind '{}'. Use: data, container, tool", other),
        };

        // Generate qualified name
        let qualified_name = if params.name.contains(':') {
            params.name.clone()
        } else {
            format!("claude:{}", params.name)
        };

        // Create the thing
        let mut thing = Thing::new(&params.name, kind);
        thing.parent_id = Some(parent_id);
        thing.qualified_name = Some(qualified_name.clone());
        thing.description = params.description.clone();
        thing.content = params.content.clone();
        thing.code = params.code.clone();

        match self.state.db.insert_thing(&thing) {
            Ok(()) => {
                let target_desc = if params.target == "me" {
                    "your inventory".to_string()
                } else if params.target.starts_with('@') {
                    format!("{}'s inventory", params.target)
                } else {
                    params.target.clone()
                };
                format!("Created {} in {} (id: {})", qualified_name, target_desc, &thing.id[..8])
            }
            Err(e) => format!("Error creating thing: {}", e),
        }
    }

    async fn thing_destroy(&self, params: ThingDestroyParams) -> String {
        // Parse owner:thing format
        let parts: Vec<&str> = params.target.splitn(2, ':').collect();
        if parts.len() != 2 {
            return "Must specify owner:thing (e.g., 'me:old-note', '@claude:test')".to_string();
        }

        let owner = parts[0];
        let thing_name = parts[1];

        // Resolve owner to parent_id
        let parent_id = match self.resolve_containment_target(owner) {
            Ok(id) => id,
            Err(e) => return e,
        };

        // Find thing under owner
        let children = match self.state.db.get_thing_children(&parent_id) {
            Ok(c) => c,
            Err(e) => return format!("Error: {}", e),
        };

        let thing = children.into_iter().find(|t| {
            t.name == thing_name || t.qualified_name.as_deref() == Some(thing_name)
        });

        let thing = match thing {
            Some(t) => t,
            None => return format!("'{}' not found under '{}'", thing_name, owner),
        };

        // Soft-delete it
        match self.state.db.soft_delete_thing(&thing.id) {
            Ok(()) => format!("Destroyed {}", thing_name),
            Err(e) => format!("Error destroying: {}", e),
        }
    }

    // =========================================================================
    // Session identity tools
    // =========================================================================

    async fn identify(&self, params: IdentifyParams) -> String {
        // Validate the name
        if params.name.is_empty() {
            return "Name cannot be empty.".to_string();
        }
        if !params.name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return "Name can only contain letters, numbers, dashes, and underscores.".to_string();
        }

        let mut session = self.session.write().await;
        let old_name = match session.update_identity(&self.state.db, &params.name) {
            Ok(old) => old,
            Err(e) => return format!("Error updating identity: {}", e),
        };

        // Try to auto-join a room matching the new name
        let auto_joined = if let Ok(Some(_room)) = self.state.db.get_room_by_name(&params.name) {
            session.current_room = Some(params.name.clone());
            true
        } else {
            false
        };

        // Build response
        let mut response = format!(
            "Identity updated: {} -> {}\nSession ID: {}",
            old_name, params.name, &session.id[..8]
        );

        if auto_joined {
            response.push_str(&format!("\nAuto-joined room: {}", params.name));
        }

        if let Some(ref context) = params.context {
            response.push_str(&format!("\nContext: {}", context));
        }

        response
    }

    async fn whoami(&self, _params: WhoamiParams) -> String {
        let session = self.session.read().await;

        let uptime = Utc::now().signed_duration_since(session.created_at);
        let uptime_str = if uptime.num_hours() > 0 {
            format!("{}h {}m", uptime.num_hours(), uptime.num_minutes() % 60)
        } else if uptime.num_minutes() > 0 {
            format!("{}m {}s", uptime.num_minutes(), uptime.num_seconds() % 60)
        } else {
            format!("{}s", uptime.num_seconds())
        };

        let mut output = format!(
            "Session Identity:\n\
             - Display Name: {}\n\
             - Session ID: {}\n\
             - Agent ID: {}\n\
             - Uptime: {}\n\
             - Created: {}",
            session.display_name,
            &session.id[..8],
            &session.agent_id[..8],
            uptime_str,
            session.created_at.format("%Y-%m-%d %H:%M:%S UTC")
        );

        if let Some(ref room) = session.current_room {
            output.push_str(&format!("\n- Current Room: {}", room));
        } else {
            output.push_str("\n- Current Room: (none)");
        }

        output
    }
}

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

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // Get built-in tools
        let mut tools = get_tool_definitions();

        // Add Lua-registered tools from the registry
        for lua_tool in self.state.tool_registry.list() {
            tools.push(Tool::new(
                lua_tool.name,
                lua_tool.description,
                lua_tool.schema,
            ));
        }

        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.as_ref();
        let params_value = request.arguments
            .map(Value::Object)
            .unwrap_or(Value::Object(serde_json::Map::new()));

        // Check if this is a Lua-registered tool first
        if self.state.tool_registry.has(name) {
            debug!(tool = %name, "Dispatching Lua MCP tool");

            // Get the per-session Lua runtime
            let session = self.session.read().await;
            let lua_runtime = session.lua_runtime.clone();

            // Dispatch using block_in_place since Lua is not async
            let registry = self.state.tool_registry.clone();
            let tool_name = name.to_string();
            let params = params_value.clone();

            let result = tokio::task::block_in_place(|| {
                let runtime = lua_runtime.blocking_lock();
                registry.dispatch(&tool_name, params, &runtime)
            });

            return match result {
                Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                Err(e) => {
                    warn!(tool = %name, error = %e, "Lua tool dispatch failed");
                    Ok(CallToolResult::error(vec![Content::text(format!("Error: {}", e))]))
                }
            };
        }

        let output = match name {
            "list_rooms" => {
                let p: ListRoomsParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.list_rooms(p).await
            }
            "get_history" => {
                let p: GetHistoryParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.get_history(p).await
            }
            "say" => {
                let p: SayParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.say(p).await
            }
            "ask_model" => {
                let p: AskModelParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.ask_model(p).await
            }
            "list_models" => {
                let p: ListModelsParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.list_models(p).await
            }
            "help" => {
                let p: HelpParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.help(p).await
            }
            "create_room" => {
                let p: CreateRoomParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.create_room(p).await
            }
            "room_context" => {
                let p: RoomContextParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.room_context(p).await
            }
            "set_vibe" => {
                let p: SetVibeParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.set_vibe(p).await
            }
            "add_exit" => {
                let p: AddExitParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.add_exit(p).await
            }
            "fork_room" => {
                let p: ForkRoomParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.fork_room(p).await
            }
            "preview_wrap" => {
                let p: PreviewWrapParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.preview_wrap(p).await
            }
            "list_scripts" => {
                let p: ListScriptsParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.list_scripts(p).await
            }
            "create_script" => {
                let p: CreateScriptParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.create_script(p).await
            }
            "read_script" => {
                let p: ReadScriptParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.read_script(p).await
            }
            "update_script" => {
                let p: UpdateScriptParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.update_script(p).await
            }
            "delete_script" => {
                let p: DeleteScriptParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.delete_script(p).await
            }
            "set_entrypoint" => {
                let p: SetEntrypointParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.set_entrypoint(p).await
            }
            "inventory_list" => {
                let p: InventoryListParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.inventory_list(p).await
            }
            "inventory_equip" => {
                let p: InventoryEquipParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.inventory_equip(p).await
            }
            "inventory_unequip" => {
                let p: InventoryUnequipParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.inventory_unequip(p).await
            }
            "thing_contents" => {
                let p: ThingContentsParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.thing_contents(p).await
            }
            "thing_take" => {
                let p: ThingTakeParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.thing_take(p).await
            }
            "thing_drop" => {
                let p: ThingDropParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.thing_drop(p).await
            }
            "thing_create" => {
                let p: ThingCreateParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.thing_create(p).await
            }
            "thing_destroy" => {
                let p: ThingDestroyParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.thing_destroy(p).await
            }
            "identify" => {
                let p: IdentifyParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.identify(p).await
            }
            "whoami" => {
                let p: WhoamiParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.whoami(p).await
            }
            _ => return Err(McpError::invalid_params(format!("Unknown tool: {}", name), None)),
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
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
        move || {
            Ok(SshwarmaMcpServer::new(state.clone())
                .expect("Failed to create MCP session - this is a fatal error"))
        },
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
