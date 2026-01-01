//! HUD (Heads-Up Display) state for sshwarma terminal
//!
//! Provides state types that are passed to Lua for HUD rendering.
//! The actual rendering is done entirely in Lua via on_tick().

mod state;

/// HUD dimensions (8 lines: 7 for display, 1 for input prompt)
pub const HUD_HEIGHT: u16 = 8;

pub use state::{
    ExitDirection, HudState, McpConnectionState, Notification, ParticipantKind, ParticipantStatus,
    Presence,
};
