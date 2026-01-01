//! SSH server and connection handler

use anyhow::Result;
use chrono::Utc;
use rig::tool::server::ToolServer;
use russh::server::{self, Handle, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec, Pty};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, instrument, warn};

use crate::ansi::EscapeParser;
use crate::completion::{Completion, CompletionContext, CompletionEngine};
use crate::display::{
    DisplayBuffer, EntryContent, EntryId, EntrySource, Ledger, StatusKind,
    hud::{HudState, McpConnectionState, ParticipantStatus, HUD_HEIGHT},
    styles::ctrl,
};
use crate::llm::normalize_schema_for_llamacpp;
use crate::lua::{mcp_request_handler, register_mcp_tools, LuaRuntime, McpBridge, WrapState};
use crate::internal_tools::{InternalToolConfig, ToolContext};
use crate::line_editor::{EditorAction, LineEditor};
use crate::player::PlayerSession;
use crate::state::SharedState;

/// Update from background task for streaming responses
#[derive(Debug)]
pub enum LedgerUpdate {
    /// Incremental text chunk (for streaming)
    Chunk {
        placeholder_id: EntryId,
        text: String,
    },
    /// Tool being invoked
    ToolCall {
        placeholder_id: EntryId,
        tool_name: String,
    },
    /// Tool result received
    ToolResult {
        placeholder_id: EntryId,
        summary: String,
    },
    /// Stream completed, finalize the entry
    Complete {
        placeholder_id: EntryId,
        model_name: String,
    },
    /// Legacy: complete response (non-streaming)
    FullResponse {
        placeholder_id: EntryId,
        model_name: String,
        content: String,
    },
}

/// SSH server implementation
#[derive(Clone)]
pub struct SshServer {
    pub state: Arc<SharedState>,
}

impl server::Server for SshServer {
    type Handler = SshHandler;

    fn new_client(&mut self, peer_addr: Option<SocketAddr>) -> Self::Handler {
        info!(?peer_addr, "new connection");
        // Channel for receiving ledger updates from background tasks
        let (update_tx, update_rx) = mpsc::channel(32);
        SshHandler {
            state: self.state.clone(),
            player: None,
            editor: LineEditor::new(),
            esc_parser: EscapeParser::new(),
            term_size: (80, 24),
            completer: CompletionEngine::new(self.state.clone()),
            completions: Vec::new(),
            completion_index: 0,
            ledger: Arc::new(Mutex::new(Ledger::new(500))),
            display: Arc::new(Mutex::new(DisplayBuffer::new(80))),
            update_tx,
            update_rx: Some(update_rx),
            session_handle: None,
            main_channel: None,
            hud_state: Arc::new(Mutex::new(HudState::new())),
            lua_runtime: None, // Created after auth with username
            mcp_bridge: None,  // Created after auth with lua_runtime
            mcp_request_rx: None,
        }
    }

    fn handle_session_error(&mut self, error: <Self::Handler as server::Handler>::Error) {
        tracing::error!("session error: {:?}", error);
    }
}

/// Per-connection SSH handler
pub struct SshHandler {
    pub state: Arc<SharedState>,
    pub player: Option<PlayerSession>,
    /// Rich line editor with history and cursor movement
    pub editor: LineEditor,
    /// Escape sequence parser for arrow keys, etc.
    pub esc_parser: EscapeParser,
    /// Terminal dimensions (cols, rows)
    pub term_size: (u16, u16),
    /// Completion engine
    pub completer: CompletionEngine,
    /// Current completion candidates
    pub completions: Vec<Completion>,
    /// Selected completion index
    pub completion_index: usize,
    /// In-memory conversation ledger (shared with background task)
    pub ledger: Arc<Mutex<Ledger>>,
    /// Display buffer for incremental rendering (shared with background task)
    pub display: Arc<Mutex<DisplayBuffer>>,
    /// Sender for ledger updates from background tasks
    pub update_tx: mpsc::Sender<LedgerUpdate>,
    /// Receiver for ledger updates (taken by background task in shell_request)
    pub update_rx: Option<mpsc::Receiver<LedgerUpdate>>,
    /// Session handle for async push (set in shell_request)
    pub session_handle: Option<Handle>,
    /// Main channel ID for output (set in shell_request)
    pub main_channel: Option<ChannelId>,
    /// HUD state (shared with refresh task)
    pub hud_state: Arc<Mutex<HudState>>,
    /// Lua runtime for HUD rendering (per-connection, created after auth)
    pub lua_runtime: Option<Arc<Mutex<LuaRuntime>>>,
    /// MCP bridge for async Lua→MCP tool calls (per-connection)
    pub mcp_bridge: Option<Arc<McpBridge>>,
    /// Receiver for MCP requests (taken by handler task in shell_request)
    pub mcp_request_rx: Option<mpsc::Receiver<crate::lua::mcp_bridge::McpRequest>>,
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

