//! sshwarma-admin - CLI for managing sshwarma users and keys
//!
//! Usage:
//!   sshwarma-admin add <handle> <pubkey-file>
//!   sshwarma-admin add <handle> --key "ssh-ed25519 AAAA..."
//!   sshwarma-admin remove <handle>
//!   sshwarma-admin remove-key <pubkey>
//!   sshwarma-admin list
//!   sshwarma-admin keys <handle>

use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;

use sshwarma::db::Database;
use sshwarma::paths;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let db_path = paths::db_path();
    let db = Database::open(&db_path).context("failed to open database")?;

    match args[1].as_str() {
        "add" => cmd_add(&db, &args[2..])?,
        "remove" => cmd_remove(&db, &args[2..])?,
        "remove-key" => cmd_remove_key(&db, &args[2..])?,
        "list" => cmd_list(&db)?,
        "keys" => cmd_keys(&db, &args[2..])?,
        "help" | "--help" | "-h" => print_usage(),
        cmd => {
            eprintln!("Unknown command: {}", cmd);
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

fn print_usage() {
    eprintln!(
        r#"sshwarma-admin - Manage sshwarma users and SSH keys

Usage:
  sshwarma-admin add <handle> <pubkey-file>
  sshwarma-admin add <handle> --key "ssh-ed25519 AAAA..."
  sshwarma-admin remove <handle>
  sshwarma-admin remove-key "ssh-ed25519 AAAA..."
  sshwarma-admin list
  sshwarma-admin keys <handle>

Environment:
  SSHWARMA_DB    Override database path

Paths:
  Data:   {data}
  Config: {config}
  DB:     {db}

Examples:
  sshwarma-admin add amy ~/.ssh/id_ed25519.pub
  sshwarma-admin add bob --key "ssh-ed25519 AAAAC3... bob@laptop"
  sshwarma-admin list
  sshwarma-admin keys amy
"#,
        data = paths::data_dir().display(),
        config = paths::config_dir().display(),
        db = paths::db_path().display(),
    );
}

fn cmd_add(db: &Database, args: &[String]) -> Result<()> {
    if args.len() < 2 {
        anyhow::bail!("Usage: sshwarma-admin add <handle> <pubkey-file | --key 'key'>");
    }

    let handle = &args[0];

    let pubkey = if args[1] == "--key" {
        if args.len() < 3 {
            anyhow::bail!("--key requires a key string");
        }
        args[2].clone()
    } else {
        // Read from file
        let path = Path::new(&args[1]);
        fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?
            .trim()
            .to_string()
    };

    // Parse key to get type and comment
    let parts: Vec<&str> = pubkey.split_whitespace().collect();
    if parts.is_empty() {
        anyhow::bail!("invalid public key format");
    }

    let key_type = parts[0];
    let comment = parts.get(2).copied();

    // Store just the first two parts (type + base64) as the canonical key
    let canonical_key = if parts.len() >= 2 {
        format!("{} {}", parts[0], parts[1])
    } else {
        pubkey.clone()
    };

    db.add_pubkey(handle, &canonical_key, key_type, comment)?;

    println!("Added key for {}", handle);
    if let Some(c) = comment {
        println!("  Type: {}", key_type);
        println!("  Comment: {}", c);
    }

    Ok(())
}

fn cmd_remove(db: &Database, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("Usage: sshwarma-admin remove <handle>");
    }

    let handle = &args[0];

    if db.remove_user(handle)? {
        println!("Removed user {} and all their keys", handle);
    } else {
        println!("User {} not found", handle);
    }

    Ok(())
}

fn cmd_remove_key(db: &Database, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("Usage: sshwarma-admin remove-key <pubkey>");
    }

    let pubkey = &args[0];

    // Normalize key (take first two parts)
    let parts: Vec<&str> = pubkey.split_whitespace().collect();
    let canonical_key = if parts.len() >= 2 {
        format!("{} {}", parts[0], parts[1])
    } else {
        pubkey.clone()
    };

    if db.remove_pubkey(&canonical_key)? {
        println!("Removed key");
    } else {
        println!("Key not found");
    }

    Ok(())
}

fn cmd_list(db: &Database) -> Result<()> {
    let users = db.list_users()?;

    if users.is_empty() {
        println!("No users registered");
        return Ok(());
    }

    println!("Users:");
    for user in users {
        let last_seen = user
            .last_seen
            .as_deref()
            .unwrap_or("never");
        println!(
            "  {} ({} keys, last seen: {})",
            user.handle, user.key_count, last_seen
        );
    }

    Ok(())
}

fn cmd_keys(db: &Database, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("Usage: sshwarma-admin keys <handle>");
    }

    let handle = &args[0];
    let keys = db.list_keys_for_user(handle)?;

    if keys.is_empty() {
        println!("No keys for {}", handle);
        return Ok(());
    }

    println!("Keys for {}:", handle);
    for key in keys {
        let comment = key.comment.as_deref().unwrap_or("");
        // Show abbreviated key
        let key_preview = if key.pubkey.len() > 40 {
            format!("{}...{}", &key.pubkey[..20], &key.pubkey[key.pubkey.len() - 10..])
        } else {
            key.pubkey.clone()
        };
        println!("  {} {} ({})", key.key_type, key_preview, comment);
    }

    Ok(())
}
