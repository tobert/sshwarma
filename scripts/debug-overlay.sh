#!/bin/bash
set -e

# Use release binary (systemd runs release)
SSHTEST="./target/release/sshtest"

echo "==> Connecting and capturing initial screen (raw)..."
$SSHTEST --wait 1500 --raw --cmd "/look" 2>&1 | tee /tmp/sshwarma-initial.txt

echo ""
echo "==> Looking for overlay indicators..."
if grep -a "show_region\|overlay" /tmp/sshwarma-initial.txt; then
    echo "Found overlay-related content"
else
    echo "No overlay text found in raw output"
fi
