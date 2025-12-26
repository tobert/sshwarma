//! Static command completion

use super::Completion;

/// Static list of commands with descriptions
const COMMANDS: &[(&str, &str)] = &[
    ("create", "Create a new room"),
    ("get", "Pick up an artifact"),
    ("help", "Show available commands"),
    ("history", "Show message history"),
    ("inv", "Show your inventory"),
    ("join", "Join a room"),
    ("leave", "Leave current room"),
    ("look", "Look around or examine something"),
    ("mcp", "MCP server management"),
    ("play", "Play an artifact"),
    ("quit", "Disconnect"),
    ("rooms", "List available rooms"),
    ("run", "Run an MCP tool"),
    ("set", "Change settings"),
    ("status", "Show session info"),
    ("stop", "Stop playback"),
    ("tools", "List available MCP tools"),
    ("who", "Who's in the room"),
];

pub struct CommandCompleter;

impl CommandCompleter {
    /// Get all command completions
    pub fn complete() -> Vec<Completion> {
        COMMANDS
            .iter()
            .map(|(name, desc)| Completion {
                text: format!("/{}", name),
                label: format!("/{:<12} {}", name, desc),
                score: 0,
            })
            .collect()
    }
}
