//! SSH server and connection handler

use anyhow::Result;
use russh::server::{self, Handle, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec, Pty};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use crate::ansi::EscapeParser;
use crate::completion::{Completion, CompletionContext, CompletionEngine};
use crate::display::{
    DisplayBuffer, EntryContent, EntryId, EntrySource, Ledger, StatusKind,
    styles::{self, ctrl},
};
use crate::line_editor::{EditorAction, LineEditor};
use crate::player::PlayerSession;
use crate::state::SharedState;
use crate::world::{MessageContent, Sender};

/// Update from background task to resolve a placeholder
#[derive(Debug)]
pub struct LedgerUpdate {
    /// The placeholder entry ID to update
    pub placeholder_id: EntryId,
    /// Model name for display
    pub model_name: String,
    /// The response content
    pub content: String,
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

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Store handle and channel for async push
        self.session_handle = Some(session.handle());
        self.main_channel = Some(channel);

        if let Some(ref player) = self.player {
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

            // Render and send
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

            // Show initial prompt
            let prompt = self.get_prompt();
            let prompt_output = format!("{}{}{}  ", ctrl::CRLF, ctrl::CRLF, styles::prompt(&prompt));
            let _ = session.data(channel, CryptoVec::from(prompt_output.as_bytes()));
            {
                let mut display = self.display.lock().await;
                display.add_lines(2);
            }
        }

