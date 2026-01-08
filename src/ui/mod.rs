//! UI module for terminal rendering
//!
//! Provides Lua-driven rendering for sshwarma's terminal interface.
//!
//! Input handling is done entirely in Lua (embedded/ui/input.lua, mode.lua).

pub mod render;

pub use render::{Cell, LuaDrawContext, RenderBuffer, Style};
