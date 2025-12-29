//! REPL command implementations

use crate::lua::WrapState;
use crate::model::{ModelBackend, ModelHandle};
use crate::ops;
use crate::ssh::SshHandler;
use crate::display::{EntryContent, EntrySource};

/// Result of a command execution
pub struct CommandResult {
    /// Output text to display
    pub text: String,
    /// If true, output is displayed but excluded from context/history
    pub ephemeral: bool,
}

impl CommandResult {
    /// Create a normal command result (included in history)
    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ephemeral: false,
        }
    }

    /// Create an ephemeral result (displayed but not in history)
    pub fn ephemeral(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ephemeral: true,
        }
    }

    /// Check if result has any output
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl SshHandler {
    pub async fn handle_input(&mut self, input: &str) -> CommandResult {
        let input = input.trim();

        if input.starts_with('/') {
            let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
            let cmd = parts.first().unwrap_or(&"");
            let args = parts.get(1).copied().unwrap_or("");

            match *cmd {
                "help" => CommandResult::normal(self.cmd_help()),
                "rooms" => CommandResult::normal(self.cmd_rooms().await),
                "who" => CommandResult::normal(self.cmd_who().await),
                "join" => CommandResult::normal(self.cmd_join(args).await),
                "create" => CommandResult::normal(self.cmd_create(args).await),
                "leave" => CommandResult::normal(self.cmd_leave().await),
                "look" => CommandResult::normal(self.cmd_look(args).await),
                "examine" => CommandResult::normal(self.cmd_examine(args).await),
                "history" => CommandResult::normal(self.cmd_history(args).await),
                "tools" => CommandResult::normal(self.cmd_tools().await),
                "run" => CommandResult::normal(self.cmd_run(args).await),
                "mcp" => CommandResult::normal(self.cmd_mcp(args).await),
                // Room context commands
                "vibe" => CommandResult::normal(self.cmd_vibe(args).await),
                "note" => CommandResult::normal(self.cmd_journal(args, "note").await),
                "decide" => CommandResult::normal(self.cmd_journal(args, "decision").await),
                "idea" => CommandResult::normal(self.cmd_journal(args, "idea").await),
                "milestone" => CommandResult::normal(self.cmd_journal(args, "milestone").await),
                "journal" => CommandResult::normal(self.cmd_journal_list(args).await),
                "bring" => CommandResult::normal(self.cmd_bring(args).await),
                "drop" => CommandResult::normal(self.cmd_drop(args).await),
                "inspire" => CommandResult::normal(self.cmd_inspire(args).await),
                "dig" => CommandResult::normal(self.cmd_dig(args).await),
                "go" => CommandResult::normal(self.cmd_go(args).await),
                "exits" => CommandResult::normal(self.cmd_exits().await),
                "fork" => CommandResult::normal(self.cmd_fork(args).await),
                "nav" => CommandResult::normal(self.cmd_nav(args).await),
                // Debug commands - ephemeral (not included in history/context)
                "wrap" => CommandResult::ephemeral(self.cmd_wrap(args).await),
                "quit" => CommandResult::normal("Goodbye!"),
                _ => CommandResult::normal(format!("Unknown command: /{}", cmd)),
            }
        } else if input.starts_with('@') {
            CommandResult::normal(self.cmd_mention(input).await)
        } else {
            CommandResult::normal(self.cmd_say(input).await)
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
  /nav [on|off]       Toggle model navigation

Communication:
  <text>              Say to room
  @model <msg>        Message a model

Tools:
  /tools              List available tools
  /run <tool> [args]  Invoke tool with JSON args

Debug:
  /wrap [model]       Preview context composition

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
        match ops::rooms(&self.state).await {
            Ok(rooms) => {
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
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_who(&self) -> String {
        let room_name = match self.current_room() {
            Some(r) => r,
            None => return "Online: you (more coming soon)".to_string(),
        };

        match ops::who(&self.state, &room_name).await {
            Ok(users) => {
                if users.is_empty() {
                    format!("In {}: (empty)", room_name)
                } else {
                    format!("In {}: {}", room_name, users.join(", "))
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_join(&mut self, args: &str) -> String {
        let target = args.trim();
        if target.is_empty() {
            return "Usage: /join <room>".to_string();
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        let current = self.current_room();

        match ops::join(&self.state, &username, current.as_deref(), target).await {
            Ok(summary) => {
                // Update player state (handler-specific)
                if let Some(ref mut player) = self.player {
                    player.join_room(target.to_string());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(target));
                }
                Self::format_room_summary(&summary)
            }
            Err(e) => e.to_string(),
        }
    }

    async fn cmd_create(&mut self, args: &str) -> String {
        let room_name = args.trim();
        if room_name.is_empty() {
            return "Usage: /create <name>".to_string();
        }

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        let current = self.current_room();

        match ops::create_room(&self.state, &username, room_name, current.as_deref()).await {
            Ok(summary) => {
                if let Some(ref mut player) = self.player {
                    player.join_room(room_name.to_string());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(room_name));
                }
                format!("Created room '{}'.\r\n\r\n{}", room_name, Self::format_room_summary(&summary))
            }
            Err(e) => e.to_string(),
        }
    }

    async fn cmd_leave(&mut self) -> String {
        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        let current_room = self.current_room();

        match current_room {
            Some(room_name) => {
                if let Err(e) = ops::leave(&self.state, &username, &room_name).await {
                    return format!("Error: {}", e);
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

        let room_name = match self.current_room() {
            Some(r) => r,
            None => {
                return "═══ Lobby ═══\r\n\r\nYou're in the lobby. Use /rooms to see rooms, /join <room> to enter one.".to_string();
            }
        };

        match ops::look(&self.state, &room_name).await {
            Ok(summary) => Self::format_room_summary(&summary),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Format a RoomSummary for TTY display
    fn format_room_summary(s: &ops::RoomSummary) -> String {
        let mut out = format!("═══ {} ═══\r\n", s.name);

        if let Some(ref desc) = s.description {
            out.push_str(&format!("{}\r\n", desc));
        }
        out.push_str("\r\n");

        if s.users.is_empty() {
            out.push_str("Nobody else is here.\r\n");
        } else {
            out.push_str(&format!("Users: {}\r\n", s.users.join(", ")));
        }

        if !s.models.is_empty() {
            out.push_str(&format!("Models: {}\r\n", s.models.join(", ")));
        }

        if s.artifact_count > 0 {
            out.push_str(&format!("Artifacts: {} items\r\n", s.artifact_count));
        }

        if let Some(ref vibe) = s.vibe {
            out.push_str(&format!("Vibe: {}\r\n", vibe));
        }

        if !s.exits.is_empty() {
            let exit_list: Vec<_> = s.exits.keys().collect();
            out.push_str(&format!("Exits: {}\r\n", exit_list.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        }

        out
    }

    async fn cmd_history(&self, args: &str) -> String {
        let limit: usize = args.trim().parse().unwrap_or(50);
        let limit = limit.min(200);

        let room_name = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to see history.".to_string(),
        };

        match ops::history(&self.state, &room_name, limit).await {
            Ok(entries) => {
                if entries.is_empty() {
                    "No messages in this room yet.".to_string()
                } else {
                    let mut output =
                        format!("─── Last {} messages in {} ───\r\n", entries.len(), room_name);
                    for entry in entries {
                        output.push_str(&format!(
                            "[{}] {}: {}\r\n",
                            entry.timestamp, entry.sender, entry.content
                        ));
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

        match self.current_room() {
            Some(room_name) => {
                if let Err(e) = ops::say(&self.state, &room_name, &username, message).await {
                    return format!("Error: {}", e);
                }
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

        // Build context from room ledger
        let history = if let Some(ref room_name) =
            self.player.as_ref().and_then(|p| p.current_room.clone())
        {
            let world = self.state.world.read().await;
            if let Some(room) = world.get_room(room_name) {
                room.ledger
                    .recent(10)
                    .iter()
                    .filter(|e| !e.ephemeral)
                    .filter_map(|entry| match &entry.content {
                        EntryContent::Chat(text) => {
                            let role = match &entry.source {
                                EntrySource::User(_) => "user",
                                EntrySource::Model { .. } => "assistant",
                                EntrySource::System | EntrySource::Command { .. } => return None,
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
                    use crate::display::LedgerEntry;
                    use chrono::Utc;

                    let entry = LedgerEntry {
                        id: crate::display::EntryId(0),
                        timestamp: Utc::now(),
                        source: EntrySource::Model {
                            name: model.short_name.clone(),
                            is_streaming: false,
                        },
                        content: EntryContent::Chat(response.clone()),
                        mutable: false,
                        ephemeral: false,
                        collapsible: true,
                    };

                    {
                        let mut world = self.state.world.write().await;
                        if let Some(room) = world.get_room_mut(room_name) {
                            room.add_entry(entry.source.clone(), entry.content.clone());
                        }
                    }
                    let _ = self.state.db.add_ledger_entry(room_name, &entry);
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
        match ops::tools(&self.state).await {
            Ok(tools) => {
                if tools.is_empty() {
                    "No tools available. Use /mcp connect <name> <url> to add an MCP server."
                        .to_string()
                } else {
                    let mut output = "Available tools:\r\n".to_string();
                    for tool in tools {
                        output.push_str(&format!(
                            "  {} ({})\r\n    {}\r\n",
                            tool.name, tool.source, tool.description
                        ));
                    }
                    output
                }
            }
            Err(e) => format!("Error: {}", e),
        }
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
            match ops::get_vibe(&self.state, &room).await {
                Ok(Some(vibe)) => format!("Vibe: {}", vibe),
                Ok(None) => "No vibe set. Use /vibe <text> to set one.".to_string(),
                Err(e) => format!("Error: {}", e),
            }
        } else {
            match ops::set_vibe(&self.state, &room, args.trim()).await {
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

        let kind = match kind_str {
            "note" => ops::JournalKind::Note,
            "decision" => ops::JournalKind::Decision,
            "idea" => ops::JournalKind::Idea,
            "milestone" => ops::JournalKind::Milestone,
            _ => return format!("Unknown journal kind: {}", kind_str),
        };

        match ops::add_journal(&self.state, &room, &author, args.trim(), kind).await {
            Ok(_) => format!("[{}] {}", kind_str, args.trim()),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_journal_list(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to view journal.".to_string(),
        };

        let kind_filter = if args.trim().is_empty() {
            None
        } else {
            Some(args.trim())
        };

        match ops::get_journal(&self.state, &room, kind_filter, 20).await {
            Ok(entries) => {
                if entries.is_empty() {
                    return "No journal entries.".to_string();
                }
                let mut output = "─── Journal ───\r\n".to_string();
                for entry in entries {
                    output.push_str(&format!(
                        "[{}] {} ({}): {}\r\n",
                        entry.timestamp, entry.kind, entry.author, entry.content
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

        match ops::bind_asset(&self.state, &room, role, artifact_id, &bound_by).await {
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

        match ops::unbind_asset(&self.state, &room, role).await {
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

        match ops::examine_asset(&self.state, &room, role).await {
            Ok(Some(binding)) => {
                let mut output = format!("═══ {} ═══\r\n", binding.role);
                output.push_str(&format!("Artifact: {}\r\n", binding.artifact_id));
                if let Some(notes) = &binding.notes {
                    output.push_str(&format!("Notes: {}\r\n", notes));
                }
                output.push_str(&format!("Bound by {} at {}\r\n", binding.bound_by, binding.bound_at));
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
            match ops::get_inspirations(&self.state, &room).await {
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
            match ops::add_inspiration(&self.state, &room, args.trim(), &added_by).await {
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

        match ops::dig(&self.state, &room, direction, target).await {
            Ok(reverse) => format!("Dug {} to {} (and {} back)", direction, target, reverse),
            Err(e) => format!("Error: {}", e),
        }
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

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        match ops::go(&self.state, &username, &room, direction).await {
            Ok(summary) => {
                if let Some(ref mut player) = self.player {
                    player.join_room(summary.name.clone());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(&summary.name));
                }
                Self::format_room_summary(&summary)
            }
            Err(e) => e.to_string(),
        }
    }

    async fn cmd_exits(&self) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to see exits.".to_string(),
        };

        match ops::exits(&self.state, &room).await {
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

        let username = match &self.player {
            Some(p) => p.username.clone(),
            None => return "Not authenticated".to_string(),
        };

        match ops::fork_room(&self.state, &username, &source, new_name).await {
            Ok(summary) => {
                if let Some(ref mut player) = self.player {
                    player.join_room(new_name.to_string());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(new_name));
                }
                format!(
                    "Forked '{}' from '{}'. Inherited: vibe, tags, assets, inspirations.\r\n\r\n{}",
                    new_name, source, Self::format_room_summary(&summary)
                )
            }
            Err(e) => e.to_string(),
        }
    }

    async fn cmd_nav(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to configure navigation.".to_string(),
        };

        let args = args.trim().to_lowercase();

        if args.is_empty() {
            // Show current status
            match ops::get_room_navigation(&self.state, &room).await {
                Ok(enabled) => {
                    let status = if enabled { "enabled" } else { "disabled" };
                    format!(
                        "Model navigation in '{}' is {}.\r\n\
                         Use /nav on or /nav off to change.",
                        room, status
                    )
                }
                Err(e) => format!("Error: {}", e),
            }
        } else if args == "on" {
            match ops::set_room_navigation(&self.state, &room, true).await {
                Ok(_) => format!("Model navigation enabled in '{}'.", room),
                Err(e) => format!("Error: {}", e),
            }
        } else if args == "off" {
            match ops::set_room_navigation(&self.state, &room, false).await {
                Ok(_) => format!(
                    "Model navigation disabled in '{}'.\r\n\
                     Models can no longer join/leave/create rooms.",
                    room
                ),
                Err(e) => format!("Error: {}", e),
            }
        } else {
            "Usage: /nav [on|off]".to_string()
        }
    }

    async fn cmd_wrap(&self, args: &str) -> String {
        let username = self.username().unwrap_or_else(|| "anonymous".to_string());
        let room_name = self.current_room();

        // Get model from args or use a default preview model
        let model = if args.trim().is_empty() {
            // Use a mock model for preview
            ModelHandle {
                short_name: "preview".to_string(),
                display_name: "Preview Model".to_string(),
                backend: ModelBackend::Mock {
                    prefix: "[preview]".to_string(),
                },
                available: true,
                system_prompt: Some("This is a preview of context composition.".to_string()),
                context_window: Some(30000),
            }
        } else {
            // Look up specified model
            match self.state.models.get(args.trim()) {
                Some(m) => m.clone(),
                None => {
                    let available: Vec<_> = self.state.models.available()
                        .iter()
                        .map(|m| m.short_name.as_str())
                        .collect();
                    return format!(
                        "Unknown model '{}'. Available: {}",
                        args.trim(),
                        available.join(", ")
                    );
                }
            }
        };

        let target_tokens = model.context_window.unwrap_or(30000);

        // Get lua_runtime and compose context
        let lua_runtime = match &self.lua_runtime {
            Some(rt) => rt,
            None => return "Lua runtime not initialized.".to_string(),
        };

        let wrap_state = WrapState {
            room_name,
            username,
            model: model.clone(),
            shared_state: self.state.clone(),
        };

        let lua = lua_runtime.lock().await;
        match lua.compose_context(wrap_state, target_tokens) {
            Ok(result) => {
                let system_tokens = result.system_prompt.len() / 4;
                let context_tokens = result.context.len() / 4;

                format!(
                    "─── wrap() preview for @{} ───\r\n\r\n\
                     === SYSTEM PROMPT ({} tokens, cacheable) ===\r\n{}\r\n\r\n\
                     === CONTEXT ({} tokens, dynamic) ===\r\n{}\r\n\r\n\
                     Total: ~{} tokens of {} budget",
                    model.short_name,
                    system_tokens,
                    result.system_prompt,
                    context_tokens,
                    if result.context.is_empty() { "(empty)" } else { &result.context },
                    system_tokens + context_tokens,
                    target_tokens
                )
            }
            Err(e) => format!("Error composing context: {}", e),
        }
    }
}
