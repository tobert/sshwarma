# SSH Test Client for sshwarma

## Goal

Build a simple SSH client tool using `russh` that can:
1. Connect to sshwarma on localhost:2222
2. Send commands (like `/join test`, `/rooms`, plain text)
3. Capture screen output/frames
4. Report results via MCP or stdout

## Why

We're iterating on the Lua UI and command dispatch. Manual SSH testing is slow. An automated client lets us:
- Run quick smoke tests after each change
- Capture exact ANSI output for debugging
- Eventually expand into full integration tests

## Approach

### Option A: Standalone binary (preferred)
```
cargo run --bin sshtest -- --cmd "/join test" --expect "Joined room"
```

### Option B: MCP tool
Add `ssh_test` tool to sshwarma's MCP server that spawns a client connection.

## Key Features

1. **Connect** with SSH key from `~/.ssh/id_ed25519` or similar
2. **Send input** - commands, text, escape sequences
3. **Capture frames** - collect raw ANSI output, optionally strip escapes
4. **Wait for patterns** - block until output matches regex
5. **Timeout** - fail if expected output doesn't appear

## Example Usage

```rust
let client = SshTestClient::connect("localhost:2222").await?;
client.send("/join test\n").await?;
let frame = client.wait_for("\\[test\\]", Duration::from_secs(2)).await?;
assert!(frame.contains("[test]"));
```

## Implementation Notes

- Use `russh` crate (already in workspace)
- PTY size: 80x24 default
- Alternate screen buffer: sshwarma uses `\x1b[?1049h`
- Parse/strip ANSI if needed for assertions

## Files to Create

- `src/bin/sshtest.rs` - CLI wrapper
- `src/testing/ssh_client.rs` - reusable client
- `src/testing/mod.rs` - module exports

## Reference

- sshwarma SSH handler: `src/ssh/handler.rs`
- russh client examples: https://docs.rs/russh/latest/russh/client/
