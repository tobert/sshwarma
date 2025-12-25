//! REPL command implementations

use crate::ssh::SshHandler;
use crate::world::{MessageContent, Sender};

impl SshHandler {
    pub async fn handle_input(&mut self, input: &str) -> String {
        let input = input.trim();

        if input.starts_with('/') {
            let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
            let cmd = parts.first().unwrap_or(&"");
            let args = parts.get(1).copied().unwrap_or("");

            match *cmd {
                "help" => self.cmd_help(),
                "rooms" => self.cmd_rooms().await,
                "who" => self.cmd_who().await,
                "join" => self.cmd_join(args).await,
                "create" => self.cmd_create(args).await,
                "leave" => self.cmd_leave().await,
                "look" => self.cmd_look(args).await,
                "history" => self.cmd_history(args).await,
                "tools" => self.cmd_tools().await,
                "run" => self.cmd_run(args).await,
                "mcp" => self.cmd_mcp(args).await,
                "quit" => "Goodbye!".to_string(),
                _ => format!("Unknown command: /{}", cmd),
            }
        } else if input.starts_with('@') {
            self.cmd_mention(input).await
        } else {
            self.cmd_say(input).await
        }
    }

    fn cmd_help(&self) -> String {
        r#"
Navigation:
  /rooms              List partylines
  /join <room>        Enter a partyline
  /leave              Return to lobby
  /create <name>      New partyline

Looking:
  /look               Room summary
  /look <thing>       Examine artifact/user/model
  /who                Who's online
  /history [n]        Recent messages

Communication:
  <text>              Say to room
  @model <msg>        Message a model

Tools:
  /tools              List available tools
  /run <tool> [args]  Invoke tool with JSON args

MCP:
  /mcp                List connected MCP servers
  /mcp connect <name> <url>  Connect to MCP server
  /mcp disconnect <name>     Disconnect from server
  /mcp refresh <name>        Refresh tool list

/quit to disconnect
"#
        .to_string()
    }

    async fn cmd_rooms(&self) -> String {
        let world = self.state.world.read().await;
        let rooms = world.list_rooms();
        if rooms.is_empty() {
            "No partylines yet. /create <name> to start one.".to_string()
        } else {
            let mut out = "Partylines:\r\n".to_string();
            for room in rooms {
                out.push_str(&format!("  {} ... {} users\r\n", room.name, room.user_count));
            }
            out
        }
    }

    async fn cmd_who(&self) -> String {
        if let Some(ref player) = self.player {
            if let Some(ref room_name) = player.current_room {
                let world = self.state.world.read().await;
                if let Some(room) = world.get_room(room_name) {
                    let users: Vec<&str> = room.users.iter().map(|s| s.as_str()).collect();
                    return format!("In {}: {}", room_name, users.join(", "));
                }
            }
        }
        "Online: you (more coming soon)".to_string()
    }

    async fn cmd_join(&mut self, args: &str) -> String {
        let room_name = args.trim();
        if room_name.is_empty() {
            return "Usage: /join <room>".to_string();
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        // Leave current room if in one
        if let Some(ref current) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(current) {
                room.remove_user(&username);
            }
        }

        // Check if room exists
        {
            let world = self.state.world.read().await;
            if world.get_room(room_name).is_none() {
                return format!(
                    "No partyline named '{}'. Use /create {} to make one.",
                    room_name, room_name
                );
            }
        }

        // Join the room
        {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(room_name) {
                room.add_user(username.clone());
            }
        }

        // Update player state and session
        if let Some(ref mut player) = self.player {
            player.join_room(room_name.to_string());
            let _ = self
                .state
                .db
                .update_session_room(&player.session_id, Some(room_name));
        }

        // Build output with room summary and history
        let mut output = self.cmd_look("").await;

        if let Ok(messages) = self.state.db.recent_messages(room_name, 20) {
            if !messages.is_empty() {
                output.push_str("\r\n\r\n─── Recent History ───\r\n");
                for msg in messages {
                    let line = format!(
                        "[{}] {}: {}\r\n",
                        &msg.timestamp[11..16],
                        msg.sender_name,
                        msg.content
                    );
                    output.push_str(&line);
                }
                output.push_str("──────────────────────\r\n");
            }
        }

        output
    }

    async fn cmd_create(&mut self, args: &str) -> String {
        let room_name = args.trim();
        if room_name.is_empty() {
            return "Usage: /create <name>".to_string();
        }

        if !room_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return "Room name can only contain letters, numbers, dashes, and underscores."
                .to_string();
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        // Check if room already exists
        {
            let world = self.state.world.read().await;
            if world.get_room(room_name).is_some() {
                return format!(
                    "Partyline '{}' already exists. Use /join {} to enter.",
                    room_name, room_name
                );
            }
        }

        // Leave current room if in one
        if let Some(ref current) = self.player.as_ref().and_then(|p| p.current_room.clone()) {
            let mut world = self.state.world.write().await;
            if let Some(room) = world.get_room_mut(current) {
                room.remove_user(&username);
            }
        }

        // Create and join the room
        {
            let mut world = self.state.world.write().await;
            world.create_room(room_name.to_string());
            if let Some(room) = world.get_room_mut(room_name) {
                room.add_user(username.clone());
            }
        }

        let _ = self.state.db.create_room(room_name, None);

        if let Some(ref mut player) = self.player {
            player.join_room(room_name.to_string());
        }

        format!(
            "Created partyline '{}'.\r\n\r\n{}",
            room_name,
            self.cmd_look("").await
        )
    }

