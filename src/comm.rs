//! Communication: say, tell, broadcast

use crate::display::{EntryContent, EntrySource, LedgerEntry};
use crate::world::Room;

/// Send a chat message to a room
pub fn say(room: &mut Room, username: &str, message: &str) -> String {
    room.add_entry(
        EntrySource::User(username.to_string()),
        EntryContent::Chat(message.to_string()),
    );
    format!("{}: {}", username, message)
}

/// Format a ledger entry for display (simple text form)
pub fn format_entry(entry: &LedgerEntry) -> Option<String> {
    let sender = match &entry.source {
        EntrySource::User(name) => name.clone(),
        EntrySource::Model { name, .. } => format!("@{}", name),
        EntrySource::System => "[system]".to_string(),
        EntrySource::Command { command } => format!("/{}", command),
    };

    match &entry.content {
        EntryContent::Chat(text) => Some(format!("{}: {}", sender, text)),
        EntryContent::CommandOutput(text) => Some(text.clone()),
        EntryContent::Presence { user, action } => {
            use crate::display::PresenceAction;
            match action {
                PresenceAction::Join => Some(format!("{} joined", user)),
                PresenceAction::Leave => Some(format!("{} left", user)),
            }
        }
        EntryContent::Error(msg) => Some(format!("[error] {}", msg)),
        EntryContent::Compaction(summary) => Some(format!("--- {} ---", summary)),
        // Skip transient/display-only entries
        EntryContent::Status(_)
        | EntryContent::RoomHeader { .. }
        | EntryContent::Welcome { .. }
        | EntryContent::HistorySeparator { .. } => None,
    }
}
