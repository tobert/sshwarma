//! Filesystem watcher for Lua hot reload
//!
//! Watches ~/.config/sshwarma/lua/ for changes and broadcasts events
//! to all sessions via LuaReloadSender.

use crate::lua::reload::{LuaReloadEvent, LuaReloadSender};
use crate::paths;
use anyhow::{Context, Result};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Convert a file path to a Lua module name
///
/// Examples:
///   ~/.config/sshwarma/lua/screen.lua -> "screen"
///   ~/.config/sshwarma/lua/ui/bars.lua -> "ui.bars"
///   ~/.config/sshwarma/lua/ui/init.lua -> "ui"
fn path_to_module_name(path: &Path, base: &Path) -> Option<String> {
    // Get relative path from base lua directory
    let rel = path.strip_prefix(base).ok()?;

    // Must be a .lua file
    if rel.extension().is_none_or(|ext| ext != "lua") {
        return None;
    }

    // Convert path components to module name
    let stem = rel.file_stem()?.to_str()?;
    let parent = rel.parent()?;

    let module_name = if parent.as_os_str().is_empty() {
        // Top-level file: screen.lua -> "screen"
        stem.to_string()
    } else if stem == "init" {
        // init.lua: ui/init.lua -> "ui"
        parent
            .iter()
            .filter_map(|s| s.to_str())
            .collect::<Vec<_>>()
            .join(".")
    } else {
        // Nested file: ui/bars.lua -> "ui.bars"
        let parts: Vec<_> = parent.iter().filter_map(|s| s.to_str()).collect();
        if parts.is_empty() {
            stem.to_string()
        } else {
            format!("{}.{}", parts.join("."), stem)
        }
    };

    Some(module_name)
}

/// Start the filesystem watcher
///
/// Returns a handle that keeps the watcher alive. Drop it to stop watching.
pub fn start_watcher(sender: LuaReloadSender) -> Result<WatcherHandle> {
    let lua_dir = paths::config_dir().join("lua");

    // Create directory if it doesn't exist
    if !lua_dir.exists() {
        std::fs::create_dir_all(&lua_dir)
            .with_context(|| format!("failed to create lua dir: {}", lua_dir.display()))?;
        info!("📂 created lua directory: {}", lua_dir.display());
    }

    let (tx, rx) = mpsc::channel();

    // Configure watcher with reasonable debounce
    let config = Config::default().with_poll_interval(Duration::from_millis(500));

    let mut watcher: RecommendedWatcher =
        Watcher::new(tx, config).context("failed to create filesystem watcher")?;

    watcher
        .watch(&lua_dir, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch: {}", lua_dir.display()))?;

    info!("👀 watching for Lua changes: {}", lua_dir.display());

    // Spawn thread to process events
    let lua_dir_clone = lua_dir.clone();
    std::thread::spawn(move || {
        process_events(rx, &lua_dir_clone, &sender);
    });

    Ok(WatcherHandle {
        _watcher: watcher,
        lua_dir,
    })
}

/// Process filesystem events and broadcast to sessions
fn process_events(
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    base: &Path,
    sender: &LuaReloadSender,
) {
    for result in rx {
        match result {
            Ok(event) => {
                // Only care about file changes, not directory events
                let kind = &event.kind;
                let is_relevant = matches!(
                    kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                );

                if !is_relevant {
                    continue;
                }

                // Process each affected path
                for path in &event.paths {
                    if let Some(module_name) = path_to_module_name(path, base) {
                        let reload_event = match kind {
                            notify::EventKind::Create(_) => LuaReloadEvent::ModuleCreated {
                                module_name: module_name.clone(),
                                path: path.clone(),
                            },
                            notify::EventKind::Modify(_) => LuaReloadEvent::ModuleChanged {
                                module_name: module_name.clone(),
                                path: path.clone(),
                            },
                            notify::EventKind::Remove(_) => LuaReloadEvent::ModuleDeleted {
                                module_name: module_name.clone(),
                                path: path.clone(),
                            },
                            _ => continue,
                        };

                        debug!(
                            "🔄 lua {:?}: {} ({})",
                            kind,
                            module_name,
                            path.display()
                        );
                        sender.send(reload_event);
                    }
                }
            }
            Err(e) => {
                warn!("filesystem watcher error: {}", e);
            }
        }
    }

    debug!("filesystem watcher stopped");
}

/// Handle that keeps the watcher alive
///
/// Drop this to stop watching.
pub struct WatcherHandle {
    _watcher: RecommendedWatcher,
    lua_dir: PathBuf,
}

impl WatcherHandle {
    /// Get the directory being watched
    pub fn lua_dir(&self) -> &Path {
        &self.lua_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_path_to_module_name() {
        let base = PathBuf::from("/home/user/.config/sshwarma/lua");

        // Top-level module
        assert_eq!(
            path_to_module_name(&base.join("screen.lua"), &base),
            Some("screen".to_string())
        );

        // Nested module
        assert_eq!(
            path_to_module_name(&base.join("ui/bars.lua"), &base),
            Some("ui.bars".to_string())
        );

        // init.lua -> parent module name
        assert_eq!(
            path_to_module_name(&base.join("ui/init.lua"), &base),
            Some("ui".to_string())
        );

        // Deeply nested
        assert_eq!(
            path_to_module_name(&base.join("commands/room/create.lua"), &base),
            Some("commands.room.create".to_string())
        );

        // Non-lua files ignored
        assert_eq!(path_to_module_name(&base.join("README.md"), &base), None);
        assert_eq!(path_to_module_name(&base.join("data.json"), &base), None);

        // Outside base path
        assert_eq!(
            path_to_module_name(&PathBuf::from("/other/screen.lua"), &base),
            None
        );
    }
}
