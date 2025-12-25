//! sshwarma - SSH-accessible partyline for humans and models
//!
//! A MUD-style REPL where users connect via SSH and collaborate with
//! AI models in shared "partylines" (rooms). Plain text is chat,
//! /commands control navigation and tools, @mentions address models.

use anyhow::{Context, Result};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId, CryptoVec};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use sshwarma::db::Database;
use sshwarma::llm::LlmClient;
use sshwarma::mcp::McpClients;
use sshwarma::model::ModelRegistry;
use sshwarma::player::PlayerSession;
use sshwarma::world::World;

/// Server configuration
#[derive(Clone)]
pub struct Config {
    /// SSH listen address
    pub listen_addr: SocketAddr,
    /// Path to server host key
    pub host_key_path: String,
    /// Path to sqlite database
    pub db_path: String,
    /// llama.cpp endpoint
    pub llm_endpoint: String,
    /// MCP server endpoints (holler, exa, etc.)
    pub mcp_endpoints: Vec<String>,
    /// Allow any key when no users registered (dev mode)
    pub allow_open_registration: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:2222".parse().unwrap(),
            host_key_path: "host_key".to_string(),
            db_path: "sshwarma.db".to_string(),
            llm_endpoint: "http://localhost:2020".to_string(),
            mcp_endpoints: vec!["http://localhost:8080/mcp".to_string()],
            allow_open_registration: true,
        }
    }
}

/// The shared world state
pub struct SharedState {
    pub world: RwLock<World>,
    pub db: Database,
    pub config: Config,
    pub llm: LlmClient,
    pub models: ModelRegistry,
    pub mcp: McpClients,
}

/// SSH server implementation
#[derive(Clone)]
struct SshServer {
    state: Arc<SharedState>,
}

impl server::Server for SshServer {
    type Handler = SshHandler;

    fn new_client(&mut self, peer_addr: Option<SocketAddr>) -> Self::Handler {
        info!(?peer_addr, "new connection");
        SshHandler {
            state: self.state.clone(),
            player: None,
            line_buffer: String::new(),
        }
    }

    fn handle_session_error(&mut self, error: <Self::Handler as server::Handler>::Error) {
        tracing::error!("session error: {:?}", error);
    }
}

/// Per-connection SSH handler
struct SshHandler {
    state: Arc<SharedState>,
    player: Option<PlayerSession>,
    line_buffer: String,
}

impl server::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        // Check if this key is registered
        let key_str = key.to_string();

        if let Ok(Some(handle)) = self.state.db.lookup_handle_by_pubkey(&key_str) {
            info!(handle, "key recognized");
            Ok(server::Auth::Accept)
        } else if self.state.config.allow_open_registration {
            // Dev mode: allow unregistered keys
            info!("unregistered key, allowing in dev mode");
            Ok(server::Auth::Accept)
        } else {
            warn!("unknown key rejected");
            Ok(server::Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            })
        }
    }

    async fn auth_publickey(
        &mut self,
        ssh_user: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        let key_str = key.to_string();

        // Look up handle by key, fall back to SSH username
        let handle = self
            .state
            .db
            .lookup_handle_by_pubkey(&key_str)
            .ok()
            .flatten()
            .unwrap_or_else(|| ssh_user.to_string());

        info!(handle, "authenticated");

        // Update last_seen
        let _ = self.state.db.touch_user(&handle);

        // Create player session and record in database
        let player = PlayerSession::new(handle.clone());
        let _ = self.state.db.start_session(&player.session_id, &handle);
        self.player = Some(player);
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Send welcome message
        if let Some(ref player) = self.player {
            let welcome = format!(
                "\x1b[1;36m╭─────────────────────────────────────╮\x1b[0m\r\n\
                 \x1b[1;36m│           sshwarma                  │\x1b[0m\r\n\
                 \x1b[1;36m╰─────────────────────────────────────╯\x1b[0m\r\n\r\n\
                 Welcome, {}.\r\n\r\n\
                 /rooms to list partylines, /join <room> to enter\r\n\r\n\
                 \x1b[33mlobby>\x1b[0m ",
                player.username
            );
            let _ = session.data(channel, CryptoVec::from(welcome.as_bytes()));
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Handle incoming data byte by byte for line editing
        for &byte in data {
            match byte {
                // Enter/Return
                b'\r' | b'\n' => {
                    if !self.line_buffer.is_empty() {
                        let input = std::mem::take(&mut self.line_buffer);
                        let response = self.handle_input(&input).await;
                        let prompt = self
                            .player
                            .as_ref()
                            .map(|p| p.prompt())
                            .unwrap_or_else(|| "lobby>".to_string());
                        let output = format!("\r\n{}\r\n\x1b[33m{}\x1b[0m ", response, prompt);
                        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                    }
                }
                // Backspace
                127 | 8 => {
                    if !self.line_buffer.is_empty() {
                        self.line_buffer.pop();
                        // Echo backspace
                        let _ = session.data(channel, CryptoVec::from(b"\x08 \x08".as_slice()));
                    }
                }
                // Printable characters
                32..=126 => {
                    self.line_buffer.push(byte as char);
                    // Echo character
                    let _ = session.data(channel, CryptoVec::from([byte].as_slice()));
                }
                // Ignore other control characters
                _ => {}
            }
        }
        Ok(())
    }
}

