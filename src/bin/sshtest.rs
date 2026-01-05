//! SSH test client CLI
//!
//! Simple tool for testing sshwarma connections.
//!
//! Usage:
//!   cargo run --bin sshtest --features testing -- --cmd "/rooms"
//!   cargo run --bin sshtest --features testing -- --cmd "/join test" --cmd "hello"

use anyhow::Result;
use sshwarma::testing::SshTestClient;
use std::time::Duration;

#[derive(Default)]
struct Args {
    addr: String,
    key: Option<String>,
    username: String,
    commands: Vec<String>,
    wait_ms: u64,
    raw: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        addr: "localhost:2222".to_string(),
        username: whoami::username(),
        wait_ms: 500,
        ..Default::default()
    };

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--addr" | "-a" => {
                args.addr = iter.next().ok_or_else(|| anyhow::anyhow!("--addr requires value"))?;
            }
            "--key" | "-k" => {
                args.key = Some(iter.next().ok_or_else(|| anyhow::anyhow!("--key requires value"))?);
            }
            "--user" | "-u" => {
                args.username = iter.next().ok_or_else(|| anyhow::anyhow!("--user requires value"))?;
            }
            "--cmd" | "-c" => {
                args.commands.push(iter.next().ok_or_else(|| anyhow::anyhow!("--cmd requires value"))?);
            }
            "--wait" | "-w" => {
                args.wait_ms = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--wait requires value"))?
                    .parse()?;
            }
            "--raw" | "-r" => {
                args.raw = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {}", other);
                print_help();
                std::process::exit(1);
            }
        }
    }

    Ok(args)
}

fn print_help() {
    eprintln!(
        r#"sshtest - SSH test client for sshwarma

USAGE:
    sshtest [OPTIONS] --cmd <COMMAND>...

OPTIONS:
    -a, --addr <ADDR>      Server address [default: localhost:2222]
    -k, --key <PATH>       SSH private key [default: ~/.ssh/id_ed25519]
    -u, --user <NAME>      Username [default: current user]
    -c, --cmd <COMMAND>    Command to send (can be repeated)
    -w, --wait <MS>        Wait time after commands [default: 500]
    -r, --raw              Print raw bytes (hex dump)
    -h, --help             Print help

EXAMPLES:
    sshtest --cmd "/rooms"
    sshtest --cmd "/join test" --cmd "hello world"
    sshtest --addr 192.168.1.10:2222 --cmd "/who"
"#
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;

    if args.commands.is_empty() {
        eprintln!("error: at least one --cmd is required");
        print_help();
        std::process::exit(1);
    }

    eprintln!("connecting to {} as {}...", args.addr, args.username);

    let mut client = SshTestClient::connect(
        &args.addr,
        args.key.as_deref(),
        &args.username,
    )
    .await?;

    eprintln!("connected, sending {} command(s)...", args.commands.len());

    // Wait a bit for initial screen render
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Send each command with a small delay between
    for cmd in &args.commands {
        eprintln!("> {}", cmd);
        client.send(cmd).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Wait for output
    let output = client.wait_and_collect(Duration::from_millis(args.wait_ms)).await?;

    eprintln!("--- output ({} bytes) ---", output.len());

    if args.raw {
        // Hex dump
        for (i, chunk) in output.chunks(16).enumerate() {
            print!("{:04x}: ", i * 16);
            for byte in chunk {
                print!("{:02x} ", byte);
            }
            print!("  ");
            for byte in chunk {
                if byte.is_ascii_graphic() || *byte == b' ' {
                    print!("{}", *byte as char);
                } else {
                    print!(".");
                }
            }
            println!();
        }
    } else {
        // Text output (may contain ANSI escapes)
        print!("{}", String::from_utf8_lossy(&output));
    }

    eprintln!("---");

    client.close().await?;
    eprintln!("disconnected");

    Ok(())
}
