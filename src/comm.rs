//! Communication: say, tell, broadcast

use crate::world::{Message, MessageContent, Partyline, Sender};

/// Send a chat message to a room
pub fn say(room: &mut Partyline, username: &str, message: &str) -> String {
    room.add_message(
        Sender::User(username.to_string()),
        MessageContent::Chat(message.to_string()),
    );
    format!("{}: {}", username, message)
}

/// Send a private message (tell) to a user or model
pub fn tell(room: &mut Partyline, from: &str, to: &str, message: &str) -> String {
    room.add_message(
        Sender::User(from.to_string()),
        MessageContent::Tell {
            to: to.to_string(),
            message: message.to_string(),
        },
    );
    format!("{} → {}: {}", from, to, message)
}

/// Format message for display
pub fn format_message(msg: &Message) -> String {
    let sender = match &msg.sender {
        Sender::User(name) => name.clone(),
        Sender::Model(name) => format!("@{}", name),
        Sender::System => "[system]".to_string(),
    };

    match &msg.content {
        MessageContent::Chat(text) => format!("{}: {}", sender, text),
        MessageContent::Tell { to, message } => format!("{} → {}: {}", sender, to, message),
        MessageContent::ToolRun { tool, result } => {
            format!("[{} ran {}]\n{}", sender, tool, result)
        }
        MessageContent::ArtifactCreated { artifact } => {
            format!(
                "New artifact: [{}] {} (by {})",
                format!("{:?}", artifact.artifact_type),
                artifact.name,
                artifact.created_by
            )
        }
        MessageContent::Join(who) => format!("{} joined", who),
        MessageContent::Leave(who) => format!("{} left", who),
    }
}
