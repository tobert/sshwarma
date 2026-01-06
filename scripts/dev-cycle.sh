#!/bin/bash
set -e

echo "==> Building sshwarma (release)..."
cargo build --release

echo "==> Restarting sshwarma service..."
systemctl --user restart sshwarma

echo "==> Tailing logs (Ctrl+C to stop)..."
journalctl --user -u sshwarma -f --since "now"
