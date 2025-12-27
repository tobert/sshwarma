//! SSH server and connection handler

use anyhow::Result;
use russh::server::{self, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec, Pty};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::ansi::EscapeParser;
use crate::completion::{Completion, CompletionContext, CompletionEngine};
use crate::display::{
    DisplayBuffer, EntryContent, EntryId, EntrySource, Ledger, StatusKind,
    styles::ctrl,
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
            ledger: Ledger::new(500),
            display: DisplayBuffer::new(80),
            update_tx,
            update_rx,
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
    /// In-memory conversation ledger
    pub ledger: Ledger,
    /// Display buffer for incremental rendering
    pub display: DisplayBuffer,
    /// Sender for ledger updates from background tasks
    pub update_tx: mpsc::Sender<LedgerUpdate>,
    /// Receiver for ledger updates
    pub update_rx: mpsc::Receiver<LedgerUpdate>,
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
        if let Some(ref player) = self.player {
            // Add welcome to ledger
            self.ledger.push(
                EntrySource::System,
                EntryContent::Welcome {
                    username: player.username.clone(),
                },
            );

            // Render and send
            let (output, lines) = self.display.render_incremental(&self.ledger);
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            self.display.add_lines(lines);

            // Show initial prompt
            let prompt = self.get_prompt();
            let prompt_output = format!("{}{}{}  ", ctrl::CRLF, ctrl::CRLF, crate::display::styles::prompt(&prompt));
            let _ = session.data(channel, CryptoVec::from(prompt_output.as_bytes()));
            self.display.add_lines(2);
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Check for any ledger updates from background tasks (model responses)
        self.flush_ledger_updates(channel, session)?;

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
                    let response = self.handle_input(&line).await;
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
            self.ledger.push(
                EntrySource::System,
                EntryContent::Error(format!("Usage: @{} <message>", model_name)),
            );
            let (output, lines) = self.display.render_incremental(&self.ledger);
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            self.display.add_lines(lines);
            return Ok(());
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => {
                self.ledger.push(
                    EntrySource::System,
                    EntryContent::Error("Not authenticated".into()),
                );
                let (output, lines) = self.display.render_incremental(&self.ledger);
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                self.display.add_lines(lines);
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
                self.ledger.push(
                    EntrySource::System,
                    EntryContent::Error(format!(
                        "Unknown model '{}'. Available: {}",
                        model_name,
                        available.join(", ")
                    )),
                );
                let (output, lines) = self.display.render_incremental(&self.ledger);
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                self.display.add_lines(lines);
                return Ok(());
            }
        };

        // Add user's message to ledger
        self.ledger.push(
            EntrySource::User(username.clone()),
            EntryContent::Chat(format!("@{}: {}", model_name, message)),
        );

        // Add placeholder for model response (mutable - will be updated)
        let placeholder_id = self.ledger.push_mutable(
            EntrySource::Model {
                name: model.short_name.clone(),
                is_streaming: false,
            },
            EntryContent::Status(StatusKind::Thinking),
        );

        // Render and send both entries
        let (output, lines) = self.display.render_incremental(&self.ledger);
        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));

        // Register placeholder for in-place update tracking
        self.display.register_placeholder(placeholder_id);
        self.display.add_lines(lines);

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

    /// Flush any pending ledger updates (model responses) to the terminal
    fn flush_ledger_updates(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), anyhow::Error> {
        let mut got_any = false;

        // Non-blocking check for pending updates
        while let Ok(update) = self.update_rx.try_recv() {
            got_any = true;

            // Update the ledger entry from Status to Chat
            self.ledger.update(
                update.placeholder_id,
                EntryContent::Chat(update.content.clone()),
            );
            self.ledger.finalize(update.placeholder_id);

            // In-place update: go back, clear, write new content
            if let Some((update_output, new_lines)) =
                self.display.render_placeholder_update(update.placeholder_id, &self.ledger)
            {
                let _ = session.data(channel, CryptoVec::from(update_output.as_bytes()));
                self.display.add_lines(new_lines.saturating_sub(1));
            }
        }

        // If we displayed any updates, redraw the prompt and current input
        if got_any {
            let prompt = self.get_prompt();
            let output = format!("{}{}", ctrl::CRLF, self.editor.render(&prompt));
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            self.display.add_lines(1);
        }

        Ok(())
    }
}
