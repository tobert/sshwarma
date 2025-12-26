//! REPL command implementations

use crate::ssh::SshHandler;
use crate::world::{JournalKind, MessageContent, Sender};

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
                "examine" => self.cmd_examine(args).await,
                "history" => self.cmd_history(args).await,
                "tools" => self.cmd_tools().await,
                "run" => self.cmd_run(args).await,
                "mcp" => self.cmd_mcp(args).await,
                // Room context commands
                "vibe" => self.cmd_vibe(args).await,
                "note" => self.cmd_journal(args, "note").await,
                "decide" => self.cmd_journal(args, "decision").await,
                "idea" => self.cmd_journal(args, "idea").await,
                "milestone" => self.cmd_journal(args, "milestone").await,
                "journal" => self.cmd_journal_list(args).await,
                "bring" => self.cmd_bring(args).await,
                "drop" => self.cmd_drop(args).await,
                "inspire" => self.cmd_inspire(args).await,
                "dig" => self.cmd_dig(args).await,
                "go" => self.cmd_go(args).await,
                "exits" => self.cmd_exits().await,
                "fork" => self.cmd_fork(args).await,
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
  /rooms              List rooms
  /join <room>        Enter a room
  /leave              Return to lobby
  /create <name>      New room
  /go <direction>     Navigate via exit
  /exits              List exits from room
  /fork <name>        Fork room (inherit context)

Looking:
  /look               Room summary
  /examine <role>     Inspect bound asset
  /who                Who's online
  /history [n]        Recent messages

