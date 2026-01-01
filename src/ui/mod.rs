//! UI module for terminal rendering
//!
//! Provides Lua-driven layout and rendering for sshwarma's terminal interface.
//!
//! # Modules
//!
//! - `layout` - Region constraint resolver, Area userdata
//! - `render` - Drawing API, RenderBuffer, widgets

pub mod layout;
pub mod render;

pub use layout::{Layout, LuaArea, Rect, RegionDef};
pub use render::{Cell, LuaDrawContext, RenderBuffer, Style};