        // Spawn background task to push model responses
        if let Some(update_rx) = self.update_rx.take() {
            let handle = session.handle();
            let ledger = self.ledger.clone();
            let display = self.display.clone();
            let room_name = self.player.as_ref().and_then(|p| p.current_room.clone());

            tokio::spawn(async move {
                push_updates_task(handle, channel, update_rx, ledger, display, room_name).await;
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
async fn push_updates_task(
    handle: Handle,
    channel: ChannelId,
    mut update_rx: mpsc::Receiver<LedgerUpdate>,
    ledger: Arc<Mutex<Ledger>>,
    display: Arc<Mutex<DisplayBuffer>>,
    room_name: Option<String>,
) {
    while let Some(update) = update_rx.recv().await {
        // Update the ledger entry from Status to Chat
        {
            let mut ledger = ledger.lock().await;
            ledger.update(
                update.placeholder_id,
                EntryContent::Chat(update.content.clone()),
            );
            ledger.finalize(update.placeholder_id);
        }

        // In-place update: go back, clear, write new content
        let update_result = {
            let ledger = ledger.lock().await;
            let mut display = display.lock().await;
            display.render_placeholder_update(update.placeholder_id, &ledger)
        };

        if let Some((update_output, new_lines)) = update_result {
            let _ = handle.data(channel, CryptoVec::from(update_output.as_bytes())).await;
            {
                let mut display = display.lock().await;
                display.add_lines(new_lines.saturating_sub(1));
            }
        }

        // Redraw a simple prompt (user can continue typing)
        let prompt = match &room_name {
            Some(r) => format!("{}> ", r),
            None => "lobby> ".to_string(),
        };
        let prompt_output = format!("{}{}", ctrl::CRLF, styles::prompt(&prompt));
        let _ = handle.data(channel, CryptoVec::from(prompt_output.as_bytes())).await;
    }
}

impl SshHandler {
    /// Handle editor action result
    async fn handle_editor_action(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        action: EditorAction,
    ) -> Result<(), anyhow::Error> {
        match action {
            EditorAction::None => {}
            EditorAction::Redraw => {
                // Clear completions when user types
                self.clear_completions();
                let prompt = self.get_prompt();
                let output = self.editor.render(&prompt);
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            }
            EditorAction::Execute(line) => {
                // Echo newline
                let _ = session.data(channel, CryptoVec::from(b"\r\n".as_slice()));

                // Async MUD-style: fire off to model and return immediately
                if line.trim().starts_with('@') {
                    self.handle_mention_async(channel, session, &line).await?;
                    // Show prompt immediately so user can keep chatting
                    let prompt = self.get_prompt();
                    let output = format!("\r\n\x1b[33m{}\x1b[0m ", prompt);
                    let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                } else {
                    // Track room before command to detect /join
                    let room_before = self.player.as_ref().and_then(|p| p.current_room.clone());

                    let response = self.handle_input(&line).await;

                    // Check if we joined a new room - if so, load history into ledger
                    let room_after = self.player.as_ref().and_then(|p| p.current_room.clone());
                    if room_after != room_before {
                        if let Some(ref room) = room_after {
                            self.load_history_into_ledger(room).await;

                            // Render the history entries
                            let (history_output, lines) = {
                                let ledger = self.ledger.lock().await;
                                let mut display = self.display.lock().await;
                                display.render_incremental(&ledger)
                            };
                            let _ = session.data(channel, CryptoVec::from(history_output.as_bytes()));
                            {
                                let mut display = self.display.lock().await;
                                display.add_lines(lines);
                            }
                        }
                    }

                    let prompt = self.get_prompt();

                    // Response with proper line endings, then prompt
                    let output = format!("{}\r\n\x1b[33m{}\x1b[0m ", response, prompt);
                    let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                }
            }
            EditorAction::Tab => {
                self.handle_tab_completion(channel, session).await?;
            }
            EditorAction::ClearScreen => {
                // Clear screen and redraw prompt
                let prompt = self.get_prompt();
                let output = format!("\x1b[2J\x1b[H\x1b[33m{}\x1b[0m ", prompt);
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            }
            EditorAction::Quit => {
                // Send goodbye and close
                let _ = session.data(
                    channel,
                    CryptoVec::from(b"\r\nGoodbye!\r\n".as_slice()),
                );
                session.close(channel)?;
            }
        }
        Ok(())
    }

    /// Get current prompt string
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

            // Redraw the input line
            let prompt = self.get_prompt();
            let output = self.editor.render(&prompt);
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
            // Add placeholder for model response (mutable - will be updated)
            ledger.push_mutable(
                EntrySource::Model {
                    name: model.short_name.clone(),
                    is_streaming: false,
                },
                EntryContent::Status(StatusKind::Thinking),
            )
        };

        // Render and send both entries
        let (output, lines) = {
            let ledger = self.ledger.lock().await;
            let mut display = self.display.lock().await;
            display.render_incremental(&ledger)
        };
        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));

        // Register placeholder for in-place update tracking
        {
            let mut display = self.display.lock().await;
            display.register_placeholder(placeholder_id);
            display.add_lines(lines);
        }

        // Build context from room history
        let history = if let Some(ref room_name) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
            let world = self.state.world.read().await;
            if let Some(room) = world.get_room(room_name) {
                room.recent_history(10)
                    .iter()
                    .filter_map(|msg| match &msg.content {
                        MessageContent::Chat(text) => {
                            let role = match &msg.sender {
                                Sender::User(_) => "user",
                                Sender::Model(_) => "assistant",
                                Sender::System => return None,
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
            "You are {} in the sshwarma collaborative chat. \
             You're conversing with {} and possibly other users. \
             Be helpful, concise, and friendly. Keep responses under 500 words unless asked for more detail.",
            model.display_name, username
        );

        // Clone what we need for the background task
        let llm = self.state.llm.clone();
        let world = self.state.world.clone();
        let db = self.state.db.clone();
        let update_tx = self.update_tx.clone();
        let room_name = self.player.as_ref().and_then(|p| p.current_room.clone());
        let model_short = model.short_name.clone();
        let message_owned = message.to_string();

        // Spawn background task - returns immediately
        tokio::spawn(async move {
            // Get response from model
            let response = match llm
                .chat_with_context(&model, &system_prompt, &history, &message_owned)
                .await
            {
                Ok(r) => r,
                Err(e) => format!("[error: {}]", e),
            };

            // Save to room history
            if let Some(ref room) = room_name {
                {
                    let mut world = world.write().await;
                    if let Some(r) = world.get_room_mut(room) {
                        r.add_message(
                            Sender::Model(model_short.clone()),
                            MessageContent::Chat(response.clone()),
                        );
                    }
                }
                let _ = db.add_message(room, "model", &model_short, "chat", &response);
            }

            // Send ledger update to resolve the placeholder
            let _ = update_tx
                .send(LedgerUpdate {
                    placeholder_id,
                    model_name: model_short,
                    content: response,
                })
                .await;
        });

        Ok(())
    }

    /// Load room history into the ledger (called after /join)
    pub async fn load_history_into_ledger(&mut self, room_name: &str) {
        if let Ok(messages) = self.state.db.recent_messages(room_name, 20) {
            if messages.is_empty() {
                return;
            }

            let mut ledger = self.ledger.lock().await;

            // Add history separator
            ledger.push(
                EntrySource::System,
                EntryContent::HistorySeparator {
                    label: "Recent History".to_string(),
                },
            );

            // Add each historical message
            for msg in messages {
                let source = match msg.sender_type.as_str() {
                    "model" => EntrySource::Model {
                        name: msg.sender_name.clone(),
                        is_streaming: false,
                    },
                    _ => EntrySource::User(msg.sender_name.clone()),
                };
                ledger.push(source, EntryContent::Chat(msg.content.clone()));
            }

            // Add separator at end
            ledger.push(
                EntrySource::System,
                EntryContent::HistorySeparator {
                    label: "Now".to_string(),
                },
            );
        }
    }
}