impl SshHandler {
    async fn handle_input(&mut self, input: &str) -> String {
        let input = input.trim();

        if input.starts_with('/') {
            // Command
            let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
            let cmd = parts.first().unwrap_or(&"");
            let args = parts.get(1).copied().unwrap_or("");

            match *cmd {
                "help" => self.cmd_help(),
                "rooms" => self.cmd_rooms().await,
                "who" => self.cmd_who().await,
                "join" => self.cmd_join(args).await,
                "create" => self.cmd_create(args).await,
                "leave" => self.cmd_leave().await,
                "look" => self.cmd_look(args).await,
                "history" => self.cmd_history(args).await,
                "tools" => self.cmd_tools().await,
                "run" => self.cmd_run(args).await,
                "mcp" => self.cmd_mcp(args).await,
                "quit" => "Goodbye!".to_string(),
                _ => format!("Unknown command: /{}", cmd),
            }
        } else if input.starts_with('@') {
            // Model mention - talk to an LLM
            self.cmd_mention(input).await
        } else {
            // Chat message - record in room if in one
            self.cmd_say(input).await
        }
    }

    fn cmd_help(&self) -> String {
        r#"
Navigation:
  /rooms              List partylines
  /join <room>        Enter a partyline
  /leave              Return to lobby
  /create <name>      New partyline

Looking:
  /look               Room summary
  /look <thing>       Examine artifact/user/model
  /who                Who's online
  /history [n]        Recent messages

Communication:
  <text>              Say to room
  @model <msg>        Message a model

Tools:
  /tools              List available tools
  /run <tool> [args]  Invoke tool with JSON args

MCP:
  /mcp                List connected MCP servers
  /mcp connect <name> <url>  Connect to MCP server
  /mcp disconnect <name>     Disconnect from server
  /mcp refresh <name>        Refresh tool list

/quit to disconnect
"#
        .to_string()
    }

    async fn cmd_rooms(&self) -> String {
        let world = self.state.world.read().await;
        let rooms = world.list_rooms();
        if rooms.is_empty() {
            "No partylines yet. /create <name> to start one.".to_string()
        } else {
            let mut out = "Partylines:\r\n".to_string();
            for room in rooms {
                out.push_str(&format!("  {} ... {} users\r\n", room.name, room.user_count));
            }
            out
        }
    }

    async fn cmd_who(&self) -> String {
        if let Some(ref player) = self.player {
            if let Some(ref room_name) = player.current_room {
                let world = self.state.world.read().await;
                if let Some(room) = world.get_room(room_name) {
                    let users: Vec<&str> = room.users.iter().map(|s| s.as_str()).collect();
                    return format!("In {}: {}", room_name, users.join(", "));
                }
            }
        }
        "Online: you (more coming soon)".to_string()
    }

    async fn cmd_join(&mut self, args: &str) -> String {
        let room_name = args.trim();
        if room_name.is_empty() {
            return "Usage: /join <room>".to_string();
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        // Leave current room if in one
        if let Some(ref current) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(&current) {
                room.remove_user(&username);
            }
        }

        // Check if room exists
        {
            let world = self.state.world.read().await;
            if world.get_room(room_name).is_none() {
                return format!("No partyline named '{}'. Use /create {} to make one.", room_name, room_name);
            }
        }

        // Join the room
        {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(room_name) {
                room.add_user(username.clone());
            }
        }

        // Update player state and session
        if let Some(ref mut player) = self.player {
            player.join_room(room_name.to_string());
            let _ = self.state.db.update_session_room(&player.session_id, Some(room_name));
        }

        // Build output with room summary and history
        let mut output = self.cmd_look("").await;

        // Load and display recent history from database
        if let Ok(messages) = self.state.db.recent_messages(room_name, 20) {
            if !messages.is_empty() {
                output.push_str("\r\n\r\n─── Recent History ───\r\n");
                for msg in messages {
                    let line = format!(
                        "[{}] {}: {}\r\n",
                        &msg.timestamp[11..16], // Just HH:MM
                        msg.sender_name,
                        msg.content
                    );
                    output.push_str(&line);
                }
                output.push_str("──────────────────────\r\n");
            }
        }

        output
    }

