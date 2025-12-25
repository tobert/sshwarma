//! sshwarma - SSH-accessible partyline for humans and models
//!
//! This library provides the core types and database for sshwarma.
//! The main binary is in `main.rs`, admin CLI in `bin/sshwarma-admin.rs`.

pub mod db;
pub mod world;
pub mod player;
pub mod model;
pub mod comm;
pub mod interp;
pub mod mcp;
pub mod mcp_server;
pub mod llm;