    async fn cmd_leave(&mut self) -> String {
        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        let current_room = match &self.player {
            Some(p) => p.current_room.clone(),
            None => None,
        };

        match current_room {
            Some(room_name) => {
                {
                    let mut world = self.state.world.write().await;
                    if let Some(room) = world.get_room_mut(&room_name) {
                        room.remove_user(&username);
                    }
                }

                if let Some(ref mut player) = self.player {
                    player.leave_room();
                    let _ = self.state.db.update_session_room(&player.session_id, None);
                }

                format!("Left '{}'.\r\nYou are now in the lobby.", room_name)
            }
            None => "You're already in the lobby.".to_string(),
        }
    }

    async fn cmd_look(&self, args: &str) -> String {
        let args = args.trim();

        if !args.is_empty() {
            return format!("You look at '{}'. (detailed inspection coming soon)", args);
        }

        match &self.player {
            Some(player) => match &player.current_room {
                Some(room_name) => {
                    let world = self.state.world.read().await;
                    if let Some(room) = world.get_room(room_name) {
                        let mut out = format!("═══ {} ═══\r\n", room_name);
                        if let Some(ref desc) = room.description {
                            out.push_str(&format!("{}\r\n", desc));
                        }
                        out.push_str("\r\n");

                        if room.users.is_empty() {
                            out.push_str("Nobody else is here.\r\n");
                        } else {
                            out.push_str(&format!("Users: {}\r\n", room.users.join(", ")));
                        }

                        if !room.models.is_empty() {
                            let model_names: Vec<_> =
                                room.models.iter().map(|m| m.short_name.as_str()).collect();
                            out.push_str(&format!("Models: {}\r\n", model_names.join(", ")));
                        }

                        if !room.artifacts.is_empty() {
                            out.push_str(&format!("Artifacts: {} items\r\n", room.artifacts.len()));
                        }

                        out
                    } else {
                        "Room not found.".to_string()
                    }
                }
                None => "═══ Lobby ═══\r\n\r\nYou're in the lobby. Use /rooms to see partylines, /join <room> to enter one.".to_string(),
            },
            None => "Not authenticated".to_string(),
        }
    }

    async fn cmd_history(&self, args: &str) -> String {
        let limit: usize = args.trim().parse().unwrap_or(50);
        let limit = limit.min(200);

        let room_name = match &self.player {
            Some(p) => match &p.current_room {
                Some(r) => r.clone(),
                None => return "You need to be in a room to see history.".to_string(),
            },
            None => return "Not authenticated".to_string(),
        };

        match self.state.db.recent_messages(&room_name, limit) {
            Ok(messages) => {
                if messages.is_empty() {
                    "No messages in this room yet.".to_string()
                } else {
                    let mut output =
                        format!("─── Last {} messages in {} ───\r\n", messages.len(), room_name);
                    for msg in messages {
                        let line = format!(
                            "[{}] {}: {}\r\n",
                            &msg.timestamp[11..16],
                            msg.sender_name,
                            msg.content
                        );
                        output.push_str(&line);
                    }
                    output.push_str("──────────────────────────────\r\n");
                    output
                }
            }
            Err(e) => format!("Error loading history: {}", e),
        }
    }

    async fn cmd_say(&mut self, message: &str) -> String {
        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        let current_room = self.player.as_ref().and_then(|p| p.current_room.clone());

        match current_room {
            Some(room_name) => {
                {
                    let mut world = self.state.world.write().await;
                    if let Some(room) = world.get_room_mut(&room_name) {
                        room.add_message(
                            Sender::User(username.clone()),
                            MessageContent::Chat(message.to_string()),
                        );
                    }
                }

                let _ = self
                    .state
                    .db
                    .add_message(&room_name, "user", &username, "chat", message);

                format!("{}: {}", username, message)
            }
            None => {
                format!("{}: {} (lobby chat not saved)", username, message)
            }
        }
    }

    async fn cmd_mention(&mut self, input: &str) -> String {
        let input = input.trim_start_matches('@');
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let model_name = parts.first().unwrap_or(&"");
        let message = parts.get(1).copied().unwrap_or("").trim();

        if message.is_empty() {
            return format!("Usage: @{} <message>", model_name);
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

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
                return format!(
                    "Unknown model '{}'. Available: {}",
                    model_name,
                    available.join(", ")
                );
            }
        };

        let mut output = format!("{} → @{}: {}\r\n", username, model_name, message);

