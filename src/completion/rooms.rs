//! Room name completion

use std::sync::Arc;

use crate::state::SharedState;

use super::Completion;

pub struct RoomCompleter;

impl RoomCompleter {
    /// Get room completions from current world state
    pub async fn complete(state: &Arc<SharedState>) -> Vec<Completion> {
        let world = state.world.read().await;

        world
            .rooms
            .iter()
            .map(|(name, room)| {
                let user_count = room.users.len();
                let suffix = match user_count {
                    0 => "(empty)".to_string(),
                    1 => "(1 user)".to_string(),
                    n => format!("({} users)", n),
                };

                Completion {
                    text: name.clone(),
                    label: format!("{:<16} {}", name, suffix),
                    score: 0,
                }
            })
            .collect()
    }
}
