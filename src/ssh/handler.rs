//! SSH connection handler

use crate::db::rows::Row;
use crate::lua::{mcp_request_handler, LuaRuntime, McpBridge};
use crate::model::ModelHandle;
use crate::ops::{spawn_model_response, ModelResponseConfig};
use crate::player::PlayerSession;
use crate::ssh::screen::spawn_screen_refresh;
use crate::ssh::session::SessionState;
use crate::ssh::streaming::{push_updates_task, RowUpdate};
use crate::state::SharedState;
use anyhow::Result;
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
    /// Looks up room name from room_id via database.
    /// Falls back to player.current_room for backward compatibility.
    pub async fn current_room(&self) -> Option<String> {
        // Try Lua session context first - look up room name from room_id
        if let Some(room_id) = self
            .with_lua(|lua| {
                lua.tool_state()
                    .session_context()
                    .and_then(|ctx| ctx.room_id.clone())
            })
            .await
            .flatten()
        {
            if let Ok(Some(room)) = self.state.db.get_room(&room_id) {
                return Some(room.name);
            }
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

    /// Initialize session after authentication
    pub async fn init_session(&mut self, username: &str) {
        // Create player session
        self.player = Some(PlayerSession::new(username.to_string()));

        // Ensure agent has a corresponding Thing in world tree for inventory
        if let Err(e) = self.state.db.ensure_agent_thing(username) {
            tracing::warn!("Failed to create agent thing for '{}': {}", username, e);
        }

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
        let room_id = self.state.db.get_room_by_name(room_name)?.map(|r| r.id);

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
                let agent = self
                    .state
                    .db
                    .get_agent_by_name(&player.username)?
                    .ok_or_else(|| anyhow::anyhow!("agent not found: {}", player.username))?;
                lua.tool_state()
                    .set_session_context(Some(crate::lua::SessionContext {
                        agent_id: agent.id,
                        model: None,
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
                let agent = self
                    .state
                    .db
                    .get_agent_by_name(&player.username)?
                    .ok_or_else(|| anyhow::anyhow!("agent not found: {}", player.username))?;
                lua.tool_state()
                    .set_session_context(Some(crate::lua::SessionContext {
                        agent_id: agent.id,
                        model: None,
                        room_id: None,
                    }));
            }
        }

        Ok(())
    }

    /// Spawn background task for model response with tool support
    ///
    /// This is a thin wrapper around `ops::spawn_model_response` that passes
    /// the SSH session's update channel for streaming updates.
    pub async fn spawn_model_response(
        &self,
        model: ModelHandle,
        message: String,
        username: String,
        room_name: Option<String>,
        placeholder_row_id: Option<String>,
    ) -> Result<()> {
        let config = ModelResponseConfig {
            model,
            message,
            username,
            room_name,
            placeholder_row_id,
        };

        // Spawn via ops with SSH's update channel for streaming
        let _handle = spawn_model_response(
            self.state.clone(),
            config,
            self.lua_runtime.clone(),
            Some(self.update_tx.clone()),
        )
        .await?;

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
        // Subscribe to hot reload events for this session
        let lua_reload_rx = self.state.lua_reload.subscribe();
        spawn_screen_refresh(
            session.handle(),
            channel,
            self.lua_runtime.clone().expect("lua_runtime"),
            self.state.clone(),
            lua_reload_rx,
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
