//! MCP connection events for status broadcasting.
//!
//! Uses `tokio::sync::broadcast` for multiple subscribers (HUD, logs, etc.).
//! Events are cheap to clone and carry enough context for display.

use tokio::sync::broadcast;

/// MCP connection event for subscribers.
///
/// Events are emitted by `McpManager` on connection state changes and
/// consumed by HUD notifications, logging, and other observers.
#[derive(Debug, Clone)]
pub enum McpEvent {
    /// Connection attempt started.
    Connecting {
        /// Connection name (e.g., "holler")
        name: String,
        /// Endpoint URL
        endpoint: String,
    },

    /// Successfully connected.
    Connected {
        /// Connection name
        name: String,
        /// Endpoint URL
        endpoint: String,
        /// Number of tools available
        tool_count: usize,
    },

    /// Connection failed, will retry.
    Reconnecting {
        /// Connection name
        name: String,
        /// Retry attempt number
        attempt: u32,
        /// Delay before next retry in milliseconds
        delay_ms: u64,
        /// Error message from failed attempt
        error: String,
    },

    /// Connection removed by user via `mcp_remove()`.
    ///
    /// Note: We retry forever, so this only happens on explicit removal.
    Removed {
        /// Connection name
        name: String,
    },

    /// Tools list refreshed from connected MCP.
    ToolsRefreshed {
        /// Connection name
        name: String,
        /// Updated tool count
        tool_count: usize,
    },
}

impl McpEvent {
    /// Get the connection name from any event variant.
    pub fn name(&self) -> &str {
        match self {
            McpEvent::Connecting { name, .. } => name,
            McpEvent::Connected { name, .. } => name,
            McpEvent::Reconnecting { name, .. } => name,
            McpEvent::Removed { name } => name,
            McpEvent::ToolsRefreshed { name, .. } => name,
        }
    }

    /// Returns true if this is an error event (Reconnecting).
    pub fn is_error(&self) -> bool {
        matches!(self, McpEvent::Reconnecting { .. })
    }
}

/// Broadcast sender for MCP events.
///
/// Clone this to share between components that emit events.
/// Sending to zero receivers silently succeeds.
#[derive(Clone)]
pub struct McpEventSender {
    tx: broadcast::Sender<McpEvent>,
}

impl McpEventSender {
    /// Create a new event sender with the specified buffer capacity.
    ///
    /// A capacity of 16 is reasonable for most use cases.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Send an event to all subscribers.
    ///
    /// Silently ignores errors (no receivers, or lagged receivers).
    pub fn send(&self, event: McpEvent) {
        let _ = self.tx.send(event);
    }

    /// Create a new subscriber to receive events.
    pub fn subscribe(&self) -> McpEventReceiver {
        McpEventReceiver {
            rx: self.tx.subscribe(),
        }
    }
}

impl Default for McpEventSender {
    fn default() -> Self {
        Self::new(16)
    }
}

/// Receiver for MCP events.
///
/// Each subscriber gets its own receiver. If a receiver falls behind
/// (lagged), it will miss events but won't block the sender.
pub struct McpEventReceiver {
    rx: broadcast::Receiver<McpEvent>,
}

