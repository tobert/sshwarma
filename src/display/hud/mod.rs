//! HUD (Heads-Up Display) for sshwarma terminal
//!
//! A composable, fixed-height display at the bottom of the terminal showing:
//! - Participants (users + models with status)
//! - MCP connections
//! - Room info (name, exits, duration)
//! - Ephemeral notifications
//!
//! The HUD is 8 lines including borders, with a bare cursor input line below.

mod renderer;
mod spinner;
mod state;

pub use renderer::{render_hud, HUD_CONTENT_LINES, HUD_HEIGHT};
pub use spinner::{spinner_char, SPINNER_FRAMES};
pub use state::{
    ExitDirection, HudState, McpConnectionState, Notification, ParticipantKind, ParticipantStatus,
    Presence,
};
