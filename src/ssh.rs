//! SSH server and connection handler

use anyhow::Result;
use russh::server::{self, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec, Pty};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};

use crate::ansi::EscapeParser;
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
                // TODO: Tab completion
                // For now, just beep
                let _ = session.data(channel, CryptoVec::from(b"\x07".as_slice()));
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
}
