//! UI module for terminal rendering
//!
//! Provides Lua-driven rendering for sshwarma's terminal interface.
//!
//! # Modules
//!
//! - `render` - Drawing API, RenderBuffer, widgets
//! - `input` - Input buffer, key events

pub mod input;
pub mod render;

pub use input::{InputBuffer, KeyEvent};
pub use render::{Cell, LuaDrawContext, RenderBuffer, Style};
