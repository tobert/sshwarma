//! MCP server for exposing sshwarma tools to Claude Code
//!
//! This module provides an MCP server that allows Claude Code to interact
//! with sshwarma rooms - listing rooms, viewing history, sending messages.

use anyhow::Result;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpService,
        session::local::LocalSessionManager,
    },
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

use crate::db::Database;
use crate::llm::LlmClient;
use crate::model::ModelRegistry;
use crate::world::World;
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
            output.push_str(&format!(
                "- {} ({} users)\n",
                room.name, room.user_count
            ));
        }
        output
    }

    #[tool(description = "Get recent message history from a room")]
    async fn get_history(&self, Parameters(params): Parameters<GetHistoryParams>) -> String {
        let limit = params.limit.unwrap_or(50).min(200);

        match self.state.db.recent_messages(&params.room, limit) {
            Ok(messages) => {
                if messages.is_empty() {
                    return format!("No messages in room '{}'.", params.room);
                }

                let mut output = format!("--- History for {} ({} messages) ---\n", params.room, messages.len());
                for msg in messages {
                    output.push_str(&format!(
                        "[{}] {}: {}\n",
                        &msg.timestamp[11..16],
                        msg.sender_name,
                        msg.content
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

        // Add message to in-memory room
        {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(&params.room) {
                room.add_message(
                    crate::world::Sender::User(sender.clone()),
                    crate::world::MessageContent::Chat(params.message.clone()),
                );
            }
        }

        // Persist to database
        match self.state.db.add_message(&params.room, "user", &sender, "chat", &params.message) {
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
                let available: Vec<_> = self.state.models.available()
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

        // Build context from room history if provided
        let history = if let Some(ref room_name) = params.room {
            let world = self.state.world.read().await;
            if let Some(room) = world.get_room(room_name) {
                room.recent_history(10)
                    .iter()
                    .filter_map(|msg| {
                        match &msg.content {
                            crate::world::MessageContent::Chat(text) => {
                                let role = match &msg.sender {
                                    crate::world::Sender::User(_) => "user",
                                    crate::world::Sender::Model(_) => "assistant",
                                    crate::world::Sender::System => return None,
                                };
                                Some((role.to_string(), text.clone()))
                            }
                            _ => None,
                        }
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

        match self.state.llm.chat_with_context(&model, &system_prompt, &history, &params.message).await {
            Ok(response) => {
                // Record in room if specified
                if let Some(ref room_name) = params.room {
                    {
                        let mut world = self.state.world.write().await;
                        if let Some(room) = world.get_room_mut(room_name) {
                            room.add_message(
                                crate::world::Sender::Model(model.short_name.clone()),
                                crate::world::MessageContent::Chat(response.clone()),
                            );
                        }
                    }
                    let _ = self.state.db.add_message(room_name, "model", &model.short_name, "chat", &response);
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
        if !params.name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return "Room name can only contain letters, numbers, dashes, and underscores.".to_string();
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
        match self.state.db.create_room(&params.name, params.description.as_deref()) {
            Ok(_) => format!("Created room '{}'.", params.name),
            Err(e) => format!("Error: {}", e),
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
