//! Input handling and command dispatch

use crate::db::rows::Row;
use crate::display::hud::ParticipantStatus;
use crate::display::styles::ctrl;
use crate::interp::{self, Input};
use anyhow::Result;
use russh::server::Session;
use russh::{ChannelId, CryptoVec};

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
                self.dispatch_command(channel, session, &name, &args).await?;
            }

            Input::Mention { model, message } => {
                self.handle_mention(channel, session, &model, &message).await?;
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
        channel: ChannelId,
        session: &mut Session,
        message: &str,
    ) -> Result<()> {
        let Some(ref player) = self.player else {
            self.send_error(channel, session, "Not authenticated").await;
            return Ok(());
        };

        let Some(ref room_name) = player.current_room else {
            self.send_error(channel, session, "Not in a room. Use /join <room>").await;
            return Ok(());
        };

        // Get buffer
        let buffer = self.state.db.get_or_create_room_buffer(room_name)?;

        // Get agent
        let agent = self.state.db.get_or_create_human_agent(&player.username)?;

        // Add message row
        let mut row = Row::message(&buffer.id, &agent.id, message, false);
        self.state.db.append_row(&mut row)?;

        // Render and send
        self.render_incremental(channel, session).await;

        Ok(())
    }

    /// Handle @mention (spawn model response)
    async fn handle_mention(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        model_name: &str,
        message: &str,
    ) -> Result<()> {
        let Some(ref player) = self.player else {
            self.send_error(channel, session, "Not authenticated").await;
            return Ok(());
        };

        if message.is_empty() {
            self.send_error(channel, session, &format!("Usage: @{} <message>", model_name)).await;
            return Ok(());
        }

        // Look up model
        let model = match self.state.models.get(model_name) {
            Some(m) => m.clone(),
            None => {
                let available: Vec<_> = self.state.models.available()
                    .iter()
                    .map(|m| m.short_name.as_str())
                    .collect();
                self.send_error(channel, session,
                    &format!("Unknown model '{}'. Available: {}", model_name, available.join(", "))).await;
                return Ok(());
            }
        };

        let room_name = player.current_room.clone();
        let username = player.username.clone();

        // Add user's message to buffer
        if let Some(ref room) = room_name {
            let buffer = self.state.db.get_or_create_room_buffer(room)?;
            let agent = self.state.db.get_or_create_human_agent(&username)?;
            let mut row = Row::message(&buffer.id, &agent.id,
                format!("@{}: {}", model_name, message), false);
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

        // Render current state
        self.render_full(channel, session).await;

        // Update HUD status
        {
            let mut hud = self.hud_state.lock().await;
            hud.update_status(&model.short_name, ParticipantStatus::Thinking);
        }

        // Spawn background task for model response
        self.spawn_model_response(
            model,
            message.to_string(),
            username,
            room_name,
            placeholder_row_id,
        ).await?;

        Ok(())
    }

    /// Dispatch a slash command
    async fn dispatch_command(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
        name: &str,
        args: &str,
    ) -> Result<()> {
        // Build the full command line and call handle_input from commands.rs
        let line = if args.is_empty() {
            format!("/{}", name)
        } else {
            format!("/{} {}", name, args)
        };
        let result = self.handle_input(&line).await;

        // Send output
        if !result.text.is_empty() {
            let formatted = result.text.replace('\n', "\r\n");
            let _ = session.data(channel, CryptoVec::from(formatted.as_bytes()));
        }

        Ok(())
    }

    /// Send error message
    async fn send_error(&self, channel: ChannelId, session: &mut Session, msg: &str) {
        let output = format!("\x1b[31m{}\x1b[0m\r\n", msg);
        let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
    }

    /// Render buffer incrementally
    pub async fn render_incremental(&mut self, channel: ChannelId, session: &mut Session) {
        let mut sess = self.session_state.lock().await;
        if let Ok(output) = sess.render_incremental(&self.state.db) {
            if !output.is_empty() {
                let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
            }
        }
    }

    /// Render full buffer
    pub async fn render_full(&mut self, channel: ChannelId, session: &mut Session) {
        let (_, height) = self.term_size;
        let mut sess = self.session_state.lock().await;

        if let Ok(rendered) = sess.render_full(&self.state.db) {
            // Clear and redraw
            let mut output = String::new();
            output.push_str(&ctrl::move_to(1, 1));
            for _ in 0..height.saturating_sub(10) {
                output.push_str(&ctrl::clear_line());
                output.push_str(ctrl::CRLF);
            }
            output.push_str(&ctrl::move_to(1, 1));
            output.push_str(&rendered);
            let _ = session.data(channel, CryptoVec::from(output.as_bytes()));
        }
    }
}
