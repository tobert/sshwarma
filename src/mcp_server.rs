//! MCP server for exposing sshwarma tools to Claude Code
//!
//! This module provides an MCP server that allows Claude Code to interact
//! with sshwarma rooms - listing rooms, viewing history, sending messages.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rmcp::{
    model::{
        CallToolRequestParam, CallToolResult, Content, ListToolsResult, PaginatedRequestParam,
        ServerCapabilities, ServerInfo, Tool,
    },
    schemars,
    service::{RequestContext, RoleServer},
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
    ErrorData as McpError, ServerHandler,
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
/// `require(module_path).handler(params)` or `require(module_path).handler_name(params)`
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
    /// Optional handler function name (defaults to "handler")
    /// For multi-tool modules, each tool can specify its own handler function
    pub handler_name: Option<String>,
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
    /// `require(module_path).handler(params)` or `require(module_path).handler_name(params)`
    pub fn dispatch(&self, name: &str, params: Value, lua_runtime: &LuaRuntime) -> Result<String> {
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

        // Get the handler function - use handler_name if specified, otherwise "handler"
        let handler_name = tool.handler_name.as_deref().unwrap_or("handler");
        let handler: mlua::Function = module.get(handler_name).with_context(|| {
            format!(
                "Module '{}' does not have a '{}' function",
                tool.module_path, handler_name
            )
        })?;

        // Convert params to Lua
        let lua_params = json_to_lua(lua, &params).context("Failed to convert params to Lua")?;

        // Call the handler
        let result: mlua::Value = handler
            .call(lua_params)
            .with_context(|| format!("Handler '{}' for '{}' failed", handler_name, name))?;

        // Convert result back to JSON
        let json_result = lua_to_json(&result).context("Failed to convert Lua result to JSON")?;

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
        let lua_runtime =
            LuaRuntime::new().context("Failed to create Lua runtime for MCP session")?;

        // Set up the shared state in the Lua tool state
        lua_runtime
            .tool_state()
            .set_shared_state(Some(shared_state));

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

/// Parameters for identify - set the session's display name
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IdentifyParams {
    #[schemars(
        description = "Display name for this session (e.g., 'claude-code', 'research-agent')"
    )]
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
            self.state
                .db
                .get_room_by_name(name)
                .ok()
                .flatten()
                .map(|r| r.id)
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

    async fn identify(&self, params: IdentifyParams) -> String {
        // Validate the name
        if params.name.is_empty() {
            return "Name cannot be empty.".to_string();
        }
        if !params
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
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
            old_name,
            params.name,
            &session.id[..8]
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
        // Start with Lua-registered tools (these replace most Rust tools)
        let mut tools: Vec<Tool> = self
            .state
            .tool_registry
            .list()
            .into_iter()
            .map(|lua_tool| Tool::new(lua_tool.name, lua_tool.description, lua_tool.schema))
            .collect();

        // Add session-specific tools that remain in Rust
        tools.push(Tool::new(
            "identify",
            "Set your display name for this session. Auto-joins matching room if it exists.",
            generate_schema::<IdentifyParams>(),
        ));
        tools.push(Tool::new(
            "whoami",
            "Get current session identity and status",
            generate_schema::<WhoamiParams>(),
        ));
        tools.push(Tool::new(
            "preview_wrap",
            "Preview what context would be composed for an LLM interaction",
            generate_schema::<PreviewWrapParams>(),
        ));

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
        let params_value = request
            .arguments
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
                    Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {}",
                        e
                    ))]))
                }
            };
        }

        // Dispatch to Rust handlers for session-specific tools only
        // All other tools are handled by Lua (checked above)
        let output = match name {
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
            "preview_wrap" => {
                let p: PreviewWrapParams = serde_json::from_value(params_value)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                self.preview_wrap(p).await
            }
            _ => {
                return Err(McpError::invalid_params(
                    format!("Unknown tool: {}", name),
                    None,
                ))
            }
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
            SshwarmaMcpServer::new(state.clone()).map_err(|e| {
                tracing::error!("Failed to create MCP session: {}", e);
                std::io::Error::other(e.to_string())
            })
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
