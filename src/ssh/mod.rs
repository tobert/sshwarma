//! SSH server module
//!
//! Modular SSH server implementation using the Row/Buffer system.

mod handler;
mod input;
mod screen;
mod session;
mod streaming;

use std::net::SocketAddr;
use std::sync::Arc;

use russh::server;
use tracing::info;

use crate::state::SharedState;

pub use handler::SshHandler;
pub use session::SessionState;
pub use streaming::RowUpdate;

/// SSH server implementation
#[derive(Clone)]
pub struct SshServer {
    pub state: Arc<SharedState>,
}

impl SshServer {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self { state }
    }
}

impl server::Server for SshServer {
    type Handler = SshHandler;

    fn new_client(&mut self, peer_addr: Option<SocketAddr>) -> Self::Handler {
        info!(?peer_addr, "new connection");
        SshHandler::new(self.state.clone())
    }

    fn handle_session_error(&mut self, error: <Self::Handler as server::Handler>::Error) {
        tracing::error!("session error: {:?}", error);
    }
}
