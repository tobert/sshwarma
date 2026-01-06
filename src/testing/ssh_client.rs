//! SSH test client for automated testing
//!
//! Connects to sshwarma, sends commands, captures output.
//! Supports SSH agent for encrypted keys.

use anyhow::{Context, Result};
use russh::client::{self, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use russh::{ChannelId, Disconnect};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

/// SSH test client for sending commands and capturing output
pub struct SshTestClient {
    handle: Handle<ClientHandler>,
    channel: russh::Channel<client::Msg>,
}

impl SshTestClient {
    /// Connect to SSH server using SSH agent (preferred) or key file
    ///
    /// Tries SSH agent first via SSH_AUTH_SOCK, falls back to key file.
    pub async fn connect(addr: &str, key_path: Option<&str>, username: &str) -> Result<Self> {
        // Create output channel
        let (output_tx, output_rx) = mpsc::unbounded_channel();

        // Create handler
        let handler = ClientHandler {
            output_tx,
            output_rx: Some(output_rx),
        };

        // Connect
        let config = Arc::new(client::Config::default());
        let mut handle = client::connect(config, addr, handler)
            .await
            .context("failed to connect")?;

        // Try SSH agent first
        if let Ok(mut agent) = AgentClient::connect_env().await {
            if let Ok(identities) = agent.request_identities().await {
                if let Some(pubkey) = identities.first() {
                    let auth_result = handle
                        .authenticate_publickey_with(username, pubkey.clone(), None, &mut agent)
                        .await
                        .context("agent authentication failed")?;

                    if auth_result.success() {
                        return Self::finish_connect(handle).await;
                    }
                }
            }
        }

        // Fall back to key file
        let key_path = key_path.map(|s| s.to_string()).unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".ssh/id_ed25519").to_string_lossy().to_string())
                .unwrap_or_else(|| "~/.ssh/id_ed25519".to_string())
        });

        let key = load_private_key(&key_path)?;

        // Authenticate with key (no hash alg needed for Ed25519)
        let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
        let auth_result = handle
            .authenticate_publickey(username, key_with_alg)
            .await
            .context("key authentication failed")?;

        if !auth_result.success() {
            anyhow::bail!("authentication rejected");
        }

        Self::finish_connect(handle).await
    }

    /// Complete connection after successful auth
    async fn finish_connect(handle: Handle<ClientHandler>) -> Result<Self> {
        // Open session channel
        let channel = handle
            .channel_open_session()
            .await
            .context("failed to open session channel")?;

        // Request PTY
        channel
            .request_pty(
                false,            // don't want reply
                "xterm-256color", // term type
                80,               // columns
                24,               // rows
                0,                // pixel width
                0,                // pixel height
                &[],              // terminal modes
            )
            .await
            .context("failed to request PTY")?;

        // Request shell
        channel
            .request_shell(false)
            .await
            .context("failed to request shell")?;

        Ok(Self { handle, channel })
    }

    /// Send input to the shell (adds newline if not present)
    pub async fn send(&mut self, input: &str) -> Result<()> {
        let data = if input.ends_with('\n') {
            input.as_bytes().to_vec()
        } else {
            format!("{}\n", input).into_bytes()
        };

        self.channel
            .data(&data[..])
            .await
            .context("failed to send data")?;

        Ok(())
    }

    /// Wait for output with timeout, then collect what we got
    ///
    /// Collects data from the channel until timeout expires.
    pub async fn wait_and_collect(&mut self, duration: std::time::Duration) -> Result<Vec<u8>> {
        self.wait_internal(duration, None).await
    }

    /// Wait until pattern appears in output, or timeout
    ///
    /// Returns all collected output once pattern is found.
    /// Pattern is matched against the visible text (ANSI codes stripped).
    pub async fn wait_for_pattern(
        &mut self,
        pattern: &str,
        timeout: std::time::Duration,
    ) -> Result<Vec<u8>> {
        self.wait_internal(timeout, Some(pattern)).await
    }

    /// Internal wait implementation
    async fn wait_internal(
        &mut self,
        duration: std::time::Duration,
        pattern: Option<&str>,
    ) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        let deadline = tokio::time::Instant::now() + duration;

        loop {
            // Check if pattern matched
            if let Some(pat) = pattern {
                let text = strip_ansi(&output);
                if text.contains(pat) {
                    // Give a tiny bit more time for any trailing output
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    // Drain any remaining data
                    while let Ok(msg) =
                        tokio::time::timeout(std::time::Duration::from_millis(10), self.channel.wait()).await
                    {
                        if let Some(russh::ChannelMsg::Data { data }) = msg {
                            output.extend_from_slice(&data);
                        } else {
                            break;
                        }
                    }
                    return Ok(output);
                }
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                if pattern.is_some() {
                    anyhow::bail!("timeout waiting for pattern");
                }
                break;
            }

            tokio::select! {
                msg = self.channel.wait() => {
                    match msg {
                        Some(russh::ChannelMsg::Data { data }) => {
                            output.extend_from_slice(&data);
                        }
                        Some(russh::ChannelMsg::Eof) => {
                            break;
                        }
                        Some(_) => {
                            // Other messages, continue
                        }
                        None => {
                            // Channel closed
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(remaining) => {
                    if pattern.is_some() {
                        anyhow::bail!("timeout waiting for pattern");
                    }
                    break;
                }
            }
        }

        Ok(output)
    }

    /// Close the connection
    pub async fn close(self) -> Result<()> {
        self.handle
            .disconnect(Disconnect::ByApplication, "goodbye", "en")
            .await
            .context("failed to disconnect")?;
        Ok(())
    }
}

/// Strip ANSI escape sequences from bytes, return as string
fn strip_ansi(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut result = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Skip until we hit a letter (the terminator)
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Load a private key from a file path
fn load_private_key(path: &str) -> Result<PrivateKey> {
    // Simple tilde expansion
    let expanded = if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&path[2..])
        } else {
            Path::new(path).to_path_buf()
        }
    } else {
        Path::new(path).to_path_buf()
    };

    let key_str = std::fs::read_to_string(&expanded)
        .with_context(|| format!("failed to read key file: {}", expanded.display()))?;

    russh::keys::decode_secret_key(&key_str, None).context("failed to decode private key")
}

/// Client handler that forwards data to a channel
struct ClientHandler {
    output_tx: mpsc::UnboundedSender<Vec<u8>>,
    #[allow(dead_code)]
    output_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
}

impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        async { Ok(true) } // Accept all keys for local testing
    }

    fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let _ = self.output_tx.send(data.to_vec());
        async { Ok(()) }
    }
}
