//! Context builders for Lua
//!
//! Converts Rust notification types to Lua tables.

use mlua::{Lua, Result as LuaResult, Table};

/// Notification level for styling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationLevel {
    #[default]
    Info,
    Warning,
    Error,
}

impl NotificationLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            NotificationLevel::Info => "info",
            NotificationLevel::Warning => "warning",
            NotificationLevel::Error => "error",
        }
    }
}

/// Pending notification for Lua to process
#[derive(Debug, Clone)]
pub struct PendingNotification {
    pub message: String,
    pub created_at_ms: i64,
    pub ttl_ms: i64,
    pub level: NotificationLevel,
}

/// Build notifications array for Lua
pub fn build_notifications_table(
    lua: &Lua,
    notifications: &[PendingNotification],
) -> LuaResult<Table> {
    let arr = lua.create_table()?;

    for (i, n) in notifications.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("message", n.message.clone())?;
        entry.set("created_at_ms", n.created_at_ms)?;
        entry.set("ttl_ms", n.ttl_ms)?;
        entry.set("level", n.level.as_str())?;
        arr.set(i + 1, entry)?;
    }

    Ok(arr)
}
