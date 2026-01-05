//! Testing utilities for sshwarma
//!
//! Provides an SSH test client for automated testing against the sshwarma server.

mod ssh_client;

pub use ssh_client::SshTestClient;
