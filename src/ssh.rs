//! SSH server and connection handler

use anyhow::Result;
use russh::server::{self, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec, Pty};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};

use crate::ansi::EscapeParser;
use crate::completion::{Completion, CompletionContext, CompletionEngine};
use crate::line_editor::{EditorAction, LineEditor};
use crate::llm::StreamChunk;
use crate::player::PlayerSession;
use crate::state::SharedState;
use crate::world::{MessageContent, Sender};

/// SSH server implementation
#[derive(Clone)]
pub struct SshServer {
    pub state: Arc<SharedState>,
}

impl server::Server for SshServer {
    type Handler = SshHandler;

    fn new_client(&mut self, peer_addr: Option<SocketAddr>) -> Self::Handler {
        info!(?peer_addr, "new connection");
        SshHandler {
            state: self.state.clone(),
            player: None,
            editor: LineEditor::new(),
            esc_parser: EscapeParser::new(),
            term_size: (80, 24),
            completer: CompletionEngine::new(self.state.clone()),
            completions: Vec::new(),
            completion_index: 0,
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
            let welcome = format!(
                "\x1b[1;36m╭─────────────────────────────────────╮\x1b[0m\r\n\
                 \x1b[1;36m│           sshwarma                  │\x1b[0m\r\n\
                 \x1b[1;36m╰─────────────────────────────────────╯\x1b[0m\r\n\r\n\
                 Welcome, {}.\r\n\r\n\
                 /rooms to list rooms, /join <room> to enter\r\n\r\n\
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

                // Use streaming for @mentions
                if line.trim().starts_with('@') {
                    self.handle_mention_stream(channel, session, &line).await?;
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

    /// Handle @mention with streaming response
    async fn handle_mention_stream(
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
            let msg = format!("Usage: @{} <message>\r\n", model_name);
            let _ = session.data(channel, CryptoVec::from(msg.as_bytes()));
            return Ok(());
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => {
                let _ = session.data(channel, CryptoVec::from(b"Not authenticated\r\n".as_slice()));
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
                let msg = format!(
                    "Unknown model '{}'. Available: {}\r\n",
                    model_name,
                    available.join(", ")
                );
                let _ = session.data(channel, CryptoVec::from(msg.as_bytes()));
                return Ok(());
            }
        };

        // Show user message
        let header = format!("{} → @{}: {}\r\n\r\n{}: ", username, model_name, message, model.short_name);
        let _ = session.data(channel, CryptoVec::from(header.as_bytes()));

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

        // Create streaming channel
        let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamChunk>(32);

        // Spawn the streaming task
        let llm = self.state.llm.clone();
        let model_clone = model.clone();
        let history_clone = history.clone();
        let message_clone = message.to_string();
        let system_prompt_clone = system_prompt.clone();

        tokio::spawn(async move {
            let _ = llm.chat_stream(
                &model_clone,
                &system_prompt_clone,
                &history_clone,
                &message_clone,
                tx,
            ).await;
        });

        // Collect full response for history
        let mut full_response = String::new();

        // Stream chunks to terminal
        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::Text(text) => {
                    full_response.push_str(&text);
                    // Convert newlines for terminal
                    let display = text.replace('\n', "\r\n");
                    let _ = session.data(channel, CryptoVec::from(display.as_bytes()));
                }
                StreamChunk::Done => break,
                StreamChunk::Error(e) => {
                    let msg = format!("\r\n[error: {}]", e);
                    let _ = session.data(channel, CryptoVec::from(msg.as_bytes()));
                    break;
                }
            }
        }

        // Save to room history
        if let Some(ref room_name) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
            {
                let mut world = self.state.world.write().await;
                if let Some(room) = world.get_room_mut(room_name) {
                    room.add_message(
                        Sender::Model(model.short_name.clone()),
                        MessageContent::Chat(full_response.clone()),
                    );
                }
            }
            let _ = self.state.db.add_message(
                room_name,
                "model",
                &model.short_name,
                "chat",
                &full_response,
            );
        }

        // Show prompt after response
        let prompt = self.get_prompt();
        let prompt_line = format!("\r\n\r\n\x1b[33m{}\x1b[0m ", prompt);
        let _ = session.data(channel, CryptoVec::from(prompt_line.as_bytes()));

        Ok(())
    }
}
