//! 4-layer system prompt builder
//!
//! Constructs system prompts from multiple layers:
//! 1. Global - sshwarma environment description
//! 2. Model - per-model personality and capabilities
//! 3. Room - context, vibe, and assets
//! 4. User - preferences and custom instructions

use crate::model::ModelHandle;
use crate::world::Room;

/// Builder for constructing layered system prompts
pub struct SystemPromptBuilder;

impl SystemPromptBuilder {
    /// Build a complete system prompt from all layers
    pub fn build(
        model: &ModelHandle,
        room: Option<&Room>,
        username: &str,
        _user_prefs: Option<&str>, // TODO: Load from database
    ) -> String {
        let mut prompt = Self::global_layer();

        prompt.push_str("\n\n");
        prompt.push_str(&Self::model_layer(model));

        if let Some(room) = room {
            prompt.push_str("\n\n## Room Context\n");
            prompt.push_str(&Self::room_layer(room));
        }

        prompt.push_str("\n\n## Current User\n");
        prompt.push_str(&format!("You are talking with **{}**.\n", username));

        // TODO: Add user preferences when database integration is ready
        // if let Some(prefs) = user_prefs {
        //     prompt.push_str("\n## User Preferences\n");
        //     prompt.push_str(prefs);
        // }

        prompt
    }

    /// Global layer: sshwarma environment description
    pub fn global_layer() -> String {
        r#"You are an AI assistant in **sshwarma**, a collaborative SSH partyline where humans and AI models work together.

## Environment
- MUD-style text interface accessed via SSH
- Multiple users and models share rooms in real-time
- You have built-in functions for exploring rooms, navigating between them, and collaborating with users

## Communication Style
- Be conversational and collaborative
- Keep responses concise - this is a chat interface
- Use markdown sparingly (bold for emphasis, code blocks for code)

## Using Your Functions
- Your available functions are listed in "Your Functions" below
- Use them proactively when they help accomplish goals
- When asked what you can do, describe your capabilities based on those functions
- If a function fails, explain what went wrong and suggest alternatives"#.to_string()
    }

    /// Model layer: per-model personality and capabilities
    pub fn model_layer(model: &ModelHandle) -> String {
        let mut layer = format!("## Your Identity\n");
        layer.push_str(&format!("You are **{}**.\n", model.display_name));

        if let Some(system_prompt) = &model.system_prompt {
            layer.push_str("\n");
            layer.push_str(system_prompt);
        }

        layer
    }

    /// Room layer: context, vibe, and assets
    pub fn room_layer(room: &Room) -> String {
        let mut layer = format!("**Room:** {}\n", room.name);

        if let Some(description) = &room.description {
            layer.push_str(&format!("**Description:** {}\n", description));
        }

        if let Some(vibe) = &room.context.vibe {
            layer.push_str(&format!("**Vibe:** {}\n", vibe));
        }

        // List users in the room
        if !room.users.is_empty() {
            layer.push_str(&format!("**Present:** {}\n", room.users.join(", ")));
        }

        // List models in the room
        if !room.models.is_empty() {
            let model_names: Vec<&str> = room.models.iter().map(|m| m.display_name.as_str()).collect();
            layer.push_str(&format!("**Models:** {}\n", model_names.join(", ")));
        }

        layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelBackend;

    fn mock_model() -> ModelHandle {
        ModelHandle {
            short_name: "test".to_string(),
            display_name: "Test Model".to_string(),
            backend: ModelBackend::Mock { prefix: "Test".to_string() },
            available: true,
            system_prompt: Some("You are a helpful test assistant.".to_string()),
            context_window: Some(30000),
        }
    }

    #[test]
    fn test_global_layer() {
        let global = SystemPromptBuilder::global_layer();
        assert!(global.contains("sshwarma"));
        assert!(global.contains("Your Functions")); // Functions listed in prompt
    }

    #[test]
    fn test_model_layer() {
        let model = mock_model();
        let layer = SystemPromptBuilder::model_layer(&model);
        assert!(layer.contains("Test Model"));
        assert!(layer.contains("helpful test assistant"));
    }

    #[test]
    fn test_build_basic() {
        let model = mock_model();
        let prompt = SystemPromptBuilder::build(&model, None, "alice", None);

        // Should contain global layer
        assert!(prompt.contains("sshwarma"));
        // Should contain model identity
        assert!(prompt.contains("Test Model"));
        // Should contain username
        assert!(prompt.contains("alice"));
    }
}
