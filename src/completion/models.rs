//! Model name completion for @mentions

use std::sync::Arc;

use crate::state::SharedState;

use super::Completion;

pub struct ModelCompleter;

impl ModelCompleter {
    /// Get model completions from registry
    pub async fn complete(state: &Arc<SharedState>) -> Vec<Completion> {
        state
            .models
            .list()
            .into_iter()
            .map(|info| {
                Completion {
                    text: format!("@{}", info.short_name),
                    label: format!("@{:<12} {}", info.short_name, info.display_name),
                    score: 0,
                }
            })
            .collect()
    }
}
