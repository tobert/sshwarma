//! SSH connection handler

use crate::db::rows::Row;
use crate::internal_tools::{InternalToolConfig, ToolContext};
use crate::llm::StreamChunk;
use crate::lua::{mcp_request_handler, LuaRuntime, McpBridge, WrapState};
use crate::model::ModelHandle;
use crate::player::PlayerSession;
use crate::ssh::screen::spawn_screen_refresh;
use crate::ssh::session::SessionState;
use crate::ssh::streaming::{push_updates_task, RowUpdate};
use crate::state::SharedState;
use anyhow::Result;
use rig::tool::server::ToolServer;
use russh::server::{self, Handle, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec, Pty};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

/// Per-connection SSH handler
pub struct SshHandler {
    pub state: Arc<SharedState>,
    pub player: Option<PlayerSession>,
    pub term_size: (u16, u16),
    /// Session state for buffer rendering
    pub session_state: Arc<Mutex<SessionState>>,
    /// Sender for row updates from background tasks
    pub update_tx: mpsc::Sender<RowUpdate>,
    /// Receiver for row updates (taken by background task)
    pub update_rx: Option<mpsc::Receiver<RowUpdate>>,
    pub session_handle: Option<Handle>,
    pub main_channel: Option<ChannelId>,
    pub lua_runtime: Option<Arc<Mutex<LuaRuntime>>>,
    pub mcp_bridge: Option<Arc<McpBridge>>,
    pub mcp_request_rx: Option<mpsc::Receiver<crate::lua::mcp_bridge::McpRequest>>,
}

impl SshHandler {
    pub fn new(state: Arc<SharedState>) -> Self {
        let (update_tx, update_rx) = mpsc::channel(32);
        Self {
            state: state.clone(),
            player: None,
            term_size: (80, 24),
            session_state: Arc::new(Mutex::new(SessionState::new())),
            update_tx,
            update_rx: Some(update_rx),
            session_handle: None,
            main_channel: None,
            lua_runtime: None,
            mcp_bridge: None,
            mcp_request_rx: None,
        }
    }

    // =========================================================================
    // Lua Helper Methods
    // =========================================================================