        // Build context from room history
        let history = if let Some(ref room_name) =
            self.player.as_ref().and_then(|p| p.current_room.clone())
        {
            let world = self.state.world.read().await;
            if let Some(room) = world.get_room(room_name) {
                room.recent_history(10)
                    .iter()
                    .filter_map(|msg| match &msg.content {
                        MessageContent::Chat(text) => {
                            let role = match &msg.sender {
                                Sender::User(_) => "user",
                                Sender::Model(_) => "assistant",
                                Sender::System => return None,
                            };
                            Some((role.to_string(), text.clone()))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let system_prompt = format!(
            "You are {} in the sshwarma collaborative chat. \
             You're conversing with {} and possibly other users. \
             Be helpful, concise, and friendly. Keep responses under 500 words unless asked for more detail.",
            model.display_name, username
        );

        output.push_str(&format!("\r\n{}: ", model.short_name));

        match self
            .state
            .llm
            .chat_with_context(&model, &system_prompt, &history, message)
            .await
        {
            Ok(response) => {
                if let Some(ref room_name) =
                    self.player.as_ref().and_then(|p| p.current_room.clone())
                {
                    {
                        let mut world = self.state.world.write().await;
                        if let Some(room) = world.get_room_mut(room_name) {
                            room.add_message(
                                Sender::Model(model.short_name.clone()),
                                MessageContent::Chat(response.clone()),
                            );
                        }
                    }
                    let _ = self.state.db.add_message(
                        room_name,
                        "model",
                        &model.short_name,
                        "chat",
                        &response,
                    );
                }

                let formatted = response.replace('\n', "\r\n");
                output.push_str(&formatted);
            }
            Err(e) => {
                output.push_str(&format!("[error: {}]", e));
            }
        }

        output
    }

    async fn cmd_tools(&self) -> String {
        let tools = self.state.mcp.list_tools().await;
        if tools.is_empty() {
            return "No tools available. Use /mcp connect <name> <url> to add an MCP server."
                .to_string();
        }

        let mut output = "Available tools:\r\n".to_string();
        for tool in tools {
            output.push_str(&format!(
                "  {} ({})\r\n    {}\r\n",
                tool.name, tool.source, tool.description
            ));
        }
        output
    }

    async fn cmd_run(&self, args: &str) -> String {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        let tool_name = match parts.first() {
            Some(name) if !name.is_empty() => *name,
            _ => {
                return "Usage: /run <tool> [json args]\r\nExample: /run orpheus_generate {\"temperature\": 1.0}".to_string()
            }
        };

        let args_json: serde_json::Value = if let Some(json_str) = parts.get(1) {
            match serde_json::from_str(json_str) {
                Ok(v) => v,
                Err(e) => return format!("Invalid JSON: {}", e),
            }
        } else {
            serde_json::json!({})
        };

        match self.state.mcp.call_tool(tool_name, args_json).await {
            Ok(result) => {
                if result.is_error {
                    format!("Tool error: {}", result.content)
                } else {
                    result.content.replace('\n', "\r\n")
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_mcp(&self, args: &str) -> String {
        let parts: Vec<&str> = args.splitn(3, ' ').collect();
        let subcmd = parts.first().copied().unwrap_or("");

        match subcmd {
            "list" | "" => {
                let connections = self.state.mcp.list_connections().await;
                if connections.is_empty() {
                    "No MCP servers connected.\r\n\r\nUsage:\r\n  /mcp connect <name> <url>\r\n  /mcp disconnect <name>".to_string()
                } else {
                    let mut output = "Connected MCP servers:\r\n".to_string();
                    for conn in connections {
                        output.push_str(&format!(
                            "  {} ... {} tools @ {}\r\n",
                            conn.name, conn.tool_count, conn.endpoint
                        ));
                    }
                    output
                }
            }
            "connect" => {
                let name = parts.get(1).copied().unwrap_or("");
                let url = parts.get(2).copied().unwrap_or("");
                if name.is_empty() || url.is_empty() {
                    return "Usage: /mcp connect <name> <url>".to_string();
                }

                match self.state.mcp.connect(name, url).await {
                    Ok(()) => format!("Connected to MCP server '{}' at {}", name, url),
                    Err(e) => format!("Failed to connect: {}", e),
                }
            }
            "disconnect" => {
                let name = parts.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return "Usage: /mcp disconnect <name>".to_string();
                }

                match self.state.mcp.disconnect(name).await {
                    Ok(true) => format!("Disconnected from '{}'", name),
                    Ok(false) => format!("Not connected to '{}'", name),
                    Err(e) => format!("Error: {}", e),
                }
            }
            "refresh" => {
                let name = parts.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return "Usage: /mcp refresh <name>".to_string();
                }

                match self.state.mcp.refresh_tools(name).await {
                    Ok(()) => format!("Refreshed tools from '{}'", name),
                    Err(e) => format!("Error: {}", e),
                }
            }
            _ => format!(
                "Unknown MCP command: {}. Try: list, connect, disconnect, refresh",
                subcmd
            ),
        }
    }
}
