//! sshwarma - SSH-accessible collaborative space for humans and models
//!
//! This library provides the core types and database for sshwarma.
//! The main binary is in `main.rs`, admin CLI in `bin/sshwarma-admin.rs`.

pub mod ansi;
pub mod config;
pub mod db;
pub mod internal_tools;
pub mod interp;
pub mod line_editor;
pub mod llm;
pub mod lua;
pub mod mcp;
pub mod mcp_server;
pub mod model;
pub mod ops;
pub mod paths;
pub mod player;
pub mod rules;
pub mod ssh;
pub mod state;
pub mod status;
pub mod ui;
pub mod world;

#[cfg(feature = "testing")]
pub mod testing;
