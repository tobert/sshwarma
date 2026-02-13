//! Input handling and command dispatch

use crate::db::rows::Row;
use crate::interp::{self, Input};
use crate::status::Status;
use anyhow::Result;
use russh::server::Session;
use russh::ChannelId;

use super::handler::SshHandler;

impl SshHandler {
    /// Process a complete line of input
    pub async fn process_input(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        line: &str,
    ) -> Result<()> {
        let input = interp::parse(line);

        match input {
            Input::Empty => {}

            Input::Command { name, args } => {
                self.dispatch_command(channel, session, &name, &args)
                    .await?;
            }

            Input::Mention { model, message } => {
                self.handle_mention(channel, session, &model, &message)
                    .await?;
            }

            Input::Chat(message) => {
                self.handle_chat(channel, session, &message).await?;
            }
        }

        Ok(())
    }

    /// Handle chat message (add to room buffer)
    async fn handle_chat(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
        message: &str,
    ) -> Result<()> {
        let Some(ref player) = self.player else {
            self.push_error("Not authenticated").await;
            return Ok(());
        };

        let Some(room_name) = self.current_room().await else {
            self.push_error("Not in a room. Use /join <room>").await;
            return Ok(());
        };

        // Get buffer
        let buffer = self.state.db.get_or_create_room_buffer(&room_name)?;

        // Get agent
        let agent = self.state.db.get_or_create_human_agent(&player.username)?;

        // Add message row
        let mut row = Row::message(&buffer.id, &agent.id, message, false);
        self.state.db.append_row(&mut row)?;

        Ok(())
    }

    /// Handle @mention (spawn model response)
    async fn handle_mention(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
        model_name: &str,
        message: &str,
    ) -> Result<()> {
        let Some(ref player) = self.player else {
            self.push_error("Not authenticated").await;
            return Ok(());
        };

        if message.is_empty() {
            self.push_error(format!("Usage: @{} <message>", model_name))
                .await;
            return Ok(());
        }

        // Look up model
        let model = match self.state.models.get(model_name) {
            Some(m) => m.clone(),
            None => {
                let available: Vec<_> = self
                    .state
                    .models
                    .available()
                    .iter()
                    .map(|m| m.short_name.as_str())
                    .collect();
                self.push_error(format!(
                    "Unknown model '{}'. Available: {}",
                    model_name,
                    available.join(", ")
                ))
                .await;
                return Ok(());
            }
        };

        let room_name = self.current_room().await;
        let room_id = if let Some(ref name) = room_name {
            self.state
                .db
                .get_room_by_name(name)
                .ok()
                .flatten()
                .map(|r| r.id)
        } else {
            None
        };
        let username = player.username.clone();

        // Add user's message to buffer
        if let Some(ref room) = room_name {
            let buffer = self.state.db.get_or_create_room_buffer(room)?;
            let agent = self.state.db.get_or_create_human_agent(&username)?;
            let mut row = Row::message(
                &buffer.id,
                &agent.id,
                format!("@{}: {}", model_name, message),
                false,
            );
            self.state.db.append_row(&mut row)?;
        }

        // Create placeholder row for model response
        let placeholder_row_id = if let Some(ref room) = room_name {
            let buffer = self.state.db.get_or_create_room_buffer(room)?;
            let model_agent = self.state.db.get_or_create_model_agent(&model.short_name)?;
            let mut row = Row::thinking(&buffer.id, &model_agent.id);
            self.state.db.append_row(&mut row)?;
            Some(row.id)
        } else {
            None
        };

        // Update session context with model and status
        let agent = self
            .state
            .db
            .get_agent_by_name(&username)?
            .ok_or_else(|| anyhow::anyhow!("agent not found: {}", username))?;
        self.with_lua(|lua| {
            lua.tool_state()
                .set_session_context(Some(crate::lua::SessionContext {
                    agent_id: agent.id.clone(),
                    model: Some(model.clone()),
                    room_id: room_id.clone(),
                }));
            lua.tool_state()
                .set_status(&model.short_name, Status::Thinking);
        })
        .await;

        // Spawn background task for model response
        self.spawn_model_response(
            model,
            message.to_string(),
            username,
            room_name,
            placeholder_row_id,
        )
        .await?;

        Ok(())
    }

    /// Dispatch a slash command via Lua command system
    async fn dispatch_command(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
        name: &str,
        args: &str,
    ) -> Result<()> {
        tracing::info!("dispatch_command: name={} args={}", name, args);

        let Some(ref lua_runtime) = self.lua_runtime else {
            tracing::error!("No Lua runtime available for command dispatch");
            return Ok(());
        };

        // Get session info for context
        let username = self.player.as_ref().map(|p| p.username.clone());
        let room_name = self.current_room().await;
        let room_id = if let Some(ref name) = room_name {
            self.state
                .db
                .get_room_by_name(name)
                .ok()
                .flatten()
                .map(|r| r.id)
        } else {
            None
        };

        let lua = lua_runtime.lock().await;

        // Set session context so commands can access tools.session()
        if let Some(ref user) = username {
            let agent = self
                .state
                .db
                .get_agent_by_name(user)?
                .ok_or_else(|| anyhow::anyhow!("agent not found: {}", user))?;
            lua.tool_state()
                .set_session_context(Some(crate::lua::SessionContext {
                    agent_id: agent.id,
                    model: None,
                    room_id: room_id.clone(),
                }));
        }

        tracing::info!("dispatch_command: calling lua.call_dispatch_command");
        match lua.call_dispatch_command(name, args) {
            Ok(Some(cmd_result)) => {
                tracing::debug!(
                    mode = %cmd_result.mode,
                    text_len = cmd_result.text.len(),
                    "dispatch_command: result"
                );
                // Commands that display content use page.show() directly in Lua.
                // We only handle notification mode here.
                if !cmd_result.text.is_empty() && cmd_result.mode == "notification" {
                    lua.tool_state()
                        .push_notification(cmd_result.text.clone(), 10000);
                }
            }
            Ok(None) => {
                lua.tool_state()
                    .push_notification(format!("Unknown command: /{}", name), 5000);
            }
            Err(e) => {
                lua.tool_state().push_notification_with_level(
                    format!("Command error: {}", e),
                    5000,
                    crate::lua::NotificationLevel::Error,
                );
            }
        }

        Ok(())
    }
}
