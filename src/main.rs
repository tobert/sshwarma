//! sshwarma - SSH-accessible partyline for humans and models
//!
//! A MUD-style REPL where users connect via SSH and collaborate with
//! AI models in shared "partylines" (rooms). Plain text is chat,
//! /commands control navigation and tools, @mentions address models.

mod comm;
mod db;
mod interp;
mod llm;
mod mcp;
mod model;
mod player;
mod world;

use anyhow::{Context, Result};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId, CryptoVec};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::player::PlayerSession;
use crate::world::World;

/// Server configuration
#[derive(Clone)]
pub struct Config {
    /// SSH listen address
    pub listen_addr: SocketAddr,
    /// Path to server host key
    pub host_key_path: String,
    /// Path to authorized_keys file
    pub authorized_keys_path: String,
    /// llama.cpp endpoint
    pub llm_endpoint: String,
    /// MCP server endpoints (holler, exa, etc.)
    pub mcp_endpoints: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:2222".parse().unwrap(),
            host_key_path: "host_key".to_string(),
            authorized_keys_path: "~/.ssh/authorized_keys".to_string(),
            llm_endpoint: "http://localhost:2020".to_string(),
            mcp_endpoints: vec!["http://localhost:8080/mcp".to_string()],
        }
    }
}

/// The shared world state
pub struct SharedState {
    pub world: RwLock<World>,
    pub config: Config,
}

/// SSH server implementation
#[derive(Clone)]
struct SshServer {
    state: Arc<SharedState>,
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
struct SshHandler {
    state: Arc<SharedState>,
    player: Option<PlayerSession>,
    line_buffer: String,
}

impl server::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        _key: &russh::keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        // Accept public key auth for now
        Ok(server::Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        _key: &russh::keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        info!(user, "authenticated");
        self.player = Some(PlayerSession::new(user.to_string()));
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Send welcome message
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
            session.data(channel, CryptoVec::from(welcome.as_bytes()));
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Handle incoming data byte by byte for line editing
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
                        session.data(channel, CryptoVec::from(output.as_bytes()));
                    }
                }
                // Backspace
                127 | 8 => {
                    if !self.line_buffer.is_empty() {
                        self.line_buffer.pop();
                        // Echo backspace
                        session.data(channel, CryptoVec::from(b"\x08 \x08".as_slice()));
                    }
                }
                // Printable characters
                32..=126 => {
                    self.line_buffer.push(byte as char);
                    // Echo character
                    session.data(channel, CryptoVec::from([byte].as_slice()));
                }
                // Ignore other control characters
                _ => {}
            }
        }
        Ok(())
    }
}

impl SshHandler {
    async fn handle_input(&self, input: &str) -> String {
        let input = input.trim();

        if input.starts_with('/') {
            // Command
            let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
            let cmd = parts.first().unwrap_or(&"");
            let _args = parts.get(1).copied().unwrap_or("");

            match *cmd {
                "help" => self.cmd_help(),
                "rooms" => self.cmd_rooms().await,
                "who" => self.cmd_who().await,
                "quit" => "Goodbye!".to_string(),
                _ => format!("Unknown command: /{}", cmd),
            }
        } else if input.starts_with('@') {
            // Model mention
            let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
            let model = parts.first().unwrap_or(&"");
            let message = parts.get(1).copied().unwrap_or("");
            let username = self
                .player
                .as_ref()
                .map(|p| p.username.as_str())
                .unwrap_or("???");
            format!(
                "{} → {}: {}\r\n[model response pending]",
                username, model, message
            )
        } else {
            // Chat message
            let username = self
                .player
                .as_ref()
                .map(|p| p.username.as_str())
                .unwrap_or("???");
            format!("{}: {}", username, input)
        }
    }

    fn cmd_help(&self) -> String {
        r#"
Navigation:
  /rooms              List partylines
  /join <room>        Enter a partyline
  /leave              Return to lobby
  /create <name>      New partyline

Looking:
  /look               Room summary
  /look <thing>       Examine artifact/user/model
  /who                Who's online
  /history [n]        Recent messages

Communication:
  <text>              Say to room
  @model <msg>        Message a model

Tools:
  /tools              List available tools
  /run <tool> [args]  Invoke tool

/quit to disconnect
"#
        .to_string()
    }

    async fn cmd_rooms(&self) -> String {
        let world = self.state.world.read().await;
        let rooms = world.list_rooms();
        if rooms.is_empty() {
            "No partylines yet. /create <name> to start one.".to_string()
        } else {
            let mut out = "Partylines:\r\n".to_string();
            for room in rooms {
                out.push_str(&format!("  {} ... {} users\r\n", room.name, room.user_count));
            }
            out
        }
    }

    async fn cmd_who(&self) -> String {
        "Online: you (more coming soon)".to_string()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("sshwarma=info".parse()?),
        )
        .init();

    let config = Config::default();
    info!(addr = %config.listen_addr, "starting sshwarma");

    // Generate or load host key
    let key_path = std::path::Path::new(&config.host_key_path);
    let key = if key_path.exists() {
        info!("loading host key from {}", config.host_key_path);
        russh::keys::decode_secret_key(
            &std::fs::read_to_string(&config.host_key_path)?,
            None,
        )?
    } else {
        info!("generating new host key");
        let key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .context("failed to generate key")?;
        // Save key for next time
        std::fs::write(
            &config.host_key_path,
            key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?,
        )?;
        key
    };

    let russh_config = russh::server::Config {
        keys: vec![key],
        ..Default::default()
    };

    let state = Arc::new(SharedState {
        world: RwLock::new(World::new()),
        config: config.clone(),
    });

    let mut server = SshServer { state };

    info!("listening on {}", config.listen_addr);
    server
        .run_on_address(Arc::new(russh_config), config.listen_addr)
        .await?;

    Ok(())
}