Room Context:
  /vibe [text]        Set/view room vibe
  /note <text>        Add journal note
  /decide <text>      Record decision
  /idea <text>        Capture idea
  /milestone <text>   Mark milestone
  /journal [kind]     View journal entries
  /bring <id> as <role>  Bind artifact
  /drop <role>        Unbind asset
  /inspire <text>     Add to mood board

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
            "No rooms yet. /create <name> to start one.".to_string()
        } else {
            let mut out = "Rooms:\r\n".to_string();
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
                    "No room named '{}'. Use /create {} to make one.",
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
                    "Room '{}' already exists. Use /join {} to enter.",
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
            "Created room '{}'.\r\n\r\n{}",
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
                None => "═══ Lobby ═══\r\n\r\nYou're in the lobby. Use /rooms to see rooms, /join <room> to enter one.".to_string(),
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

        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.state.mcp.call_tool(tool_name, args_json),
        )
        .await
        {
            Ok(Ok(result)) => {
                if result.is_error {
                    format!("Tool error: {}", result.content)
                } else {
                    result.content.replace('\n', "\r\n")
                }
            }
            Ok(Err(e)) => format!("Error: {}", e),
            Err(_) => format!("Tool '{}' timed out after 30s", tool_name),
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

    // Helper methods
    fn current_room(&self) -> Option<String> {
        self.player.as_ref().and_then(|p| p.current_room.clone())
    }

    fn username(&self) -> Option<String> {
        self.player.as_ref().map(|p| p.username.clone())
    }

    // Room context commands
    async fn cmd_vibe(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to set vibe.".to_string(),
        };

        if args.trim().is_empty() {
            match self.state.db.get_vibe(&room) {
                Ok(Some(vibe)) => format!("Vibe: {}", vibe),
                Ok(None) => "No vibe set. Use /vibe <text> to set one.".to_string(),
                Err(e) => format!("Error: {}", e),
            }
        } else {
            match self.state.db.set_vibe(&room, Some(args.trim())) {
                Ok(_) => format!("Vibe set: {}", args.trim()),
                Err(e) => format!("Error: {}", e),
            }
        }
    }

    async fn cmd_journal(&self, args: &str, kind_str: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to add journal entries.".to_string(),
        };

        let author = self.username().unwrap_or_else(|| "unknown".to_string());

        if args.trim().is_empty() {
            return format!("Usage: /{} <text>", kind_str);
        }

        let kind = match JournalKind::from_str(kind_str) {
            Some(k) => k,
            None => return format!("Unknown journal kind: {}", kind_str),
        };

        match self.state.db.add_journal_entry(&room, &author, args.trim(), kind) {
            Ok(_) => format!("[{}] {}", kind_str, args.trim()),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_journal_list(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to view journal.".to_string(),
        };

        let kind = if args.trim().is_empty() {
            None
        } else {
            JournalKind::from_str(args.trim())
        };

        match self.state.db.get_journal_entries(&room, kind, 20) {
            Ok(entries) => {
                if entries.is_empty() {
                    return "No journal entries.".to_string();
                }
                let mut output = "─── Journal ───\r\n".to_string();
                for entry in entries {
                    let ts = entry.timestamp.format("%m-%d %H:%M");
                    output.push_str(&format!(
                        "[{}] {} ({}): {}\r\n",
                        ts, entry.kind, entry.author, entry.content
                    ));
                }
                output
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_bring(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to bind assets.".to_string(),
        };

        let parts: Vec<&str> = args.splitn(3, " as ").collect();
        if parts.len() < 2 {
            return "Usage: /bring <artifact_id> as <role>".to_string();
        }

        let artifact_id = parts[0].trim();
        let role = parts[1].trim();

        if artifact_id.is_empty() || role.is_empty() {
            return "Usage: /bring <artifact_id> as <role>".to_string();
        }

        let bound_by = self.username().unwrap_or_else(|| "unknown".to_string());

        match self.state.db.bind_asset(&room, role, artifact_id, None, &bound_by) {
            Ok(_) => format!("Bound '{}' as '{}'", artifact_id, role),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_drop(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to unbind assets.".to_string(),
        };

        let role = args.trim();
        if role.is_empty() {
            return "Usage: /drop <role>".to_string();
        }

        match self.state.db.unbind_asset(&room, role) {
            Ok(_) => format!("Unbound '{}'", role),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_examine(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to examine assets.".to_string(),
        };

        let role = args.trim();
        if role.is_empty() {
            return "Usage: /examine <role>".to_string();
        }

        match self.state.db.get_asset_binding(&room, role) {
            Ok(Some(binding)) => {
                let mut output = format!("═══ {} ═══\r\n", binding.role);
                output.push_str(&format!("Artifact: {}\r\n", binding.artifact_id));
                if let Some(notes) = &binding.notes {
                    output.push_str(&format!("Notes: {}\r\n", notes));
                }
                output.push_str(&format!("Bound by {} at {}\r\n", binding.bound_by, binding.bound_at.format("%Y-%m-%d %H:%M")));
                output
            }
            Ok(None) => format!("No asset bound as '{}'", role),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_inspire(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to add inspirations.".to_string(),
        };

        if args.trim().is_empty() {
            match self.state.db.get_inspirations(&room) {
                Ok(inspirations) => {
                    if inspirations.is_empty() {
                        return "No inspirations yet. Use /inspire <text> to add one.".to_string();
                    }
                    let mut output = "─── Inspirations ───\r\n".to_string();
                    for insp in inspirations {
                        output.push_str(&format!("• {}\r\n", insp.content));
                    }
                    output
                }
                Err(e) => format!("Error: {}", e),
            }
        } else {
            let added_by = self.username().unwrap_or_else(|| "unknown".to_string());
            match self.state.db.add_inspiration(&room, args.trim(), &added_by) {
                Ok(_) => format!("Added inspiration: {}", args.trim()),
                Err(e) => format!("Error: {}", e),
            }
        }
    }

    async fn cmd_dig(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to dig exits.".to_string(),
        };

        let parts: Vec<&str> = args.splitn(3, " to ").collect();
        if parts.len() < 2 {
            return "Usage: /dig <direction> to <room>".to_string();
        }

        let direction = parts[0].trim();
        let target = parts[1].trim();

        if direction.is_empty() || target.is_empty() {
            return "Usage: /dig <direction> to <room>".to_string();
        }

        // Create exit from current room
        if let Err(e) = self.state.db.add_exit(&room, direction, target) {
            return format!("Error: {}", e);
        }

        // Create reverse exit
        let reverse = match direction {
            "north" => "south",
            "south" => "north",
            "east" => "west",
            "west" => "east",
            "up" => "down",
            "down" => "up",
            "in" => "out",
            "out" => "in",
            _ => "back",
        };

        if let Err(e) = self.state.db.add_exit(target, reverse, &room) {
            return format!("Created {} → {} but failed reverse: {}", direction, target, e);
        }

        format!("Dug {} to {} (and {} back)", direction, target, reverse)
    }

    async fn cmd_go(&mut self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to navigate.".to_string(),
        };

        let direction = args.trim();
        if direction.is_empty() {
            return "Usage: /go <direction>".to_string();
        }

        let exits = match self.state.db.get_exits(&room) {
            Ok(e) => e,
            Err(e) => return format!("Error: {}", e),
        };

        match exits.get(direction) {
            Some(target) => {
                // Use existing join logic
                self.cmd_join(target).await
            }
            None => {
                let available: Vec<_> = exits.keys().collect();
                if available.is_empty() {
                    "No exits from this room.".to_string()
                } else {
                    format!("No exit '{}'. Available: {}", direction, available.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                }
            }
        }
    }

    async fn cmd_exits(&self) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to see exits.".to_string(),
        };

        match self.state.db.get_exits(&room) {
            Ok(exits) => {
                if exits.is_empty() {
                    "No exits. Use /dig <direction> to <room> to create one.".to_string()
                } else {
                    let mut output = "Exits:\r\n".to_string();
                    for (dir, target) in &exits {
                        output.push_str(&format!("  {} → {}\r\n", dir, target));
                    }
                    output
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_fork(&mut self, args: &str) -> String {
        let source = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to fork.".to_string(),
        };

        let new_name = args.trim();
        if new_name.is_empty() {
            return "Usage: /fork <new_room_name>".to_string();
        }

        if !new_name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return "Room name can only contain letters, numbers, dashes, and underscores.".to_string();
        }

        // Check if target exists
        {
            let world = self.state.world.read().await;
            if world.get_room(new_name).is_some() {
                return format!("Room '{}' already exists.", new_name);
            }
        }

        // Fork in database (creates room + copies context)
        if let Err(e) = self.state.db.fork_room(&source, new_name) {
            return format!("Error forking: {}", e);
        }

        // Create in memory
        {
            let mut world = self.state.world.write().await;
            world.create_room(new_name.to_string());
        }

        // Join the new room
        let join_result = self.cmd_join(new_name).await;
        format!("Forked '{}' from '{}'. Inherited: vibe, tags, assets, inspirations.\r\n\r\n{}", new_name, source, join_result)
    }
}
