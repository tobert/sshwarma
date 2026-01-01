//! SSH connection handler

use crate::ansi::EscapeParser;
use crate::completion::{Completion, CompletionContext, CompletionEngine};
use crate::db::rows::Row;
use crate::display::hud::{HudState, McpConnectionState, HUD_HEIGHT};
use crate::display::styles::ctrl;
use crate::line_editor::{EditorAction, LineEditor};
use crate::lua::{mcp_request_handler, LuaRuntime, McpBridge};
use crate::model::ModelHandle;
use crate::player::PlayerSession;
use crate::ssh::hud::spawn_hud_refresh;
use crate::ssh::session::SessionState;
use crate::ssh::streaming::{push_updates_task, RowUpdate};
use crate::state::SharedState;
use anyhow::Result;
use chrono::Utc;
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
    pub hud_state: Arc<Mutex<HudState>>,
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
            session_state: Arc::new(Mutex::new(SessionState::new(80))),
            update_tx,
            update_rx: Some(update_rx),
            session_handle: None,
            main_channel: None,
            hud_state: Arc::new(Mutex::new(HudState::new())),
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
        let runtime = Arc::new(Mutex::new(runtime));
        self.lua_runtime = Some(runtime.clone());

        // Create MCP bridge
        let (bridge, request_rx) = McpBridge::with_defaults();
        self.mcp_bridge = Some(Arc::new(bridge));
        self.mcp_request_rx = Some(request_rx);

        // Initialize HUD state
        {
            let mut hud = self.hud_state.lock().await;
            hud.add_user(username.to_string());
            hud.session_start = Utc::now();
        }
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

        // Update HUD
        {
            let mut hud = self.hud_state.lock().await;
            let exits = self.state.db.get_exits(room_name).unwrap_or_default();
            let vibe = self.state.db.get_vibe(room_name).ok().flatten();
            hud.set_room(Some(room_name.to_string()), vibe, None, exits);
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

        // Update HUD
        {
            let mut hud = self.hud_state.lock().await;
            hud.set_room(None, None, None, std::collections::HashMap::new());
        }

        Ok(())
    }

    /// Spawn background task for model response
    pub async fn spawn_model_response(
        &self,
        model: ModelHandle,
        message: String,
        _username: String,
        room_name: Option<String>,
        placeholder_row_id: Option<String>,
    ) -> Result<()> {
        let llm = self.state.llm.clone();
        let db = self.state.db.clone();
        let update_tx = self.update_tx.clone();

        tokio::spawn(async move {
            // Build context
            let system_prompt = format!(
                "You are {} in a collaborative chat. Be helpful and concise.",
                model.display_name
            );

            // Get history from room buffer
            let history = if let Some(ref room) = room_name {
                if let Ok(buffer) = db.get_or_create_room_buffer(room) {
                    if let Ok(rows) = db.list_recent_buffer_rows(&buffer.id, 10) {
                        rows.into_iter()
                            .filter(|r| r.content_method.starts_with("message."))
                            .filter_map(|r| {
                                let content = r.content?;
                                let role = if r.content_method == "message.user" {
                                    "user"
                                } else {
                                    "assistant"
                                };
                                Some((role.to_string(), content))
                            })
                            .collect()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            // Call LLM
            let response = match llm.chat_with_context(&model, &system_prompt, &history, &message).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("LLM error: {}", e);
                    format!("[error: {}]", e)
                }
            };

            // Update placeholder row with response
            if let Some(ref row_id) = placeholder_row_id {
                if let Err(e) = db.append_to_row(row_id, &response) {
                    tracing::error!("failed to update row: {}", e);
                }
            }

            // Send completion
            if let Some(row_id) = placeholder_row_id {
                let _ = update_tx.send(RowUpdate::Complete {
                    row_id,
                    model_name: model.short_name.clone(),
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
        {
            let mut sess = self.session_state.lock().await;
            sess.set_width(col_width as usize);
        }
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

        // Setup terminal
        let mut init = String::new();
        init.push_str(&ctrl::set_scroll_region(1, height.saturating_sub(HUD_HEIGHT)));
        init.push_str(&ctrl::move_to(1, 1));
        init.push_str(&ctrl::clear_screen());
        let _ = session.data(channel, CryptoVec::from(init.as_bytes()));

        // Send welcome
        if let Some(ref player) = self.player {
            let welcome = format!("\x1b[32mWelcome, {}.\x1b[0m\r\n\r\n", player.username);
            let _ = session.data(channel, CryptoVec::from(welcome.as_bytes()));

            // Initialize HUD
            {
                let mut hud = self.hud_state.lock().await;
                hud.add_user(player.username.clone());

                let mcp_connections = self.state.mcp.list_connections().await;
                hud.set_mcp_connections(
                    mcp_connections.into_iter()
                        .map(|c| McpConnectionState {
                            name: c.name,
                            tool_count: c.tool_count,
                            connected: true,
                            call_count: c.call_count,
                            last_tool: c.last_tool,
                        })
                        .collect(),
                );
            }
        }

        // Draw initial HUD
        {
            let hud = self.hud_state.lock().await;
            if let Some(ref lua) = self.lua_runtime {
                let lua = lua.lock().await;
                lua.update_state(hud.clone());
                let now_ms = Utc::now().timestamp_millis();
                if let Ok(hud_str) = lua.render_hud_string(now_ms, width, height) {
                    let hud_row = height.saturating_sub(HUD_HEIGHT);
                    let output = format!("{}{}", ctrl::move_to(hud_row, 1), hud_str);
                    let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                }
            }
        }

        // Spawn background tasks
        if let Some(update_rx) = self.update_rx.take() {
            let handle = session.handle();
            let db = self.state.db.clone();
            let buffer_id = self.current_buffer_id().await.unwrap_or_default();
            let hud_state = self.hud_state.clone();
            let lua_runtime = self.lua_runtime.clone().expect("lua_runtime");

            tokio::spawn(async move {
                push_updates_task(
                    handle, channel, update_rx, db, buffer_id,
                    hud_state, lua_runtime, width, height,
                ).await;
            });
        }

        // Spawn HUD refresh
        spawn_hud_refresh(
            session.handle(),
            channel,
            self.hud_state.clone(),
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
        {
            let mut sess = self.session_state.lock().await;
            sess.set_width(col_width as usize);
        }
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
        let (_, height) = self.term_size;
        let input_row = height;

        match action {
            EditorAction::None => {}

            EditorAction::Redraw => {
                // Clear completions when user types
                self.clear_completions();
                // Redraw the input line
                let output = format!(
                    "{}{}{}",
                    ctrl::move_to(input_row, 1),
                    ctrl::clear_line(),
                    self.editor.value()
                );
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            }

            EditorAction::Execute(line) => {
                // Handle /quit specially
                if line.trim() == "/quit" {
                    let output = format!("{}\r\nGoodbye!\r\n", ctrl::reset_scroll_region());
                    let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                    let _ = session.close(channel);
                    return;
                }

                // Move to new line
                let _ = session.data(channel, CryptoVec::from(ctrl::CRLF.as_bytes()));

                // Process the input
                if let Err(e) = self.process_input(channel, session, &line).await {
                    let error = format!("\x1b[31mError: {}\x1b[0m\r\n", e);
                    let _ = session.data(channel, CryptoVec::from(error.as_bytes()));
                }

                self.show_prompt(channel, session).await;
            }

            EditorAction::Tab => {
                self.handle_tab_completion(channel, session).await;
            }

            EditorAction::ClearScreen => {
                // Full redraw
                let _ = session.data(channel, CryptoVec::from(ctrl::clear_screen().as_bytes()));
                self.render_full(channel, session).await;
                self.show_prompt(channel, session).await;
            }

            EditorAction::Quit => {
                // Reset scroll region and say goodbye
                let output = format!("{}\r\nGoodbye!\r\n", ctrl::reset_scroll_region());
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                let _ = session.close(channel);
            }
        }
    }

    async fn show_prompt(&self, channel: ChannelId, session: &mut Session) {
        let prompt = if let Some(ref player) = self.player {
            if let Some(ref room) = player.current_room {
                format!("{}> ", room)
            } else {
                "lobby> ".to_string()
            }
        } else {
            "> ".to_string()
        };
        let _ = session.data(channel, CryptoVec::from(prompt.as_bytes()));
    }

    #[allow(dead_code)]
    async fn redraw_line(&self, channel: ChannelId, session: &mut Session, line: &str) {
        let output = format!("{}{}{}", ctrl::CR, ctrl::clear_to_eol(), line);
        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
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
            self.apply_completion(channel, session, &ctx);
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
            self.apply_completion(channel, session, &ctx);
            self.completions.clear();
        } else {
            // Multiple completions, apply first
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.apply_completion(channel, session, &ctx);
        }
    }

    fn apply_completion(&mut self, channel: ChannelId, session: &mut Session, ctx: &CompletionContext<'_>) {
        if let Some(completion) = self.completions.get(self.completion_index) {
            let (start, _end) = self.completer.replacement_range(ctx);
            self.editor.replace_with_completion(start, &completion.text);

            // Add space after completion if it's a command or mention
            if completion.text.starts_with('/') || completion.text.starts_with('@') {
                self.editor.insert_completion(" ");
            }

            // Redraw the line
            let output = self.editor.render("");
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
        }
    }

    fn clear_completions(&mut self) {
        self.completions.clear();
        self.completion_index = 0;
    }
}