    async fn cmd_create(&mut self, args: &str) -> String {
        let room_name = args.trim();
        if room_name.is_empty() {
            return "Usage: /create <name>".to_string();
        }

        // Validate room name (alphanumeric, dashes, underscores)
        if !room_name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return "Room name can only contain letters, numbers, dashes, and underscores.".to_string();
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        // Check if room already exists
        {
            let world = self.state.world.read().await;
            if world.get_room(room_name).is_some() {
                return format!("Partyline '{}' already exists. Use /join {} to enter.", room_name, room_name);
            }
        }

        // Leave current room if in one
        if let Some(ref current) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(&current) {
                room.remove_user(&username);
            }
        }

        // Create and join the room
        {
            let mut world = self.state.world.write().await;
            world.create_room(room_name.to_string());
            if let Some(room) = world.get_room_mut(room_name) {
                room.add_user(username.clone());
            }
        }

        // Persist to database
        let _ = self.state.db.create_room(room_name, None);

        // Update player state
        if let Some(ref mut player) = self.player {
            player.join_room(room_name.to_string());
        }

        format!("Created partyline '{}'.\r\n\r\n{}", room_name, self.cmd_look("").await)
    }

    async fn cmd_leave(&mut self) -> String {
        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        let current_room = match &self.player {
            Some(p) => p.current_room.clone(),
            None => None,
        };

        match current_room {
            Some(room_name) => {
                // Remove from room
                {
                    let mut world = self.state.world.write().await;
                    if let Some(room) = world.get_room_mut(&room_name) {
                        room.remove_user(&username);
                    }
                }

                // Update player state and session
                if let Some(ref mut player) = self.player {
                    player.leave_room();
                    let _ = self.state.db.update_session_room(&player.session_id, None);
                }

                format!("Left '{}'.\r\nYou are now in the lobby.", room_name)
            }
            None => "You're already in the lobby.".to_string(),
        }
    }

    async fn cmd_look(&self, args: &str) -> String {
        let args = args.trim();

        // If args provided, look at specific thing (future: artifacts, users, models)
        if !args.is_empty() {
            return format!("You look at '{}'. (detailed inspection coming soon)", args);
        }

        // Look at current room
        match &self.player {
            Some(player) => {
                match &player.current_room {
                    Some(room_name) => {
                        let world = self.state.world.read().await;
                        if let Some(room) = world.get_room(room_name) {
                            let mut out = format!("═══ {} ═══\r\n", room_name);
                            if let Some(ref desc) = room.description {
                                out.push_str(&format!("{}\r\n", desc));
                            }
                            out.push_str("\r\n");

                            // Users
                            if room.users.is_empty() {
                                out.push_str("Nobody else is here.\r\n");
                            } else {
                                out.push_str(&format!("Users: {}\r\n", room.users.join(", ")));
                            }

                            // Models (lurking)
                            if !room.models.is_empty() {
                                let model_names: Vec<_> = room.models.iter().map(|m| m.short_name.as_str()).collect();
                                out.push_str(&format!("Models: {}\r\n", model_names.join(", ")));
                            }

                            // Artifacts
                            if !room.artifacts.is_empty() {
                                out.push_str(&format!("Artifacts: {} items\r\n", room.artifacts.len()));
                            }

                            out
                        } else {
                            "Room not found.".to_string()
                        }
                    }
                    None => {
                        "═══ Lobby ═══\r\n\r\nYou're in the lobby. Use /rooms to see partylines, /join <room> to enter one.".to_string()
                    }
                }
            }
            None => "Not authenticated".to_string(),
        }
    }

    async fn cmd_history(&self, args: &str) -> String {
        let limit: usize = args.trim().parse().unwrap_or(50);
        let limit = limit.min(200); // Cap at 200

        let room_name = match &self.player {
            Some(p) => match &p.current_room {
                Some(r) => r.clone(),
                None => return "You need to be in a room to see history.".to_string(),
            },
            None => return "Not authenticated".to_string(),
        };

        match self.state.db.recent_messages(&room_name, limit) {
            Ok(messages) => {
                if messages.is_empty() {
                    "No messages in this room yet.".to_string()
                } else {
                    let mut output = format!("─── Last {} messages in {} ───\r\n", messages.len(), room_name);
                    for msg in messages {
                        let line = format!(
                            "[{}] {}: {}\r\n",
                            &msg.timestamp[11..16], // Just HH:MM
                            msg.sender_name,
                            msg.content
                        );
                        output.push_str(&line);
                    }
                    output.push_str("──────────────────────────────\r\n");
                    output
                }
            }
            Err(e) => format!("Error loading history: {}", e),
        }
    }

    async fn cmd_say(&mut self, message: &str) -> String {
        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        let current_room = self.player.as_ref().and_then(|p| p.current_room.clone());

        match current_room {
            Some(room_name) => {
                // Add message to room history
                {
                    let mut world = self.state.world.write().await;
                    if let Some(room) = world.get_room_mut(&room_name) {
                        room.add_message(
                            sshwarma::world::Sender::User(username.clone()),
                            sshwarma::world::MessageContent::Chat(message.to_string()),
                        );
                    }
                }

                // Persist to database
                let _ = self.state.db.add_message(&room_name, "user", &username, "chat", message);

                format!("{}: {}", username, message)
            }
            None => {
                format!("{}: {} (lobby chat not saved)", username, message)
            }
        }
    }

    async fn cmd_mention(&mut self, input: &str) -> String {
        // Parse @model message
        let input = input.trim_start_matches('@');
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let model_name = parts.first().unwrap_or(&"");
        let message = parts.get(1).copied().unwrap_or("").trim();

        if message.is_empty() {
            return format!("Usage: @{} <message>", model_name);
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        // Look up the model
        let model = match self.state.models.get(model_name) {
            Some(m) => m.clone(),
            None => {
                let available: Vec<_> = self.state.models.available().iter().map(|m| m.short_name.as_str()).collect();
                return format!(
                    "Unknown model '{}'. Available: {}",
                    model_name,
                    available.join(", ")
                );
            }
        };

        // Show the user's message
        let mut output = format!("{} → @{}: {}\r\n", username, model_name, message);

        // Build context from room history if in a room
        let history = if let Some(ref room_name) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
            let world = self.state.world.read().await;
            if let Some(room) = world.get_room(room_name) {
                room.recent_history(10)
                    .iter()
                    .filter_map(|msg| {
                        match &msg.content {
                            sshwarma::world::MessageContent::Chat(text) => {
                                let role = match &msg.sender {
                                    sshwarma::world::Sender::User(_) => "user",
                                    sshwarma::world::Sender::Model(_) => "assistant",
                                    sshwarma::world::Sender::System => return None,
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

        // System prompt for the model
        let system_prompt = format!(
            "You are {} in the sshwarma collaborative chat. \
             You're conversing with {} and possibly other users. \
             Be helpful, concise, and friendly. Keep responses under 500 words unless asked for more detail.",
            model.display_name,
            username
        );

        // Call the LLM
        output.push_str(&format!("\r\n{}: ", model.short_name));

        match self.state.llm.chat_with_context(&model, &system_prompt, &history, message).await {
            Ok(response) => {
                // Record the model's response in room history
                if let Some(ref room_name) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
                    {
                        let mut world = self.state.world.write().await;
                        if let Some(room) = world.get_room_mut(room_name) {
                            room.add_message(
                                sshwarma::world::Sender::Model(model.short_name.clone()),
                                sshwarma::world::MessageContent::Chat(response.clone()),
                            );
                        }
                    }
                    let _ = self.state.db.add_message(room_name, "model", &model.short_name, "chat", &response);
                }

                // Format response for terminal (replace \n with \r\n)
                let formatted = response.replace('\n', "\r\n");
                output.push_str(&formatted);
            }
            Err(e) => {
                output.push_str(&format!("[error: {}]", e));
            }
        }

        output
    }

    /// List all available MCP tools
    async fn cmd_tools(&self) -> String {
        let tools = self.state.mcp.list_tools().await;
        if tools.is_empty() {
            return "No tools available. Use /mcp connect <name> <url> to add an MCP server.".to_string();
        }

        let mut output = "Available tools:\r\n".to_string();
        for tool in tools {
            output.push_str(&format!(
                "  {} ({})\r\n    {}\r\n",
                tool.name, tool.source, tool.description
            ));
        }
        output
    }

    /// Run an MCP tool
    async fn cmd_run(&self, args: &str) -> String {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        let tool_name = match parts.first() {
            Some(name) if !name.is_empty() => *name,
            _ => return "Usage: /run <tool> [json args]\r\nExample: /run orpheus_generate {\"temperature\": 1.0}".to_string(),
        };

        // Parse optional JSON args
        let args_json: serde_json::Value = if let Some(json_str) = parts.get(1) {
            match serde_json::from_str(json_str) {
                Ok(v) => v,
                Err(e) => return format!("Invalid JSON: {}", e),
            }
        } else {
            serde_json::json!({})
        };

        match self.state.mcp.call_tool(tool_name, args_json).await {
            Ok(result) => {
                if result.is_error {
                    format!("Tool error: {}", result.content)
                } else {
                    result.content.replace('\n', "\r\n")
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// MCP server management
    async fn cmd_mcp(&self, args: &str) -> String {
        let parts: Vec<&str> = args.splitn(3, ' ').collect();
        let subcmd = parts.first().copied().unwrap_or("");

        match subcmd {
            "list" | "" => {
                let connections = self.state.mcp.list_connections().await;
                if connections.is_empty() {
                    "No MCP servers connected.\r\n\r\nUsage:\r\n  /mcp connect <name> <url>\r\n  /mcp disconnect <name>".to_string()
                } else {
                    let mut output = "Connected MCP servers:\r\n".to_string();
                    for conn in connections {
                        output.push_str(&format!(
                            "  {} ... {} tools @ {}\r\n",
                            conn.name, conn.tool_count, conn.endpoint
                        ));
                    }
                    output
                }
            }
            "connect" => {
                let name = parts.get(1).copied().unwrap_or("");
                let url = parts.get(2).copied().unwrap_or("");
                if name.is_empty() || url.is_empty() {
                    return "Usage: /mcp connect <name> <url>".to_string();
                }

                match self.state.mcp.connect(name, url).await {
                    Ok(()) => format!("Connected to MCP server '{}' at {}", name, url),
                    Err(e) => format!("Failed to connect: {}", e),
                }
            }
            "disconnect" => {
                let name = parts.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return "Usage: /mcp disconnect <name>".to_string();
                }

                match self.state.mcp.disconnect(name).await {
                    Ok(true) => format!("Disconnected from '{}'", name),
                    Ok(false) => format!("Not connected to '{}'", name),
                    Err(e) => format!("Error: {}", e),
                }
            }
            "refresh" => {
                let name = parts.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return "Usage: /mcp refresh <name>".to_string();
                }

                match self.state.mcp.refresh_tools(name).await {
                    Ok(()) => format!("Refreshed tools from '{}'", name),
                    Err(e) => format!("Error: {}", e),
                }
            }
            _ => format!("Unknown MCP command: {}. Try: list, connect, disconnect, refresh", subcmd),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("sshwarma=info".parse()?),
        )
        .init();

    let config = Config::default();
    info!(addr = %config.listen_addr, "starting sshwarma");

    // Generate or load host key
    let key_path = std::path::Path::new(&config.host_key_path);
    let key = if key_path.exists() {
        info!("loading host key from {}", config.host_key_path);
        russh::keys::decode_secret_key(
            &std::fs::read_to_string(&config.host_key_path)?,
            None,
        )?
    } else {
        info!("generating new host key");
        let key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .context("failed to generate key")?;
        // Save key for next time
        std::fs::write(
            &config.host_key_path,
            key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?,
        )?;
        key
    };

    let russh_config = russh::server::Config {
        keys: vec![key],
        ..Default::default()
    };

    // Initialize database
    info!("opening database at {}", config.db_path);
    let db = Database::open(&config.db_path).context("failed to open database")?;

    // Check if any users registered
    let users = db.list_users()?;
    if users.is_empty() {
        if config.allow_open_registration {
            warn!("no users registered - running in open mode");
            warn!("use sshwarma-admin to add users");
        } else {
            anyhow::bail!("no users registered and open registration disabled");
        }
    } else {
        info!("{} users registered", users.len());
    }

    // Initialize LLM client and model registry
    info!("initializing LLM client for {}", config.llm_endpoint);
    let llm = LlmClient::with_ollama_endpoint(&config.llm_endpoint)
        .context("failed to create LLM client")?;
    let models = ModelRegistry::with_defaults(&config.llm_endpoint);
    info!("{} models registered", models.list().len());

    // Load rooms from database
    let mut world = World::new();
    let saved_rooms = db.get_all_rooms()?;
    for room_info in &saved_rooms {
        world.create_room(room_info.name.clone());
        if let Some(room) = world.get_room_mut(&room_info.name) {
            room.description = room_info.description.clone();
        }
    }
    info!("{} rooms loaded from database", saved_rooms.len());

    let mcp = McpClients::new();
    let state = Arc::new(SharedState {
        world: RwLock::new(world),
        db,
        config: config.clone(),
        llm,
        models,
        mcp,
    });

    let mut server = SshServer { state };

    info!("listening on {}", config.listen_addr);
    server
        .run_on_address(Arc::new(russh_config), config.listen_addr)
        .await?;

    Ok(())
}
