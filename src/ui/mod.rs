//! UI module for terminal rendering
//!
//! Provides Lua-driven layout and rendering for sshwarma's terminal interface.
//!
//! # Modules
//!
//! - `layout` - Region constraint resolver, Area userdata
//! - `render` - Drawing API, RenderBuffer, widgets
//! - `input` - Input buffer, key events, completion

pub mod input;
pub mod layout;
pub mod render;

pub use input::{CompletionItem, CompletionState, InputBuffer, KeyEvent, LuaInputBuffer};
pub use layout::{Layout, LuaArea, Rect, RegionDef};
pub use render::{Cell, LuaDrawContext, RenderBuffer, Style};