        let handle = self
            .state
            .db
            .lookup_handle_by_pubkey(&key_str)
            .ok()
            .flatten()
            .unwrap_or_else(|| ssh_user.to_string());

        info!(handle, "authenticated");

        let _ = self.state.db.touch_user(&handle);

        let player = PlayerSession::new(handle.clone());
        let _ = self.state.db.start_session(&player.session_id, &handle);
        self.player = Some(player);

        // Create Lua runtime with user-specific script lookup
        let lua_runtime = Arc::new(Mutex::new(
            LuaRuntime::new_for_user(Some(&handle))
                .expect("failed to create Lua runtime"),
        ));

        // Create MCP bridge for async Lua→MCP tool calls
        let (mcp_bridge, mcp_request_rx) = McpBridge::with_defaults();
        let mcp_bridge = Arc::new(mcp_bridge);

        // Register MCP tools with Lua runtime
        {
            let lua = lua_runtime.lock().await;
            register_mcp_tools(lua.lua(), mcp_bridge.clone())
                .expect("failed to register MCP tools");
        }

        self.lua_runtime = Some(lua_runtime);
        self.mcp_bridge = Some(mcp_bridge);
        self.mcp_request_rx = Some(mcp_request_rx);

        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.term_size = (col_width as u16, row_height as u16);
        self.editor.set_width(col_width as u16);
        session.channel_success(channel)?;
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
        self.editor.set_width(col_width as u16);
        Ok(())
    }

    #[instrument(
        name = "ssh.shell_request",
        skip(self, session),
        fields(
            user.name = self.player.as_ref().map(|p| p.username.as_str()).unwrap_or("unknown"),
            term.width = self.term_size.0,
            term.height = self.term_size.1,
        )
    )]
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Store handle and channel for async push
        self.session_handle = Some(session.handle());
        self.main_channel = Some(channel);

        let (width, height) = self.term_size;

        if let Some(ref player) = self.player {
            // Set up screen: clear, set scroll region, position cursor
            let mut setup = String::new();
            setup.push_str(&ctrl::clear_screen());
            // Scroll region: rows 1 to height-9 (leave 8 for HUD + 1 for input)
            let scroll_bottom = height.saturating_sub(HUD_HEIGHT + 1);
            setup.push_str(&ctrl::set_scroll_region(1, scroll_bottom));
            // Move to top of scroll region
            setup.push_str(&ctrl::move_to(1, 1));
            let _ = session.data(channel, CryptoVec::from(setup.as_bytes()));

            // Initialize HUD state
            {
                let mut hud = self.hud_state.lock().await;
                hud.add_user(player.username.clone());
                if let Some(room_name) = &player.current_room {
                    let world = self.state.world.read().await;
                    if let Some(room) = world.get_room(room_name) {
                        hud.set_room(
                            Some(room_name.clone()),
                            room.description.clone(),
                            room.context.vibe.clone(),
                            room.context.exits.clone(),
                        );
                        // Add other users and models
                        for user in &room.users {
                            if user != &player.username {
                                hud.add_user(user.clone());
                            }
                        }
                        for model in &room.models {
                            hud.add_model(model.short_name.clone());
                        }
                    }
                } else {
                    hud.set_room(None, None, None, std::collections::HashMap::new());
                }

                // Initialize MCP connections
                let mcp_connections = self.state.mcp.list_connections().await;
                hud.set_mcp_connections(
                    mcp_connections
                        .into_iter()
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

            // Add welcome to ledger
            {
                let mut ledger = self.ledger.lock().await;
                ledger.push(
                    EntrySource::System,
                    EntryContent::Welcome {
                        username: player.username.clone(),
                    },
                );
            }

            // Render ledger in scroll region
            let output = {
                let ledger = self.ledger.lock().await;
                let mut display = self.display.lock().await;
                display.set_width(width);
                let (rendered, _) = display.render_full(&ledger);
                rendered
            };
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));

            // Move to HUD position and draw HUD
            let hud_start_row = height.saturating_sub(HUD_HEIGHT);
            let hud_output = {
                let hud = self.hud_state.lock().await;
                let lua_runtime = self.lua_runtime.as_ref().expect("lua_runtime not initialized");
                let lua = lua_runtime.lock().await;
                lua.update_state(hud.clone());
                let now_ms = Utc::now().timestamp_millis();
                let rendered = lua.render_hud_string(now_ms, width, height)
                    .unwrap_or_else(|e| format!("HUD error: {}", e));
                format!(
                    "{}{}",
                    ctrl::move_to(hud_start_row, 1),
                    rendered
                )
            };
            let _ = session.data(channel, CryptoVec::from(hud_output.as_bytes()));
        }

        // Spawn background task to push model responses
        if let Some(update_rx) = self.update_rx.take() {
            let handle = session.handle();
            let ledger = self.ledger.clone();
            let display = self.display.clone();
            let hud_state_for_updates = self.hud_state.clone();
            let lua_runtime_for_updates = self.lua_runtime.clone().expect("lua_runtime not initialized");
            let term_width = width;
            let term_height = height;

            tokio::spawn(async move {
                push_updates_task(handle, channel, update_rx, ledger, display, hud_state_for_updates, lua_runtime_for_updates, term_width, term_height).await;
            });
        }

        // Spawn HUD refresh task (100ms interval, Lua renders)
        {
            let handle = session.handle();
            let hud_state = self.hud_state.clone();
            let lua_runtime = self.lua_runtime.clone().expect("lua_runtime not initialized");
            let state = self.state.clone();
            let term_width = width;
            let term_height = height;

            tokio::spawn(async move {
                hud_refresh_task(handle, channel, hud_state, lua_runtime, state, term_width, term_height).await;
            });
        }

        // Spawn MCP request handler task (processes async MCP calls from Lua)
        if let Some(mcp_request_rx) = self.mcp_request_rx.take() {
            let mcp_clients = self.state.mcp.clone();
            let mcp_bridge = self.mcp_bridge.clone().expect("mcp_bridge not initialized");
            let timeout = mcp_bridge.timeout();
            let requests = mcp_bridge.requests();

            tokio::spawn(async move {
                mcp_request_handler(mcp_request_rx, mcp_clients, requests, timeout).await;
            });
        }

        // Spawn background tick task (500ms interval, calls Lua background())
        {
            let lua_runtime = self.lua_runtime.clone().expect("lua_runtime not initialized");

            tokio::spawn(async move {
                background_tick_task(lua_runtime).await;
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
        for &byte in data {
            // Parse through escape sequence handler
            if let Some(event) = self.esc_parser.feed(byte) {
                let action = self.editor.handle_event(event);
                self.handle_editor_action(channel, session, action).await?;
            }
        }
        Ok(())
    }
}

/// Background task that watches for model responses and pushes them to the terminal
#[instrument(name = "ssh.push_updates", skip_all)]
async fn push_updates_task(
    handle: Handle,
    channel: ChannelId,
    mut update_rx: mpsc::Receiver<LedgerUpdate>,
    ledger: Arc<Mutex<Ledger>>,
    display: Arc<Mutex<DisplayBuffer>>,
    hud_state: Arc<Mutex<HudState>>,
    lua_runtime: Arc<Mutex<LuaRuntime>>,
    term_width: u16,
    term_height: u16,
) {
    let scroll_bottom = term_height.saturating_sub(HUD_HEIGHT + 1);
    let hud_start_row = term_height.saturating_sub(HUD_HEIGHT);

    while let Some(update) = update_rx.recv().await {
        // Process the update based on type
        let needs_redraw = match update {
            LedgerUpdate::Chunk { placeholder_id, text } => {
                let mut ledger = ledger.lock().await;
                ledger.append(placeholder_id, &text)
            }
            LedgerUpdate::ToolCall { placeholder_id: _, tool_name: _ } => {
                // Status shown in HUD, not in ledger
                false
            }
            LedgerUpdate::ToolResult { placeholder_id, summary } => {
                let mut ledger = ledger.lock().await;
                // Append tool result summary (status shown in HUD)
                ledger.append(placeholder_id, &format!("\n[{}]\n", summary))
            }
            LedgerUpdate::Complete { placeholder_id, model_name: _ } => {
                let mut ledger = ledger.lock().await;
                ledger.finalize(placeholder_id);
                true
            }
            LedgerUpdate::FullResponse { placeholder_id, model_name: _, content } => {
                let mut ledger = ledger.lock().await;
                ledger.update(placeholder_id, EntryContent::Chat(content));
                ledger.finalize(placeholder_id);
                true
            }
        };

        if !needs_redraw {
            continue;
        }

        // Re-render the full ledger in scroll region
        let rendered = {
            let ledger = ledger.lock().await;
            let mut display = display.lock().await;
            let (output, _) = display.render_full(&ledger);
            output
        };

        // Build output: save cursor, go to scroll region, clear & redraw, restore cursor
        let mut output = String::new();
        output.push_str(&ctrl::save_cursor());
        output.push_str(&ctrl::move_to(1, 1));
        // Clear scroll region and redraw
        for _ in 0..scroll_bottom {
            output.push_str(&ctrl::clear_line());
            output.push_str(ctrl::CRLF);
        }
        output.push_str(&ctrl::move_to(1, 1));
        output.push_str(&rendered);
        output.push_str(&ctrl::restore_cursor());

        let _ = handle.data(channel, CryptoVec::from(output.as_bytes())).await;

        // Redraw HUD at bottom
        let hud_output = {
            let hud = hud_state.lock().await;
            let lua = lua_runtime.lock().await;
            lua.update_state(hud.clone());
            let now_ms = Utc::now().timestamp_millis();
            let rendered = lua.render_hud_string(now_ms, term_width, term_height)
                .unwrap_or_else(|e| format!("HUD error: {}", e));
            format!(
                "{}{}",
                ctrl::move_to(hud_start_row, 1),
                rendered
            )
        };
        let _ = handle.data(channel, CryptoVec::from(hud_output.as_bytes())).await;
    }
}

/// Background task that refreshes the HUD at 100ms intervals
///
/// Tiered update schedule:
/// - Every tick (100ms): Lua renders HUD (handles spinner, notifications)
/// - Every 10 ticks (1s): Poll MCP connection status, check Lua hot-reload
#[instrument(name = "ssh.hud_refresh", skip_all)]
async fn hud_refresh_task(
    handle: Handle,
    channel: ChannelId,
    hud_state: Arc<Mutex<HudState>>,
    lua_runtime: Arc<Mutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    let hud_start_row = term_height.saturating_sub(HUD_HEIGHT);
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    let mut tick_count: u64 = 0;

    loop {
        interval.tick().await;
        tick_count = tick_count.wrapping_add(1);

        // Every 10 ticks (1 second): poll MCP status and check hot-reload
        if tick_count % 10 == 0 {
            let mcp_connections = state.mcp.list_connections().await;
            let mut hud = hud_state.lock().await;
            hud.set_mcp_connections(
                mcp_connections
                    .into_iter()
                    .map(|c| McpConnectionState {
                        name: c.name,
                        tool_count: c.tool_count,
                        connected: true,
                        call_count: c.call_count,
                        last_tool: c.last_tool,
                    })
                    .collect(),
            );

            // Check for Lua script hot-reload
            let mut lua = lua_runtime.lock().await;
            lua.check_reload();
        }

        // Render and send HUD (Lua manages spinner, notifications, timing)
        let hud_output = {
            let hud = hud_state.lock().await;
            let lua = lua_runtime.lock().await;
            lua.update_state(hud.clone());
            let now_ms = Utc::now().timestamp_millis();
            let rendered = lua.render_hud_string(now_ms, term_width, term_height)
                .unwrap_or_else(|e| format!("HUD error: {}", e));
            format!(
                "{}{}{}{}",
                ctrl::save_cursor(),
                ctrl::move_to(hud_start_row, 1),
                rendered,
                ctrl::restore_cursor()
            )
        };
        let _ = handle.data(channel, CryptoVec::from(hud_output.as_bytes())).await;
    }
}

/// Background tick task - calls Lua background() at 120 BPM (500ms)
///
/// This allows Lua scripts to poll MCP tools and update state on a regular
/// interval. The tick counter enables subdivision timing:
/// - tick % 1 == 0: every 500ms
/// - tick % 2 == 0: every 1s
/// - tick % 4 == 0: every 2s
/// - tick % 8 == 0: every 4s
#[instrument(name = "ssh.background_tick", skip_all)]
async fn background_tick_task(lua_runtime: Arc<Mutex<LuaRuntime>>) {
    let tick_interval = Duration::from_millis(500); // 120 BPM
    let mut interval = tokio::time::interval(tick_interval);
    let mut tick: u64 = 0;

    loop {
        interval.tick().await;
        tick = tick.wrapping_add(1);

        let lua = lua_runtime.lock().await;
        if let Err(e) = lua.call_background(tick) {
            tracing::warn!("background() error: {}", e);
        }
    }
}

impl SshHandler {
    /// Redraw the entire screen with scroll region
    async fn redraw_screen(&self, channel: ChannelId, session: &mut Session) -> Result<(), anyhow::Error> {
        let (width, height) = self.term_size;
        let scroll_bottom = height.saturating_sub(HUD_HEIGHT + 1);
        let hud_start_row = height.saturating_sub(HUD_HEIGHT);

        // Render ledger content
        let rendered = {
            let ledger = self.ledger.lock().await;
            let mut display = self.display.lock().await;
            display.set_width(width);
            let (output, _) = display.render_full(&ledger);
            output
        };

        // Build screen output
        let mut output = String::new();
        // Move to top of scroll region and clear
        output.push_str(&ctrl::move_to(1, 1));
        for _ in 0..scroll_bottom {
            output.push_str(&ctrl::clear_line());
            output.push_str(ctrl::CRLF);
        }
        output.push_str(&ctrl::move_to(1, 1));
        output.push_str(&rendered);

        // Render HUD
        let hud_rendered = {
            let hud = self.hud_state.lock().await;
            let lua_runtime = self.lua_runtime.as_ref().expect("lua_runtime not initialized");
            let lua = lua_runtime.lock().await;
            lua.update_state(hud.clone());
            let now_ms = Utc::now().timestamp_millis();
            lua.render_hud_string(now_ms, width, height)
                .unwrap_or_else(|e| format!("HUD error: {}", e))
        };
        output.push_str(&ctrl::move_to(hud_start_row, 1));
        output.push_str(&hud_rendered);

        // Move cursor to input line (bare cursor, no prompt)
        output.push_str(&ctrl::move_to(height, 1));
        output.push_str(&ctrl::clear_line());
        output.push_str(self.editor.value());

        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
        Ok(())
    }

    /// Handle editor action result
    async fn handle_editor_action(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        action: EditorAction,
    ) -> Result<(), anyhow::Error> {
        let (_width, height) = self.term_size;
        let input_row = height; // Bare cursor input at bottom

        match action {
            EditorAction::None => {}
            EditorAction::Redraw => {
                // Clear completions when user types
                self.clear_completions();
                // Just redraw the input line at bottom (bare cursor, no prompt)
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
                    let output = format!(
                        "{}\r\nGoodbye!\r\n",
                        ctrl::reset_scroll_region()
                    );
                    let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                    session.close(channel)?;
                    return Ok(());
                }

                // Async MUD-style: fire off to model and return immediately
                if line.trim().starts_with('@') {
                    self.handle_mention_async(channel, session, &line).await?;
                    // handle_mention_async already rendered, just redraw input line
                    let output = format!(
                        "{}{}",
                        ctrl::move_to(input_row, 1),
                        ctrl::clear_line()
                    );
                    let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                } else {
                    // Track room before command to detect /join
                    let room_before = self.player.as_ref().and_then(|p| p.current_room.clone());

                    let result = self.handle_input(&line).await;

                    // Add command output to ledger if non-empty
                    if !result.is_empty() {
                        let mut ledger = self.ledger.lock().await;
                        let source = EntrySource::Command {
                            command: line.split_whitespace().next().unwrap_or("").to_string(),
                        };
                        let content = EntryContent::CommandOutput(result.text);

                        if result.ephemeral {
                            ledger.push_ephemeral(source, content);
                        } else {
                            ledger.push(source, content);
                        }
                    }

                    // Check if we joined a new room - if so, render room's ledger
                    let room_after = self.player.as_ref().and_then(|p| p.current_room.clone());
                    if room_after != room_before {
                        if let Some(ref room) = room_after {
                            self.render_room_history(room).await;
                        }
                    }

                    // Redraw full screen with updated ledger
                    self.redraw_screen(channel, session).await?;
                }
            }
            EditorAction::Tab => {
                self.handle_tab_completion(channel, session).await?;
            }
            EditorAction::ClearScreen => {
                // Full redraw
                self.redraw_screen(channel, session).await?;
            }
            EditorAction::Quit => {
                // Reset scroll region and say goodbye
                let output = format!(
                    "{}\r\nGoodbye!\r\n",
                    ctrl::reset_scroll_region()
                );
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                session.close(channel)?;
            }
        }
        Ok(())
    }

    /// Get current prompt string (may be used for HUD room indicator)
    #[allow(dead_code)]
    fn get_prompt(&self) -> String {
        self.player
            .as_ref()
            .map(|p| p.prompt())
            .unwrap_or_else(|| "lobby>".to_string())
    }

    /// Handle tab completion
    async fn handle_tab_completion(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), anyhow::Error> {
        let line = self.editor.value().to_string();
        let cursor = self.editor.cursor();
        let room: Option<String> = self
            .player
            .as_ref()
            .and_then(|p| p.current_room.clone());

        // If we already have completions and user pressed tab again, cycle
        if !self.completions.is_empty() {
            self.completion_index = (self.completion_index + 1) % self.completions.len();
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.apply_completion(channel, session, &ctx)?;
            return Ok(());
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
            // Single completion, apply it directly
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.apply_completion(channel, session, &ctx)?;
            self.completions.clear();
        } else {
            // Multiple completions, show menu and apply first
            self.show_completion_menu(channel, session)?;
            let ctx = CompletionContext {
                line: &line,
                cursor,
                room: room.as_deref(),
            };
            self.apply_completion(channel, session, &ctx)?;
        }

        Ok(())
    }

    /// Apply current completion to the editor
    fn apply_completion(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        ctx: &CompletionContext<'_>,
    ) -> Result<(), anyhow::Error> {
        if let Some(completion) = self.completions.get(self.completion_index) {
            let (start, _end) = self.completer.replacement_range(ctx);
            self.editor.replace_with_completion(start, &completion.text);

            // Add space after completion if it's a command or complete word
            if completion.text.starts_with('/') || completion.text.starts_with('@') {
                self.editor.insert_completion(" ");
            }

            // Redraw the input line (bare cursor, no prompt)
            let output = self.editor.render("");
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
        }
        Ok(())
    }

    /// Show completion menu below input
    fn show_completion_menu(
        &self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), anyhow::Error> {
        let mut output = String::new();

        // Save cursor, move to new line
        output.push_str("\x1b[s\r\n");

        // Show up to 10 completions
        let max_show = 10.min(self.completions.len());
        output.push_str("  ╭");
        output.push_str(&"─".repeat(40));
        output.push_str("╮\r\n");

        for (i, completion) in self.completions.iter().take(max_show).enumerate() {
            let marker = if i == self.completion_index {
                "→"
            } else {
                " "
            };
            // Truncate label to fit
            let label: String = completion.label.chars().take(38).collect();
            output.push_str(&format!("  │{} {:<38}│\r\n", marker, label));
        }

        if self.completions.len() > max_show {
            output.push_str(&format!(
                "  │  ... and {} more{:>21}│\r\n",
                self.completions.len() - max_show,
                ""
            ));
        }

        output.push_str("  ╰");
        output.push_str(&"─".repeat(40));
        output.push_str("╯");

        // Restore cursor
        output.push_str("\x1b[u");

        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
        Ok(())
    }

    /// Clear completions state (call when user types something else)
    pub fn clear_completions(&mut self) {
        self.completions.clear();
        self.completion_index = 0;
    }

    /// Handle @mention - spawn background task and return immediately
    async fn handle_mention_async(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        input: &str,
    ) -> Result<(), anyhow::Error> {
        let input = input.trim_start_matches('@');
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let model_name = parts.first().unwrap_or(&"");
        let message = parts.get(1).copied().unwrap_or("").trim();

        if message.is_empty() {
            // Add error to ledger
            {
                let mut ledger = self.ledger.lock().await;
                ledger.push(
                    EntrySource::System,
                    EntryContent::Error(format!("Usage: @{} <message>", model_name)),
                );
            }
            let (output, lines) = {
                let ledger = self.ledger.lock().await;
                let mut display = self.display.lock().await;
                display.render_incremental(&ledger)
            };
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            {
                let mut display = self.display.lock().await;
                display.add_lines(lines);
            }
            return Ok(());
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => {
                {
                    let mut ledger = self.ledger.lock().await;
                    ledger.push(
                        EntrySource::System,
                        EntryContent::Error("Not authenticated".into()),
                    );
                }
                let (output, lines) = {
                    let ledger = self.ledger.lock().await;
                    let mut display = self.display.lock().await;
                    display.render_incremental(&ledger)
                };
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                {
                    let mut display = self.display.lock().await;
                    display.add_lines(lines);
                }
                return Ok(());
            }
        };

        let model = match self.state.models.get(model_name) {
            Some(m) => m.clone(),
            None => {
                let available: Vec<_> = self
                    .state
                    .models
                    .available()
                    .iter()
                    .map(|m| m.short_name.as_str())
                    .collect();
                {
                    let mut ledger = self.ledger.lock().await;
                    ledger.push(
                        EntrySource::System,
                        EntryContent::Error(format!(
                            "Unknown model '{}'. Available: {}",
                            model_name,
                            available.join(", ")
                        )),
                    );
                }
                let (output, lines) = {
                    let ledger = self.ledger.lock().await;
                    let mut display = self.display.lock().await;
                    display.render_incremental(&ledger)
                };
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                {
                    let mut display = self.display.lock().await;
                    display.add_lines(lines);
                }
                return Ok(());
            }
        };

        // Add user's message and placeholder to ledger
        let placeholder_id = {
            let mut ledger = self.ledger.lock().await;
            ledger.push(
                EntrySource::User(username.clone()),
                EntryContent::Chat(format!("@{}: {}", model_name, message)),
            );
            // Add placeholder for model response (will be updated when response arrives)
            ledger.push_mutable(
                EntrySource::Model {
                    name: model.short_name.clone(),
                    is_streaming: false,
                },
                EntryContent::Status(StatusKind::Pending), // Invisible - status shown in HUD
            )
        };

        // Render full ledger in scroll region (like push_updates_task does)
        let (_, height) = self.term_size;
        let rendered = {
            let ledger = self.ledger.lock().await;
            let mut display = self.display.lock().await;
            let (output, _) = display.render_full(&ledger);
            output
        };

        // Clear scroll region and redraw content
        let mut output = String::new();
        output.push_str(&ctrl::move_to(1, 1));
        for _ in 0..height.saturating_sub(2) {
            output.push_str(&ctrl::clear_line());
            output.push_str(ctrl::CRLF);
        }
        output.push_str(&ctrl::move_to(1, 1));
        output.push_str(&rendered);
        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));

        // Build context using Lua wrap() system
        let room_name = self.player.as_ref().and_then(|p| p.current_room.clone());
        let (system_prompt, context_prefix) = {
            let lua_runtime = self.lua_runtime.as_ref().expect("lua_runtime not initialized");
            let lua = lua_runtime.lock().await;

            // Create WrapState for context composition
            let wrap_state = WrapState {
                room_name: room_name.clone(),
                username: username.clone(),
                model: model.clone(),
                shared_state: self.state.clone(),
            };

            // Get token budget from model's context_window (default 30K)
            let target_tokens = model.context_window.unwrap_or(30000);

            match lua.compose_context(wrap_state, target_tokens) {
                Ok(result) => (result.system_prompt, result.context),
                Err(e) => {
                    tracing::warn!("wrap() failed, falling back to basic prompt: {}", e);
                    // Fallback to minimal system prompt
                    let fallback = format!(
                        "You are {} in sshwarma. You are talking with {}.",
                        model.display_name, username
                    );
                    (fallback, String::new())
                }
            }
        };

        // Get MCP tools for rig agent
        let mcp_context = self.state.mcp.rig_tools().await;
        let needs_schema_strip = matches!(model.backend, crate::model::ModelBackend::LlamaCpp { .. });

        // Build ToolServer with MCP + internal tools
        let tool_server_handle = {
            let mut server = ToolServer::new();

            // Add MCP tools if available
            if let Some(ctx) = &mcp_context {
                for (tool, peer) in ctx.tools.iter() {
                    // Normalize schemas for llama.cpp (limited schema support)
                    let tool = if needs_schema_strip {
                        let original_schema = serde_json::to_string(&tool.input_schema).unwrap_or_default();
                        let normalized = normalize_schema_for_llamacpp(tool);
                        let normalized_schema = serde_json::to_string(&normalized.input_schema).unwrap_or_default();
                        if original_schema != normalized_schema {
                            tracing::info!("normalized schema for {}: {} -> {} bytes", tool.name, original_schema.len(), normalized_schema.len());
                        }
                        normalized
                    } else {
                        tool.clone()
                    };
                    server = server.rmcp_tool(tool, peer.clone());
                }
            }

            server.run()
        };

        // Always add internal sshwarma tools (read-only always, write tools in rooms)
        let in_room = room_name.is_some();
        let room_for_tools = room_name.clone().unwrap_or_else(|| "lobby".to_string());
        tracing::info!("registering internal tools for room: {} (write_tools={})", room_for_tools, in_room);
        let tool_ctx = ToolContext {
            state: self.state.clone(),
            room: room_for_tools.clone(),
            username: username.clone(),
            lua_runtime: self.lua_runtime.clone().expect("lua_runtime not initialized"),
        };
        // Use per-room config (navigation may be disabled for this room)
        let config = InternalToolConfig::for_room(&self.state, &room_for_tools).await;
        // Register tools individually (not append_toolset) so they appear in static_tool_names
        match crate::internal_tools::register_tools(&tool_server_handle, tool_ctx, &config, in_room).await {
            Ok(count) => tracing::info!("registered {} internal tools", count),
            Err(e) => tracing::error!("failed to register internal tools: {}", e),
        }

        // Get tool definitions and append to system prompt
        let system_prompt = match tool_server_handle.get_tool_defs(None).await {
            Ok(tool_defs) if !tool_defs.is_empty() => {
                let mut tool_guide = String::from("\n\n## Your Functions\n");
                tool_guide.push_str("You have these built-in functions:\n\n");
                for tool in &tool_defs {
                    // Strip sshwarma_ prefix for cleaner display
                    let display_name = tool.name.strip_prefix("sshwarma_").unwrap_or(&tool.name);
                    tool_guide.push_str(&format!("- **{}**: {}\n", display_name, tool.description));
                }
                tracing::info!("injecting {} function definitions into prompt", tool_defs.len());
                format!("{}{}", system_prompt, tool_guide)
            }
            Ok(_) => {
                // This should never happen - we always have at least read-only tools
                tracing::error!("BUG: 0 tools available - internal tools not registered!");
                system_prompt
            }
            Err(e) => {
                tracing::error!("failed to get tool definitions: {}", e);
                system_prompt
            }
        };

        // Set model status to Thinking in HUD
        {
            let mut hud = self.hud_state.lock().await;
            hud.update_status(&model.short_name, ParticipantStatus::Thinking);
        }

        // Clone what we need for the background task
        let llm = self.state.llm.clone();
        let world = self.state.world.clone();
        let db = self.state.db.clone();
        let update_tx = self.update_tx.clone();
        let hud_state = self.hud_state.clone();
        let model_short = model.short_name.clone();

        // Prepend context to user message (context is dynamic room/history state)
        let message_with_context = if context_prefix.is_empty() {
            message.to_string()
        } else {
            format!("{}\n\n---\n\n{}", context_prefix, message)
        };
        let message_owned = message_with_context;

        // Spawn background task - returns immediately
        tokio::spawn(async move {
            use crate::llm::StreamChunk;

            // Create channel for streaming chunks
            let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<StreamChunk>(32);

            // Spawn the streaming LLM call
            let stream_handle = tokio::spawn({
                let llm = llm.clone();
                let model = model.clone();
                let system_prompt = system_prompt.clone();
                let message = message_owned.clone();
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

            // Collect full response while streaming chunks to display
            let mut full_response = String::new();

            while let Some(chunk) = chunk_rx.recv().await {
                match chunk {
                    StreamChunk::Text(text) => {
                        full_response.push_str(&text);
                        let _ = update_tx.send(LedgerUpdate::Chunk {
                            placeholder_id,
                            text,
                        }).await;
                    }
                    StreamChunk::ToolCall(name) => {
                        // Update HUD: model is running a tool
                        {
                            let mut hud = hud_state.lock().await;
                            hud.update_status(&model_short, ParticipantStatus::RunningTool(name.clone()));
                        }
                        let _ = update_tx.send(LedgerUpdate::ToolCall {
                            placeholder_id,
                            tool_name: name,
                        }).await;
                    }
                    StreamChunk::ToolResult(summary) => {
                        // Update HUD: back to thinking after tool result
                        {
                            let mut hud = hud_state.lock().await;
                            hud.update_status(&model_short, ParticipantStatus::Thinking);
                        }
                        let _ = update_tx.send(LedgerUpdate::ToolResult {
                            placeholder_id,
                            summary,
                        }).await;
                    }
                    StreamChunk::Done => {
                        break;
                    }
                    StreamChunk::Error(e) => {
                        full_response = format!("[error: {}]", e);
                        break;
                    }
                }
            }

            // Update HUD: model is idle again
            {
                let mut hud = hud_state.lock().await;
                hud.update_status(&model_short, ParticipantStatus::Idle);
            }

            // Wait for stream task to complete
            let _ = stream_handle.await;

            // Save to room history
            if let Some(ref room) = room_name {
                use crate::display::LedgerEntry;
                use chrono::Utc;

                let entry = LedgerEntry {
                    id: crate::display::EntryId(0), // Will be assigned
                    timestamp: Utc::now(),
                    source: EntrySource::Model {
                        name: model_short.clone(),
                        is_streaming: false,
                    },
                    content: EntryContent::Chat(full_response.clone()),
                    mutable: false,
                    ephemeral: false,
                    collapsible: true,
                };

                {
                    let mut world = world.write().await;
                    if let Some(r) = world.get_room_mut(room) {
                        r.add_entry(entry.source.clone(), entry.content.clone());
                    }
                }
                let _ = db.add_ledger_entry(room, &entry);
            }

            // Send completion to finalize the placeholder
            let _ = update_tx
                .send(LedgerUpdate::Complete {
                    placeholder_id,
                    model_name: model_short,
                })
                .await;
        });

        Ok(())
    }

    /// Render room's ledger into session ledger (called after /join)
    pub async fn render_room_history(&mut self, room_name: &str) {
        let world = self.state.world.read().await;
        if let Some(room) = world.get_room(room_name) {
            let mut session_ledger = self.ledger.lock().await;

            // Copy room's ledger entries into session ledger
            for entry in room.ledger.all() {
                session_ledger.push(entry.source.clone(), entry.content.clone());
            }
        }
    }
}
