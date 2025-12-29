//! HUD context builder for Lua
//!
//! Converts Rust HudState into Lua-compatible tables.

use crate::display::hud::{HudState, McpConnectionState, ParticipantKind, ParticipantStatus, Presence};
use mlua::{Lua, Result as LuaResult, Table, Value};

/// Build a Lua table representing the HUD context
///
/// The returned table contains:
/// - room: string or nil (current room name)
/// - session_start_ms: number (session start time in milliseconds)
/// - participants: array of participant tables
/// - mcp: array of MCP connection tables
/// - exits: table mapping direction strings to room names
pub fn build_hud_context(lua: &Lua, state: &HudState) -> LuaResult<Table> {
    let ctx = lua.create_table()?;

    // Room name (nil for lobby)
    if let Some(ref room) = state.room_name {
        ctx.set("room", room.clone())?;
    } else {
        ctx.set("room", Value::Nil)?;
    }

    // Session start time in milliseconds
    let session_start_ms = state.session_start.timestamp_millis();
    ctx.set("session_start_ms", session_start_ms)?;

    // Participants array
    let participants = build_participants_table(lua, &state.participants)?;
    ctx.set("participants", participants)?;

    // MCP connections array
    let mcp = build_mcp_table(lua, &state.mcp_connections)?;
    ctx.set("mcp", mcp)?;

    // Exits table
    let exits = lua.create_table()?;
    for (dir, room) in &state.exits {
        exits.set(dir.clone(), room.clone())?;
    }
    ctx.set("exits", exits)?;

    Ok(ctx)
}

/// Build participants array for Lua
fn build_participants_table(lua: &Lua, participants: &[Presence]) -> LuaResult<Table> {
    let arr = lua.create_table()?;

    for (i, p) in participants.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("name", p.name.clone())?;

        // Kind as string
        let kind_str = match p.kind {
            ParticipantKind::User => "user",
            ParticipantKind::Model => "model",
        };
        entry.set("kind", kind_str)?;

        // Status as string
        let (status_str, status_detail) = status_to_strings(&p.status);
        entry.set("status", status_str)?;
        if let Some(detail) = status_detail {
            entry.set("status_detail", detail)?;
        }

        // Updated at as milliseconds
        entry.set("updated_at_ms", p.updated_at.timestamp_millis())?;

        // Lua arrays are 1-indexed
        arr.set(i + 1, entry)?;
    }

    Ok(arr)
}

/// Convert ParticipantStatus to (status_string, optional_detail)
fn status_to_strings(status: &ParticipantStatus) -> (&'static str, Option<String>) {
    match status {
        ParticipantStatus::Idle => ("idle", None),
        ParticipantStatus::Thinking => ("thinking", None),
        ParticipantStatus::RunningTool(name) => ("running_tool", Some(name.clone())),
        ParticipantStatus::Error(msg) => ("error", Some(msg.clone())),
        ParticipantStatus::Offline => ("offline", None),
        ParticipantStatus::Emoji(e) => ("emoji", Some(e.clone())),
    }
}

/// Build MCP connections array for Lua
fn build_mcp_table(lua: &Lua, connections: &[McpConnectionState]) -> LuaResult<Table> {
    let arr = lua.create_table()?;

    for (i, m) in connections.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("name", m.name.clone())?;
        entry.set("tools", m.tool_count)?;
        entry.set("connected", m.connected)?;
        entry.set("calls", m.call_count)?;

        if let Some(ref last_tool) = m.last_tool {
            entry.set("last_tool", last_tool.clone())?;
        }

        arr.set(i + 1, entry)?;
    }

    Ok(arr)
}

/// Pending notification for Lua to process
#[derive(Debug, Clone)]
pub struct PendingNotification {
    pub message: String,
    pub created_at_ms: i64,
    pub ttl_ms: i64,
}

/// Build notifications array for Lua
pub fn build_notifications_table(lua: &Lua, notifications: &[PendingNotification]) -> LuaResult<Table> {
    let arr = lua.create_table()?;

    for (i, n) in notifications.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("message", n.message.clone())?;
        entry.set("created_at_ms", n.created_at_ms)?;
        entry.set("ttl_ms", n.ttl_ms)?;
        arr.set(i + 1, entry)?;
    }

    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_hud_context() {
        let lua = Lua::new();
        let mut state = HudState::new();
        state.room_name = Some("test_room".to_string());
        state.add_user("alice".to_string());
        state.add_model("qwen-8b".to_string());
        state.exits.insert("north".to_string(), "other_room".to_string());

        let ctx = build_hud_context(&lua, &state).expect("should build context");

        // Check room
        let room: String = ctx.get("room").expect("should have room");
        assert_eq!(room, "test_room");

        // Check participants
        let participants: Table = ctx.get("participants").expect("should have participants");
        let p1: Table = participants.get(1).expect("should have first participant");
        let name: String = p1.get("name").expect("should have name");
        assert_eq!(name, "alice");

        // Check exits
        let exits: Table = ctx.get("exits").expect("should have exits");
        let north: String = exits.get("north").expect("should have north exit");
        assert_eq!(north, "other_room");
    }

    #[test]
    fn test_status_to_strings() {
        assert_eq!(status_to_strings(&ParticipantStatus::Idle), ("idle", None));
        assert_eq!(status_to_strings(&ParticipantStatus::Thinking), ("thinking", None));
        assert_eq!(
            status_to_strings(&ParticipantStatus::RunningTool("sample".to_string())),
            ("running_tool", Some("sample".to_string()))
        );
    }
}