impl McpEventReceiver {
    /// Receive the next event, waiting asynchronously.
    ///
    /// Returns `None` if the sender is dropped or the receiver is lagged
    /// (in which case, events were missed).
    pub async fn recv(&mut self) -> Option<McpEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Skip lagged events and try again
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Try to receive an event without waiting.
    ///
    /// Returns `None` if no event is available or the channel is closed.
    pub fn try_recv(&mut self) -> Option<McpEvent> {
        loop {
            match self.rx.try_recv() {
                Ok(event) => return Some(event),
                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    // Skip lagged and try again
                    continue;
                }
                Err(broadcast::error::TryRecvError::Empty) => return None,
                Err(broadcast::error::TryRecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_name() {
        let connecting = McpEvent::Connecting {
            name: "holler".to_string(),
            endpoint: "http://localhost:8080".to_string(),
        };
        assert_eq!(connecting.name(), "holler");

        let connected = McpEvent::Connected {
            name: "otlp".to_string(),
            endpoint: "http://localhost:4380".to_string(),
            tool_count: 5,
        };
        assert_eq!(connected.name(), "otlp");

        let reconnecting = McpEvent::Reconnecting {
            name: "test".to_string(),
            attempt: 3,
            delay_ms: 400,
            error: "connection refused".to_string(),
        };
        assert_eq!(reconnecting.name(), "test");

        let removed = McpEvent::Removed {
            name: "removed".to_string(),
        };
        assert_eq!(removed.name(), "removed");

        let refreshed = McpEvent::ToolsRefreshed {
            name: "refreshed".to_string(),
            tool_count: 10,
        };
        assert_eq!(refreshed.name(), "refreshed");
    }

    #[test]
    fn test_is_error() {
        assert!(!McpEvent::Connecting {
            name: "x".into(),
            endpoint: "y".into()
        }
        .is_error());

        assert!(!McpEvent::Connected {
            name: "x".into(),
            endpoint: "y".into(),
            tool_count: 0
        }
        .is_error());

        assert!(McpEvent::Reconnecting {
            name: "x".into(),
            attempt: 1,
            delay_ms: 100,
            error: "err".into()
        }
        .is_error());

        assert!(!McpEvent::Removed { name: "x".into() }.is_error());

        assert!(!McpEvent::ToolsRefreshed {
            name: "x".into(),
            tool_count: 5
        }
        .is_error());
    }

    #[test]
    fn test_event_clone() {
        let event = McpEvent::Connected {
            name: "test".to_string(),
            endpoint: "http://localhost".to_string(),
            tool_count: 42,
        };
        let cloned = event.clone();
        assert_eq!(cloned.name(), "test");
    }

    #[test]
    fn test_sender_no_receivers() {
        let sender = McpEventSender::new(16);
        // Should not panic when sending with no receivers
        sender.send(McpEvent::Connecting {
            name: "test".to_string(),
            endpoint: "http://localhost".to_string(),
        });
    }

    #[tokio::test]
    async fn test_single_subscriber() {
        let sender = McpEventSender::new(16);
        let mut receiver = sender.subscribe();

        sender.send(McpEvent::Connecting {
            name: "holler".to_string(),
            endpoint: "http://localhost:8080".to_string(),
        });

        let event = receiver.recv().await.unwrap();
        assert_eq!(event.name(), "holler");
        assert!(!event.is_error());
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let sender = McpEventSender::new(16);
        let mut rx1 = sender.subscribe();
        let mut rx2 = sender.subscribe();

        sender.send(McpEvent::Connected {
            name: "test".to_string(),
            endpoint: "http://localhost".to_string(),
            tool_count: 10,
        });

        // Both receivers should get the event
        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();

        assert_eq!(e1.name(), "test");
        assert_eq!(e2.name(), "test");
    }

    #[tokio::test]
    async fn test_try_recv_empty() {
        let sender = McpEventSender::new(16);
        let mut receiver = sender.subscribe();

        // No event sent yet
        assert!(receiver.try_recv().is_none());

        sender.send(McpEvent::Removed {
            name: "test".to_string(),
        });

        // Now there's an event
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.name(), "test");

        // Queue is empty again
        assert!(receiver.try_recv().is_none());
    }

    #[test]
    fn test_sender_default() {
        let sender = McpEventSender::default();
        sender.send(McpEvent::Removed {
            name: "test".to_string(),
        });
        // Just verify it doesn't panic
    }

    #[test]
    fn test_sender_clone() {
        let sender1 = McpEventSender::new(16);
        let sender2 = sender1.clone();
        let mut receiver = sender1.subscribe();

        // Events from cloned sender should reach receiver
        sender2.send(McpEvent::Removed {
            name: "from_clone".to_string(),
        });

        let event = receiver.try_recv().unwrap();
        assert_eq!(event.name(), "from_clone");
    }
}
