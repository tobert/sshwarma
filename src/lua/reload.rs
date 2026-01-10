//! Lua hot reload events
//!
//! Broadcasts module changes from filesystem watcher to all sessions.
//! Each session can subscribe and decide how to handle the invalidation.

use std::path::PathBuf;
use tokio::sync::broadcast;

/// Events broadcast when Lua files change on disk
#[derive(Debug, Clone)]
pub enum LuaReloadEvent {
    /// A module file was created
    ModuleCreated {
        module_name: String,
        path: PathBuf,
    },
    /// A module file was modified
    ModuleChanged {
        module_name: String,
        path: PathBuf,
    },
    /// A module file was deleted
    ModuleDeleted {
        module_name: String,
        path: PathBuf,
    },
}

impl LuaReloadEvent {
    /// Get the module name for this event
    pub fn module_name(&self) -> &str {
        match self {
            LuaReloadEvent::ModuleCreated { module_name, .. } => module_name,
            LuaReloadEvent::ModuleChanged { module_name, .. } => module_name,
            LuaReloadEvent::ModuleDeleted { module_name, .. } => module_name,
        }
    }

    /// Get the file path for this event
    pub fn path(&self) -> &PathBuf {
        match self {
            LuaReloadEvent::ModuleCreated { path, .. } => path,
            LuaReloadEvent::ModuleChanged { path, .. } => path,
            LuaReloadEvent::ModuleDeleted { path, .. } => path,
        }
    }
}

/// Sender for Lua reload events (held by the watcher)
#[derive(Clone)]
pub struct LuaReloadSender {
    tx: broadcast::Sender<LuaReloadEvent>,
}

impl Default for LuaReloadSender {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaReloadSender {
    /// Create a new sender with default capacity
    pub fn new() -> Self {
        // 64 events should be plenty - modules don't change that fast
        let (tx, _) = broadcast::channel(64);
        Self { tx }
    }

    /// Send a reload event to all subscribers
    pub fn send(&self, event: LuaReloadEvent) {
        // Ignore error - means no receivers (no active sessions)
        let _ = self.tx.send(event);
    }

    /// Subscribe to receive reload events
    pub fn subscribe(&self) -> LuaReloadReceiver {
        LuaReloadReceiver {
            rx: self.tx.subscribe(),
        }
    }
}

/// Receiver for Lua reload events (one per session)
pub struct LuaReloadReceiver {
    rx: broadcast::Receiver<LuaReloadEvent>,
}

impl LuaReloadReceiver {
    /// Receive the next reload event
    ///
    /// Returns None if the sender is dropped (server shutdown)
    pub async fn recv(&mut self) -> Option<LuaReloadEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    // Missed some events - log and continue
                    tracing::warn!("Lua reload receiver lagged by {} events", n);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
