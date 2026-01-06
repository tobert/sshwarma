#!/bin/bash
# Debug overlay rendering - capture raw output
set -e

SSHTEST="${SSHTEST:-./target/debug/sshtest}"

echo "==> Connecting and capturing initial screen (raw)..."
$SSHTEST --cmd "/look" --wait-for "lobby" --raw 2>&1 | tee /tmp/sshwarma-initial.txt

echo ""
echo "==> Looking for overlay indicators..."
if grep -a "show_region\|overlay" /tmp/sshwarma-initial.txt; then
    echo "Found overlay-related content"
else
    echo "No overlay text found in raw output"
fi