    /// Execute a closure with the Lua runtime locked.
    ///
    /// Returns None if no Lua runtime is available.
    pub async fn with_lua<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&LuaRuntime) -> R,
    {
        let lua_runtime = self.lua_runtime.as_ref()?;
        let lua = lua_runtime.lock().await;
        Some(f(&lua))
    }

    /// Get the current room name from Lua session context.
    ///
    /// Falls back to player.current_room for backward compatibility.
    pub async fn current_room(&self) -> Option<String> {
        // Try Lua session context first
        if let Some(room) = self
            .with_lua(|lua| {
                lua.tool_state()
                    .session_context()
                    .and_then(|ctx| ctx.room_name.clone())
            })
            .await
            .flatten()
        {
            return Some(room);
        }
        // Fall back to player state
        self.player.as_ref().and_then(|p| p.current_room.clone())
    }

    /// Mark a UI region as dirty for redraw.
    pub async fn mark_dirty(&self, tag: &str) {
        self.with_lua(|lua| lua.tool_state().mark_dirty(tag)).await;
    }

    /// Push a notification to the UI.
    pub async fn push_notification(&self, msg: impl Into<String>, duration_ms: i64) {
        let msg = msg.into();
        self.with_lua(|lua| lua.tool_state().push_notification(msg, duration_ms))
            .await;
    }

    /// Push an error notification to the UI.
    pub async fn push_error(&self, msg: impl Into<String>) {
        let msg = msg.into();
        self.with_lua(|lua| {
            lua.tool_state().push_notification_with_level(
                msg,
                5000,
                crate::lua::NotificationLevel::Error,
            )
        })
        .await;
    }

    // =========================================================================
    // Other Methods
    // =========================================================================

    /// Get set of equipped tool qualified names for a room
    ///
    /// Returns empty set if room has no equipped tools or things system not initialized.
    /// This is used to filter which MCP/internal tools are available during @mention.
    fn get_equipped_tool_names(&self, room_name: &str) -> std::collections::HashSet<String> {
        use crate::db::things::ThingKind;

        // Ensure world is bootstrapped
        if self.state.db.bootstrap_world().is_err() {
            return std::collections::HashSet::new();
        }

        // Find room thing by name
        let room_thing = match self.state.db.find_things_by_name(room_name) {
            Ok(things) => things.into_iter().find(|t| t.kind == ThingKind::Room),
            Err(_) => None,
        };

        // Get room ID, falling back to convention
        let room_id = room_thing
            .map(|t| t.id)
            .unwrap_or_else(|| format!("room_{}", room_name));

        // Get equipped tools for this room
        let equipped = self
            .state
            .db
            .get_room_equipment_tools(&room_id)
            .unwrap_or_default();

        // Build set of qualified names
        equipped
            .into_iter()
            .filter_map(|eq| eq.thing.qualified_name)
            .collect()
    }

    /// Initialize session after authentication
    pub async fn init_session(&mut self, username: &str) {
        // Create player session
        self.player = Some(PlayerSession::new(username.to_string()));

        // Create Lua runtime (automatically loads user-specific script)
        let runtime =
            LuaRuntime::new_for_user(Some(username)).expect("failed to create lua runtime");

        // Set shared state so sshwarma.call() has access to DB/MCP
        runtime
            .tool_state()
            .set_shared_state(Some(self.state.clone()));

        // Try to load user's UI entrypoint from database
        // This uses the new virtual require system to load user DB scripts
        if let Err(e) = runtime.try_load_db_entrypoint(&self.state.db, username) {
            tracing::warn!("Failed to load DB entrypoint for '{}': {}", username, e);
        }

        let runtime = Arc::new(Mutex::new(runtime));
        self.lua_runtime = Some(runtime.clone());

        // Create MCP bridge
        let (bridge, request_rx) = McpBridge::with_defaults();
        self.mcp_bridge = Some(Arc::new(bridge));
        self.mcp_request_rx = Some(request_rx);
    }

    /// Join a room
    pub async fn join_room(&mut self, room_name: &str) -> Result<()> {
        // Get/create room buffer
        let buffer = self.state.db.get_or_create_room_buffer(room_name)?;

        // Get room ID from database
        let room_id = self
            .state
            .db
            .get_room_by_name(room_name)?
            .map(|r| r.id);

        // Update session state
        {
            let mut sess = self.session_state.lock().await;
            sess.set_buffer(Some(buffer.id.clone()));
        }

        // Update player
        if let Some(ref mut player) = self.player {
            player.current_room = Some(room_name.to_string());
        }

        // Update Lua session context so status tool knows the room
        if let Some(ref lua_runtime) = self.lua_runtime {
            let lua = lua_runtime.lock().await;
            if let Some(ref player) = self.player {
                lua.tool_state()
                    .set_session_context(Some(crate::lua::SessionContext {
                        username: player.username.clone(),
                        model: None,
                        room_name: Some(room_name.to_string()),
                        room_id: room_id.clone(),
                    }));
            }
        }

        // Add join presence row
        if let Some(ref player) = self.player {
            let mut row = Row::new(&buffer.id, "presence.join");
            row.content = Some(player.username.clone());
            self.state.db.append_row(&mut row)?;
        }

        // Update in-memory world
        {
            let mut world = self.state.world.write().await;
            if world.get_room(room_name).is_none() {
                world.create_room_with_buffer(room_name.to_string(), buffer.id.clone());
            }
            if let Some(room) = world.get_room_mut(room_name) {
                if let Some(ref player) = self.player {
                    room.add_user(player.username.clone());
                }
                if room.buffer_id.is_none() {
                    room.set_buffer_id(buffer.id);
                }
            }
        }

        Ok(())
    }

    /// Leave current room
    pub async fn leave_room(&mut self) -> Result<()> {
        let Some(ref player) = self.player else {
            return Ok(());
        };

        let Some(ref room_name) = player.current_room.clone() else {
            return Ok(());
        };

        // Add leave presence row
        if let Ok(buffer) = self.state.db.get_or_create_room_buffer(room_name) {
            let mut row = Row::new(&buffer.id, "presence.leave");
            row.content = Some(player.username.clone());
            let _ = self.state.db.append_row(&mut row);
        }

        // Update in-memory world
        {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(room_name) {
                room.remove_user(&player.username);
            }
        }

        // Clear session buffer
        {
            let mut sess = self.session_state.lock().await;
            sess.set_buffer(None);
        }

        // Update player
        if let Some(ref mut player) = self.player {
            player.current_room = None;
        }

        // Update Lua session context
        if let Some(ref lua_runtime) = self.lua_runtime {
            let lua = lua_runtime.lock().await;
            if let Some(ref player) = self.player {
                lua.tool_state()
                    .set_session_context(Some(crate::lua::SessionContext {
                        username: player.username.clone(),
                        model: None,
                        room_name: None,
                        room_id: None,
                    }));
            }
        }

        Ok(())
    }

    /// Spawn background task for model response with tool support
    pub async fn spawn_model_response(
        &self,
        model: ModelHandle,
        message: String,
        username: String,
        room_name: Option<String>,
        placeholder_row_id: Option<String>,
    ) -> Result<()> {
        let llm = self.state.llm.clone();
        let update_tx = self.update_tx.clone();
        let state = self.state.clone();
        let lua_runtime = self.lua_runtime.clone();

        // Get MCP tools for rig agent
        let mcp_context = self.state.mcp.rig_tools().await;

        // Get equipped tools for this room to filter available tools
        let room_for_tools = room_name.clone().unwrap_or_else(|| "lobby".to_string());
        let equipped_tools = self.get_equipped_tool_names(&room_for_tools);

        // Build ToolServer with MCP + internal tools (filtered by equipped)
        let tool_server_handle = {
            let mut server = ToolServer::new();

            // Add MCP tools if available (filtered by equipped status)
            if let Some(ref ctx) = mcp_context {
                for (tool, peer) in ctx.tools.iter() {
                    // Convert MCP tool name to qualified format: server__tool -> server:tool
                    let qualified = tool.name.replace("__", ":");

                    // Check if this tool is equipped (or if no filtering is active)
                    if equipped_tools.is_empty() || equipped_tools.contains(&qualified) {
                        server = server.rmcp_tool(tool.clone(), peer.clone());
                    } else {
                        tracing::debug!("skipping MCP tool {} (not equipped)", qualified);
                    }
                }
            }

            server.run()
        };

        // Register internal sshwarma tools (filtered by equipment status)
        let room_for_tools = room_name.clone().unwrap_or_else(|| "lobby".to_string());
        let in_room = room_name.is_some();

        if let Some(ref lua_rt) = lua_runtime {
            let tool_ctx = ToolContext {
                state: state.clone(),
                room: room_for_tools.clone(),
                username: username.clone(),
                lua_runtime: lua_rt.clone(),
            };
            let config = InternalToolConfig::for_room(&state, &room_for_tools).await;
            match crate::internal_tools::register_tools(
                &tool_server_handle,
                tool_ctx,
                &config,
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
        let target_tokens = model.context_window.unwrap_or(8000);
        let (system_prompt, full_message) = if let Some(ref lua_rt) = lua_runtime {
            let wrap_state = WrapState {
                room_name: room_name.clone(),
                username: username.clone(),
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
                        message.clone()
                    } else {
                        format!("{}\n\n---\n\n{}", result.context, message)
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
                    return Ok(());
                }
            }
        } else {
            // No Lua runtime - use basic fallback
            let prompt = format!(
                "You are {} in a collaborative chat room. Be helpful and concise.{}",
                model.display_name, tool_guide
            );
            (prompt, message.clone())
        };

        let model_short = model.short_name.clone();
        let row_id = placeholder_row_id.clone();
        let room_for_tracking = room_name.clone();

        tokio::spawn(async move {
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
                    let result = llm.stream_with_tool_server(
                        &model,
                        &system_prompt,
                        &full_message,
                        tool_server_handle,
                        chunk_tx,
                        100, // max tool turns
                    )
                    .await;
                    tracing::info!("spawn_model_response: stream_with_tool_server returned: {:?}", result.is_ok());
                    result
                }
            });

            // Process streaming chunks
            let mut full_response = String::new();
            tracing::info!("spawn_model_response: waiting for chunks");

            while let Some(chunk) = chunk_rx.recv().await {
                tracing::info!("spawn_model_response: received chunk: {:?}", std::mem::discriminant(&chunk));
                match chunk {
                    StreamChunk::Text(text) => {
                        tracing::info!("spawn_model_response: text chunk len={}", text.len());
                        full_response.push_str(&text);
                        if let Some(ref row_id) = row_id {
                            let _ = update_tx
                                .send(RowUpdate::Chunk {
                                    row_id: row_id.clone(),
                                    text,
                                })
                                .await;
                        }
                    }
                    StreamChunk::ToolCall { name, arguments } => {
                        last_tool_name = name.clone();
                        if let Some(ref row_id) = row_id {
                            let _ = update_tx
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
                    StreamChunk::ToolResult(summary) => {
                        if let Some(ref row_id) = row_id {
                            let _ = update_tx
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
                let _ = update_tx
                    .send(RowUpdate::Complete {
                        row_id,
                        model_name: model_short,
                    })
                    .await;
            }
        });

        Ok(())
    }

    /// Get current room buffer ID
    pub async fn current_buffer_id(&self) -> Option<String> {
        let sess = self.session_state.lock().await;
        sess.buffer_id.clone()
    }
}

impl server::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        let key_str = key.to_string();

        if let Ok(Some(handle)) = self.state.db.lookup_handle_by_pubkey(&key_str) {
            info!(handle, "key recognized");
            Ok(server::Auth::Accept)
        } else if self.state.config.allow_open_registration {
            info!("open registration enabled, accepting new key");
            Ok(server::Auth::Accept)
        } else {
            warn!("unknown key");
            Ok(server::Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            })
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        let key_str = key.to_string();

        // Check existing user
        if let Ok(Some(handle)) = self.state.db.lookup_handle_by_pubkey(&key_str) {
            info!(handle = %handle, "authenticated");
            self.init_session(&handle).await;
            return Ok(server::Auth::Accept);
        }

        // Open registration
        if self.state.config.allow_open_registration {
            let handle = user.to_string();
            if let Err(e) = self.state.db.add_pubkey(&handle, &key_str, "ssh", None) {
                warn!("failed to register user: {}", e);
                return Ok(server::Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                });
            }
            info!(handle = %handle, "registered new user");
            self.init_session(&handle).await;
            return Ok(server::Auth::Accept);
        }

        Ok(server::Auth::Reject {
            proceed_with_methods: None,
            partial_success: false,
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        info!(channel_id = ?channel.id(), "session channel opened");
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        _channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.term_size = (col_width as u16, row_height as u16);
        // Screen refresh task will pick up new dimensions on next tick
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let (width, height) = self.term_size;

        // Store handles
        self.session_handle = Some(session.handle());
        self.main_channel = Some(channel);

        // Enter alternate screen buffer, clear it, and hide cursor
        // \x1b[?1049h = enter alternate screen buffer
        // \x1b[2J = clear entire screen
        // \x1b[H = cursor to home (0,0)
        // \x1b[?25l = hide cursor (screen refresh will show it at input position)
        let init_seq = "\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l";
        let _ = session.data(channel, CryptoVec::from(init_seq.as_bytes()));

        // Auto-join user to lobby and send welcome notification
        if let Err(e) = self.join_room("lobby").await {
            tracing::warn!("failed to join lobby: {}", e);
        }

        if let Some(ref player) = self.player {
            if let Some(ref lua_runtime) = self.lua_runtime {
                let lua = lua_runtime.lock().await;
                // Welcome as notification
                lua.tool_state()
                    .push_notification(format!("Welcome, {}!", player.username), 5000);
            }
        }

        // Spawn background task for model streaming updates
        if let Some(update_rx) = self.update_rx.take() {
            let db = self.state.db.clone();
            let lua_runtime = self.lua_runtime.clone();

            tokio::spawn(async move {
                push_updates_task(update_rx, db, lua_runtime).await;
            });
        }

        // Spawn screen refresh - Lua owns full terminal
        spawn_screen_refresh(
            session.handle(),
            channel,
            self.lua_runtime.clone().expect("lua_runtime"),
            self.state.clone(),
            width,
            height,
        );

        // Spawn MCP request handler
        if let Some(request_rx) = self.mcp_request_rx.take() {
            let mcp_bridge = self.mcp_bridge.clone().expect("mcp_bridge");
            let mcp = self.state.mcp.clone();
            let requests = mcp_bridge.requests();
            let timeout = mcp_bridge.timeout();
            tokio::spawn(async move {
                mcp_request_handler(request_rx, mcp, requests, timeout).await;
            });
        }

        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Forward raw bytes to Lua for parsing - no fallback, fail visibly
        let Some(ref lua_runtime) = self.lua_runtime else {
            tracing::error!("No Lua runtime available for input handling");
            return Ok(());
        };

        let lua = lua_runtime.lock().await;
        match lua.call_on_input(data) {
            Ok(Some(action)) => {
                drop(lua); // Release lock before handling action
                self.handle_input_action(channel, session, action).await;
            }
            Ok(None) => {
                // No action needed, just mark input dirty for redraw
                lua.tool_state().mark_dirty("input");
            }
            Err(e) => {
                // Log error visibly - no silent fallback
                tracing::error!("Lua input handling failed: {}", e);
                lua.tool_state().push_notification_with_level(
                    format!("Input error: {}", e),
                    5000,
                    crate::lua::NotificationLevel::Error,
                );
            }
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.term_size = (col_width as u16, row_height as u16);
        // Screen refresh task will pick up new dimensions on next tick
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Exit alternate screen buffer and show cursor before disconnect
        // This restores the terminal to normal state
        let cleanup_seq = "\x1b[?25h\x1b[?1049l";
        let _ = session.data(channel, CryptoVec::from(cleanup_seq.as_bytes()));
        Ok(())
    }
}

impl SshHandler {
    /// Handle action from Lua input parser
    async fn handle_input_action(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        action: crate::lua::InputAction,
    ) {
        use crate::lua::InputAction;

        match action {
            InputAction::None => {}

            InputAction::Redraw => {
                self.mark_dirty("input").await;
            }

            InputAction::Execute(line) => {
                if line.trim() == "/quit" {
                    let _ = session.close(channel);
                    return;
                }

                if let Err(e) = self.process_input(channel, session, &line).await {
                    self.push_notification(format!("Error: {}", e), 5000).await;
                }

                self.mark_dirty("input").await;
                self.mark_dirty("chat").await;
            }

            InputAction::Tab => {
                // Tab completion TODO: rewrite in Lua
            }

            InputAction::ClearScreen => {
                self.mark_dirty("chat").await;
                self.mark_dirty("status").await;
                self.mark_dirty("input").await;
            }

            InputAction::Quit => {
                let _ = session.close(channel);
            }

            InputAction::Escape | InputAction::PageUp | InputAction::PageDown => {
                // Navigation handled by Lua mode.lua
            }
        }
    }
}
