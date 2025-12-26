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
use crate::player::PlayerSession;
use crate::state::SharedState;

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
                // Echo newline, execute command, show response with new prompt
                let _ = session.data(channel, CryptoVec::from(b"\r\n".as_slice()));

                let response = self.handle_input(&line).await;
                let prompt = self.get_prompt();

                // Response with proper line endings, then prompt
                let output = format!("{}\r\n\x1b[33m{}\x1b[0m ", response, prompt);
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
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
}
