//! SSH test client CLI
//!
//! Simple tool for testing sshwarma connections.
//!
//! Usage:
//!   cargo run --bin sshtest --features testing -- --cmd "/rooms"
//!   cargo run --bin sshtest --features testing -- --cmd "/join test" --wait-for "test>"

use anyhow::Result;
use sshwarma::testing::SshTestClient;
use std::time::Duration;

#[derive(Default)]
struct Args {
    addr: String,
    key: Option<String>,
    username: String,
    commands: Vec<String>,
    wait_for: Option<String>,
    timeout_ms: u64,
    raw: bool,
    offset: usize,
    limit: Option<usize>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        addr: "localhost:2222".to_string(),
        username: whoami::username(),
        timeout_ms: 5000,
        ..Default::default()
    };

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--addr" | "-a" => {
                args.addr = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--addr requires value"))?;
            }
            "--key" | "-k" => {
                args.key = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--key requires value"))?,
                );
            }
            "--user" | "-u" => {
                args.username = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--user requires value"))?;
            }
            "--cmd" | "-c" => {
                args.commands.push(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--cmd requires value"))?,
                );
            }
            "--wait-for" | "-f" => {
                args.wait_for = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--wait-for requires value"))?,
                );
            }
            "--timeout" | "-t" => {
                args.timeout_ms = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--timeout requires value"))?
                    .parse()?;
            }
            "--raw" | "-r" => {
                args.raw = true;
            }
            "--offset" | "-o" => {
                args.offset = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--offset requires value"))?
                    .parse()?;
            }
            "--limit" | "-l" => {
                args.limit = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--limit requires value"))?
                        .parse()?,
                );
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
    -a, --addr <ADDR>        Server address [default: localhost:2222]
    -k, --key <PATH>         SSH private key [default: ~/.ssh/id_ed25519]
    -u, --user <NAME>        Username [default: current user]
    -c, --cmd <COMMAND>      Command to send (can be repeated)
    -f, --wait-for <PATTERN> Wait until pattern appears in output
    -t, --timeout <MS>       Max wait time [default: 5000]
    -r, --raw                Print raw bytes (hex dump)
    -o, --offset <N>         Skip first N lines of output
    -l, --limit <N>          Show only N lines of output
    -h, --help               Print help

OUTPUT:
    Default output shows ANSI escapes as readable \e[...m sequences.
    Use --raw for hex dump when debugging binary data.

EXAMPLES:
    sshtest --cmd "/rooms"
    sshtest --cmd "/join test" --wait-for "test>"
    sshtest --cmd "/rooms" --offset 5 --limit 10
    sshtest --cmd "/who" --raw
"#
    );
}

/// Escape ANSI sequences for readable output
fn escape_ansi(bytes: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            // ESC - start of escape sequence
            result.push_str("\\e");
            i += 1;
        } else if b == 0x07 {
            // BEL
            result.push_str("\\a");
            i += 1;
        } else if b == 0x08 {
            // BS
            result.push_str("\\b");
            i += 1;
        } else if b == 0x09 {
            // TAB
            result.push('\t');
            i += 1;
        } else if b == 0x0a {
            // LF
            result.push('\n');
            i += 1;
        } else if b == 0x0d {
            // CR
            result.push_str("\\r");
            i += 1;
        } else if b < 0x20 {
            // Other control chars
            result.push_str(&format!("\\x{:02x}", b));
            i += 1;
        } else if b < 0x7f {
            // Printable ASCII
            result.push(b as char);
            i += 1;
        } else {
            // UTF-8 or high bytes - try to decode as UTF-8
            let remaining = &bytes[i..];
            if let Some((ch, len)) = decode_utf8_char(remaining) {
                result.push(ch);
                i += len;
            } else {
                result.push_str(&format!("\\x{:02x}", b));
                i += 1;
            }
        }
    }

    result
}

/// Try to decode a single UTF-8 character, return char and byte length
fn decode_utf8_char(bytes: &[u8]) -> Option<(char, usize)> {
    if bytes.is_empty() {
        return None;
    }

    let len = match bytes[0] {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => return None,
    };

    if bytes.len() < len {
        return None;
    }

    std::str::from_utf8(&bytes[..len])
        .ok()
        .and_then(|s| s.chars().next())
        .map(|c| (c, len))
}

/// Apply offset and limit to lines
fn slice_lines(text: &str, offset: usize, limit: Option<usize>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = offset.min(lines.len());
    let end = match limit {
        Some(n) => (start + n).min(lines.len()),
        None => lines.len(),
    };

    lines[start..end].join("\n")
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

    let mut client =
        SshTestClient::connect(&args.addr, args.key.as_deref(), &args.username).await?;

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
    let timeout = Duration::from_millis(args.timeout_ms);
    let output = if let Some(pattern) = &args.wait_for {
        eprintln!("waiting for '{}'...", pattern);
        client.wait_for_pattern(pattern, timeout).await?
    } else {
        client.wait_and_collect(timeout).await?
    };

    eprintln!("--- output ({} bytes) ---", output.len());

    if args.raw {
        // Hex dump
        for (i, chunk) in output.chunks(16).enumerate() {
            print!("{:04x}: ", i * 16);
            for byte in chunk {
                print!("{:02x} ", byte);
            }
            // Pad if short row
            for _ in chunk.len()..16 {
                print!("   ");
            }
            print!(" ");
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
        // Escaped output (readable ANSI codes)
        let escaped = escape_ansi(&output);
        let sliced = slice_lines(&escaped, args.offset, args.limit);
        print!("{}", sliced);
        if !sliced.ends_with('\n') {
            println!();
        }
    }

    eprintln!("---");

    client.close().await?;
    eprintln!("disconnected");

    Ok(())
}
