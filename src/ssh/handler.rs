//! SSH connection handler

use crate::ansi::EscapeParser;
use crate::completion::{Completion, CompletionContext, CompletionEngine};
use crate::db::rows::Row;
use crate::internal_tools::{InternalToolConfig, ToolContext};
use crate::line_editor::{EditorAction, LineEditor};
use crate::llm::StreamChunk;
use crate::lua::{mcp_request_handler, LuaRuntime, McpBridge};
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
    pub editor: LineEditor,
    pub esc_parser: EscapeParser,
    pub term_size: (u16, u16),
    pub completer: CompletionEngine,
    pub completions: Vec<Completion>,
    pub completion_index: usize,
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
            editor: LineEditor::new(),
            esc_parser: EscapeParser::new(),
            term_size: (80, 24),
            completer: CompletionEngine::new(state),
            completions: Vec::new(),
            completion_index: 0,
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

    /// Initialize session after authentication
    pub async fn init_session(&mut self, username: &str) {
        // Create player session
        self.player = Some(PlayerSession::new(username.to_string()));

        // Create Lua runtime (automatically loads user-specific script)
        let runtime = LuaRuntime::new_for_user(Some(username))
            .expect("failed to create lua runtime");

        // Set shared state so sshwarma.call() has access to DB/MCP
        runtime.tool_state().set_shared_state(Some(self.state.clone()));

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
                lua.tool_state().set_session_context(Some(crate::lua::SessionContext {
                    username: player.username.clone(),
                    model: None,
                    room_name: Some(room_name.to_string()),
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
                lua.tool_state().set_session_context(Some(crate::lua::SessionContext {
                    username: player.username.clone(),
                    model: None,
                    room_name: None,
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

        // Build ToolServer with MCP + internal tools
        let tool_server_handle = {
            let mut server = ToolServer::new();

            // Add MCP tools if available (each tool paired with its peer)
            if let Some(ref ctx) = mcp_context {
                for (tool, peer) in ctx.tools.iter() {
                    server = server.rmcp_tool(tool.clone(), peer.clone());
                }
            }

            server.run()
        };

        // Register internal sshwarma tools
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
            match crate::internal_tools::register_tools(&tool_server_handle, tool_ctx, &config, in_room).await {
                Ok(count) => tracing::info!("registered {} internal tools for @mention", count),
                Err(e) => tracing::error!("failed to register internal tools: {}", e),
            }
        }

        // Build system prompt with tool definitions
        let base_prompt = format!(
            "You are {} in a collaborative chat room. Be helpful and concise.",
            model.display_name
        );

        let system_prompt = match tool_server_handle.get_tool_defs(None).await {
            Ok(tool_defs) if !tool_defs.is_empty() => {
                let mut tool_guide = String::from("\n\n## Your Functions\n");
                tool_guide.push_str("You have these built-in functions:\n\n");
                for tool in &tool_defs {
                    let display_name = tool.name.strip_prefix("sshwarma_").unwrap_or(&tool.name);
                    tool_guide.push_str(&format!("- **{}**: {}\n", display_name, tool.description));
                }
                tracing::info!("injecting {} tool definitions into prompt", tool_defs.len());
                format!("{}{}", base_prompt, tool_guide)
            }
            Ok(_) => {
                tracing::warn!("no tools available for @mention");
                base_prompt
            }
            Err(e) => {
                tracing::error!("failed to get tool definitions: {}", e);
                base_prompt
            }
        };

        let model_short = model.short_name.clone();
        let row_id = placeholder_row_id.clone();

        tokio::spawn(async move {
            // Create channel for streaming chunks
            let (chunk_tx, mut chunk_rx) = mpsc::channel::<StreamChunk>(32);

            // Spawn the streaming LLM call
            let stream_handle = tokio::spawn({
                let llm = llm.clone();
                let model = model.clone();
                let system_prompt = system_prompt.clone();
                let message = message.clone();
                async move {
                    llm.stream_with_tool_server(
                        &model,
                        &system_prompt,
                        &message,
                        tool_server_handle,
                        chunk_tx,
                        100, // max tool turns
                    ).await
                }
            });

            // Process streaming chunks
            let mut full_response = String::new();

            while let Some(chunk) = chunk_rx.recv().await {
                match chunk {
                    StreamChunk::Text(text) => {
                        full_response.push_str(&text);
                        if let Some(ref row_id) = row_id {
                            let _ = update_tx.send(RowUpdate::Chunk {
                                row_id: row_id.clone(),
                                text,
                            }).await;
                        }
                    }
                    StreamChunk::ToolCall(name) => {
                        if let Some(ref row_id) = row_id {
                            let _ = update_tx.send(RowUpdate::ToolCall {
                                row_id: row_id.clone(),
                                tool_name: name,
                                model_name: model_short.clone(),
                            }).await;
                        }
                    }
                    StreamChunk::ToolResult(summary) => {
                        if let Some(ref row_id) = row_id {
                            let _ = update_tx.send(RowUpdate::ToolResult {
                                row_id: row_id.clone(),
                                summary,
                            }).await;
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
                let _ = update_tx.send(RowUpdate::Complete {
                    row_id,
                    model_name: model_short,
                }).await;
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

        // Set session context and send welcome notification
        if let Some(ref player) = self.player {
            if let Some(ref lua_runtime) = self.lua_runtime {
                let lua = lua_runtime.lock().await;
                lua.tool_state().set_session_context(Some(crate::lua::SessionContext {
                    username: player.username.clone(),
                    model: None,
                    room_name: None,
                }));
                // Welcome as notification
                lua.tool_state().push_notification(
                    format!("Welcome, {}!", player.username),
                    5000,
                );
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

        // Show prompt
        self.show_prompt(channel, session).await;

        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        for &byte in data {
            if let Some(event) = self.esc_parser.feed(byte) {
                let action = self.editor.handle_event(event);
                self.handle_editor_action(channel, session, action).await;
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
}

impl SshHandler {
    async fn handle_editor_action(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        action: EditorAction,
    ) {
        match action {
            EditorAction::None => {}

            EditorAction::Redraw => {
                // Clear completions when user types
                self.clear_completions();
                // Update input state - Lua renders it
                self.update_input_state().await;
            }

            EditorAction::Execute(line) => {
                // Handle /quit specially
                if line.trim() == "/quit" {
                    let _ = session.close(channel);
                    return;
                }

                // Process the input - Lua renders results via tools.history()
                if let Err(e) = self.process_input(channel, session, &line).await {
                    // Push error as notification for Lua to display
                    if let Some(ref lua_runtime) = self.lua_runtime {
                        let lua = lua_runtime.lock().await;
                        lua.tool_state().push_notification(format!("Error: {}", e), 5000);
                    }
                }

                // Clear input and update state
                self.show_prompt(channel, session).await;
            }

            EditorAction::Tab => {
                self.handle_tab_completion(channel, session).await;
            }

            EditorAction::ClearScreen => {
                // Lua handles screen rendering - just update input state
                self.show_prompt(channel, session).await;
            }

            EditorAction::Quit => {
                let _ = session.close(channel);
            }
        }
    }

    /// Get the current prompt string
    fn get_prompt(&self) -> String {
        if let Some(ref player) = self.player {
            if let Some(ref room) = player.current_room {
                format!("{}> ", room)
            } else {
                "lobby> ".to_string()
            }
        } else {
            "> ".to_string()
        }
    }

    /// Update input state for Lua rendering (no direct terminal output)
    async fn update_input_state(&self) {
        if let Some(ref lua_runtime) = self.lua_runtime {
            let lua = lua_runtime.lock().await;
            lua.tool_state().set_input(
                self.editor.value(),
                self.editor.cursor(),
                &self.get_prompt(),
            );
        }
    }

    async fn show_prompt(&self, _channel: ChannelId, _session: &mut Session) {
        // Input is now rendered by Lua - just update state
        self.update_input_state().await;
    }

    async fn handle_tab_completion(&mut self, channel: ChannelId, session: &mut Session) {
        let line = self.editor.value().to_string();
        let cursor = self.editor.cursor();
        let room: Option<String> = self.player.as_ref().and_then(|p| p.current_room.clone());

        // If we already have completions and user pressed tab again, cycle
        if !self.completions.is_empty() {
            self.completion_index = (self.completion_index + 1) % self.completions.len();
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.apply_completion(channel, session, &ctx).await;
            return;
        }

        // Get fresh completions
        {
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.completions = self.completer.complete(&ctx).await;
        }
        self.completion_index = 0;

        if self.completions.is_empty() {
            // No completions, beep
            let _ = session.data(channel, CryptoVec::from(b"\x07".as_slice()));
        } else if self.completions.len() == 1 {
            // Single completion, apply it
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.apply_completion(channel, session, &ctx).await;
            self.completions.clear();
        } else {
            // Multiple completions, apply first
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.apply_completion(channel, session, &ctx).await;
        }
    }

    async fn apply_completion(&mut self, _channel: ChannelId, _session: &mut Session, ctx: &CompletionContext<'_>) {
        if let Some(completion) = self.completions.get(self.completion_index) {
            let (start, _end) = self.completer.replacement_range(ctx);
            self.editor.replace_with_completion(start, &completion.text);

            // Add space after completion if it's a command or mention
            if completion.text.starts_with('/') || completion.text.starts_with('@') {
                self.editor.insert_completion(" ");
            }

            // Update input state - Lua renders it
            self.update_input_state().await;
        }
    }

    fn clear_completions(&mut self) {
        self.completions.clear();
        self.completion_index = 0;
    }
}
