//! sshwarma - SSH-accessible collaborative space for humans and models
//!
//! This library provides the core types and database for sshwarma.
//! The main binary is in `main.rs`, admin CLI in `bin/sshwarma-admin.rs`.

pub mod ansi;
pub mod commands;
pub mod comm;
pub mod completion;
pub mod config;
pub mod db;
pub mod display;
pub mod internal_tools;
pub mod interp;
pub mod line_editor;
pub mod llm;
pub mod lua;
pub mod mcp;
pub mod mcp_server;
pub mod model;
pub mod ops;
pub mod player;
pub mod ssh;
pub mod state;
pub mod world;
