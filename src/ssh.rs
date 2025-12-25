//! SSH server and connection handler

use anyhow::Result;
use russh::server::{self, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};

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
            line_buffer: String::new(),
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
    pub line_buffer: String,
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
                 /rooms to list partylines, /join <room> to enter\r\n\r\n\
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
            match byte {
                // Enter/Return
                b'\r' | b'\n' => {
                    if !self.line_buffer.is_empty() {
                        let input = std::mem::take(&mut self.line_buffer);
                        let response = self.handle_input(&input).await;
                        let prompt = self
                            .player
                            .as_ref()
                            .map(|p| p.prompt())
                            .unwrap_or_else(|| "lobby>".to_string());
                        let output = format!("\r\n{}\r\n\x1b[33m{}\x1b[0m ", response, prompt);
                        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
                    }
                }
                // Backspace
                127 | 8 => {
                    if !self.line_buffer.is_empty() {
                        self.line_buffer.pop();
                        let _ = session.data(channel, CryptoVec::from(b"\x08 \x08".as_slice()));
                    }
                }
                // Printable characters
                32..=126 => {
                    self.line_buffer.push(byte as char);
                    let _ = session.data(channel, CryptoVec::from([byte].as_slice()));
                }
                // Ignore other control characters
                _ => {}
            }
        }
        Ok(())
    }
}
