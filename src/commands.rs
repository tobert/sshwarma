//! REPL command implementations

use crate::display::{EntryContent, EntrySource};
use crate::lua::WrapState;
use crate::model::{ModelBackend, ModelHandle};
use crate::ops;
use crate::ssh::SshHandler;
use opentelemetry::KeyValue;
use tracing::instrument;

/// Get or create the command counter
fn command_counter() -> opentelemetry::metrics::Counter<u64> {
    static COUNTER: std::sync::OnceLock<opentelemetry::metrics::Counter<u64>> =
        std::sync::OnceLock::new();
    COUNTER
        .get_or_init(|| {
            opentelemetry::global::meter("sshwarma")
                .u64_counter("sshwarma.commands.total")
                .with_description("Total number of slash commands executed")
                .build()
        })
        .clone()
}

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
    #[instrument(name = "cmd.dispatch", skip(self), fields(input.len = input.len()))]
    pub async fn handle_input(&mut self, input: &str) -> CommandResult {
        let input = input.trim();

        if let Some(rest) = input.strip_prefix('/') {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            let cmd = parts.first().unwrap_or(&"");
            let args = parts.get(1).copied().unwrap_or("");

            // Record command metric
            command_counter().add(1, &[KeyValue::new("command", cmd.to_string())]);

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
                "prompt" => CommandResult::normal(self.cmd_prompt(args).await),
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

Prompts:
  /prompt                        List prompts and targets
  /prompt <name> "<text>"        Create/update named prompt
  /prompt show <name|target>     Show prompt or target slots
  /prompt push <target> <name>   Add prompt to target
  /prompt pop <target>           Remove last slot
  /prompt rm <target> <slot>     Remove slot by index
  /prompt delete <name>          Delete a prompt

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

    #[instrument(name = "cmd.join", skip(self), fields(room.target = args.trim()))]
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
            Ok(_summary) => {
                // Update player state (handler-specific)
                if let Some(ref mut player) = self.player {
                    player.join_room(target.to_string());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(target));
                }
                self.render_room_ansi(target).await
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
            Ok(_summary) => {
                if let Some(ref mut player) = self.player {
                    player.join_room(room_name.to_string());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(room_name));
                }
                let room_output = self.render_room_ansi(room_name).await;
                format!("Created room '{}'.\r\n\r\n{}", room_name, room_output)
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

        self.render_room_ansi(&room_name).await
    }

    /// Render room summary using Lua ANSI formatter
    async fn render_room_ansi(&self, room_name: &str) -> String {
        let username = self.username().unwrap_or_else(|| "anonymous".to_string());

        let lua_runtime = match &self.lua_runtime {
            Some(rt) => rt,
            None => return format!("Room: {} (Lua runtime not initialized)", room_name),
        };

        let wrap_state = WrapState {
            room_name: Some(room_name.to_string()),
            username,
            model: ModelHandle::default(),
            shared_state: self.state.clone(),
        };

        let lua = lua_runtime.lock().await;
        match lua.render_look_ansi(wrap_state) {
            Ok(output) => output,
            Err(e) => format!("Room: {} (render error: {})", room_name, e),
        }
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

    #[instrument(name = "cmd.mention", skip(self, input), fields(model.name))]
    async fn cmd_mention(&mut self, input: &str) -> String {
        let input = input.trim_start_matches('@');
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let model_name = parts.first().unwrap_or(&"");
        tracing::Span::current().record("model.name", model_name);
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

    #[instrument(name = "cmd.run", skip(self), fields(tool.name))]
    async fn cmd_run(&self, args: &str) -> String {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        let tool_name = match parts.first() {
            Some(name) if !name.is_empty() => {
                tracing::Span::current().record("tool.name", *name);
                *name
            }
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
            "connect" | "add" => {
                let name = parts.get(1).copied().unwrap_or("");
                let url = parts.get(2).copied().unwrap_or("");
                if name.is_empty() || url.is_empty() {
                    return "Usage: /mcp connect <name> <url>".to_string();
                }

                self.state.mcp.add(name, url); // Non-blocking
                format!("Connecting to MCP server '{}' at {} (background)", name, url)
            }
            "disconnect" | "remove" => {
                let name = parts.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return "Usage: /mcp disconnect <name>".to_string();
                }

                if self.state.mcp.remove(name) {
                    format!("Removed MCP server '{}'", name)
                } else {
                    format!("MCP server '{}' not found", name)
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
                let new_room = summary.name.clone();
                if let Some(ref mut player) = self.player {
                    player.join_room(new_room.clone());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(&new_room));
                }
                self.render_room_ansi(&new_room).await
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
            Ok(_summary) => {
                if let Some(ref mut player) = self.player {
                    player.join_room(new_name.to_string());
                    let _ = self.state.db.update_session_room(&player.session_id, Some(new_name));
                }
                let room_output = self.render_room_ansi(new_name).await;
                format!(
                    "Forked '{}' from '{}'. Inherited: vibe, tags, assets, inspirations.\r\n\r\n{}",
                    new_name, source, room_output
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

    async fn cmd_prompt(&self, args: &str) -> String {
        let room = match self.current_room() {
            Some(r) => r,
            None => return "You need to be in a room to manage prompts.".to_string(),
        };

        let username = self.username().unwrap_or_else(|| "unknown".to_string());

        // Parse the command
        let args = args.trim();

        // Check for subcommands first
        if args.starts_with("list") {
            return self.cmd_prompt_list(&room).await;
        }

        if args.starts_with("delete ") {
            let name = args.trim_start_matches("delete ").trim();
            return self.cmd_prompt_delete(&room, name).await;
        }

        if args.starts_with("push ") {
            let rest = args.trim_start_matches("push ").trim();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 {
                return "Usage: /prompt push <target> <prompt_name>".to_string();
            }
            return self.cmd_prompt_push(&room, parts[0], parts[1]).await;
        }

        if args.starts_with("pop ") {
            let target = args.trim_start_matches("pop ").trim();
            return self.cmd_prompt_pop(&room, target).await;
        }

        if args.starts_with("rm ") {
            let rest = args.trim_start_matches("rm ").trim();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 {
                return "Usage: /prompt rm <target> <slot_index>".to_string();
            }
            let index: i64 = match parts[1].parse() {
                Ok(i) => i,
                Err(_) => return format!("Invalid slot index: {}", parts[1]),
            };
            return self.cmd_prompt_rm(&room, parts[0], index).await;
        }

        if args.starts_with("insert ") {
            let rest = args.trim_start_matches("insert ").trim();
            let parts: Vec<&str> = rest.splitn(3, ' ').collect();
            if parts.len() < 3 {
                return "Usage: /prompt insert <target> <slot_index> <prompt_name>".to_string();
            }
            let index: i64 = match parts[1].parse() {
                Ok(i) => i,
                Err(_) => return format!("Invalid slot index: {}", parts[1]),
            };
            return self.cmd_prompt_insert(&room, parts[0], index, parts[2]).await;
        }

        if args.starts_with("show ") {
            let name_or_target = args.trim_start_matches("show ").trim();
            return self.cmd_prompt_show(&room, name_or_target).await;
        }

        // Default: create/update a prompt
        // Format: /prompt <name> "<text>"
        if args.is_empty() {
            return self.cmd_prompt_list(&room).await;
        }

        // Parse: <name> "<text>" or <name> <text>
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        let name = parts[0];
        let content = parts.get(1).copied().unwrap_or("");

        if content.is_empty() {
            // Could be just a name - show it
            return self.cmd_prompt_show(&room, name).await;
        }

        // Strip surrounding quotes if present
        let content = content.trim();
        let content = if content.starts_with('"') && content.ends_with('"') && content.len() > 1 {
            &content[1..content.len()-1]
        } else {
            content
        };

        self.cmd_prompt_set(&room, name, content, &username).await
    }

    async fn cmd_prompt_list(&self, room: &str) -> String {
        let prompts = match self.state.db.list_prompts(room) {
            Ok(p) => p,
            Err(e) => return format!("Error loading prompts: {}", e),
        };

        let targets = match self.state.db.list_targets_with_slots(room) {
            Ok(t) => t,
            Err(e) => return format!("Error loading targets: {}", e),
        };

        if prompts.is_empty() && targets.is_empty() {
            return "No prompts defined.\r\n\r\nUsage:\r\n  /prompt <name> \"<text>\"        Create a named prompt\r\n  /prompt push <target> <name>   Add prompt to target's slots\r\n  /prompt show <name|target>     Show prompt or target slots\r\n  /prompt delete <name>          Delete a prompt".to_string();
        }

        let mut output = String::new();

        if !prompts.is_empty() {
            output.push_str("Prompts:\r\n");
            for p in &prompts {
                let preview = if p.content.len() > 50 {
                    format!("{}...", &p.content[..50])
                } else {
                    p.content.clone()
                };
                output.push_str(&format!("  {} → \"{}\"\r\n", p.name, preview));
            }
        }

        if !targets.is_empty() {
            if !output.is_empty() {
                output.push_str("\r\n");
            }
            output.push_str("Targets:\r\n");
            for (target, target_type) in &targets {
                let slots = match self.state.db.get_target_slots(room, target) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let slot_names: Vec<_> = slots.iter().map(|s| s.prompt_name.as_str()).collect();
                output.push_str(&format!(
                    "  {} ({}): [{}]\r\n",
                    target,
                    target_type,
                    slot_names.join(", ")
                ));
            }
        }

        output
    }

    async fn cmd_prompt_set(&self, room: &str, name: &str, content: &str, created_by: &str) -> String {
        if let Err(e) = self.state.db.set_prompt(room, name, content, created_by) {
            return format!("Error saving prompt: {}", e);
        }

        format!("Prompt '{}' set: \"{}\"", name, content)
    }

    async fn cmd_prompt_delete(&self, room: &str, name: &str) -> String {
        match self.state.db.delete_prompt(room, name) {
            Ok(true) => format!("Deleted prompt '{}' (and removed from all targets)", name),
            Ok(false) => format!("Prompt '{}' not found", name),
            Err(e) => format!("Error deleting prompt: {}", e),
        }
    }

    async fn cmd_prompt_show(&self, room: &str, name_or_target: &str) -> String {
        // First try to find as a prompt
        if let Ok(Some(prompt)) = self.state.db.get_prompt(room, name_or_target) {
            return format!(
                "Prompt '{}':\r\n  \"{}\"\r\n  Created by: {}",
                prompt.name,
                prompt.content,
                prompt.created_by.unwrap_or_else(|| "unknown".to_string())
            );
        }

        // Try as a target
        let slots = match self.state.db.get_target_slots(room, name_or_target) {
            Ok(s) => s,
            Err(e) => return format!("Error: {}", e),
        };

        if slots.is_empty() {
            return format!("No prompt or target named '{}'", name_or_target);
        }

        let mut output = format!("Target '{}':\r\n", name_or_target);
        for slot in &slots {
            let content_preview = match &slot.content {
                Some(c) if c.len() > 50 => format!("\"{}...\"", &c[..50]),
                Some(c) => format!("\"{}\"", c),
                None => "(prompt deleted)".to_string(),
            };
            output.push_str(&format!(
                "  [{}] {} → {}\r\n",
                slot.index, slot.prompt_name, content_preview
            ));
        }

        output
    }

    async fn cmd_prompt_push(&self, room: &str, target: &str, prompt_name: &str) -> String {
        // Verify prompt exists
        match self.state.db.get_prompt(room, prompt_name) {
            Ok(Some(_)) => {}
            Ok(None) => return format!("Prompt '{}' not found. Create it first with: /prompt {} \"<text>\"", prompt_name, prompt_name),
            Err(e) => return format!("Error: {}", e),
        }

        // Determine target type
        let target_type = self.determine_target_type(target).await;

        if let Err(e) = self.state.db.push_slot(room, target, &target_type, prompt_name) {
            return format!("Error adding slot: {}", e);
        }

        // Get current slot count
        let slots = self.state.db.get_target_slots(room, target).unwrap_or_default();
        format!("Added '{}' to {} ({} total slots)", prompt_name, target, slots.len())
    }

    async fn cmd_prompt_pop(&self, room: &str, target: &str) -> String {
        match self.state.db.pop_slot(room, target) {
            Ok(true) => format!("Removed last slot from '{}'", target),
            Ok(false) => format!("'{}' has no slots to remove", target),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_prompt_rm(&self, room: &str, target: &str, index: i64) -> String {
        match self.state.db.rm_slot(room, target, index) {
            Ok(true) => format!("Removed slot {} from '{}'", index, target),
            Ok(false) => format!("'{}' has no slot at index {}", target, index),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn cmd_prompt_insert(&self, room: &str, target: &str, index: i64, prompt_name: &str) -> String {
        // Verify prompt exists
        match self.state.db.get_prompt(room, prompt_name) {
            Ok(Some(_)) => {}
            Ok(None) => return format!("Prompt '{}' not found", prompt_name),
            Err(e) => return format!("Error: {}", e),
        }

        // Determine target type
        let target_type = self.determine_target_type(target).await;

        if let Err(e) = self.state.db.insert_slot(room, target, &target_type, index, prompt_name) {
            return format!("Error inserting slot: {}", e);
        }

        format!("Inserted '{}' at slot {} for '{}'", prompt_name, index, target)
    }

    /// Determine if a target is a model or user
    async fn determine_target_type(&self, target: &str) -> String {
        // Check if it's a known model
        if self.state.models.get(target).is_some() {
            return "model".to_string();
        }

        // Check connected users in world
        let world = self.state.world.read().await;
        for room in world.rooms.values() {
            if room.users.contains(&target.to_string()) {
                return "user".to_string();
            }
        }

        // Default to user (could also check DB users table)
        "user".to_string()
    }
}
