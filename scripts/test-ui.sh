#!/bin/bash
set -e

# Use release binary (systemd runs release)
SSHTEST="./target/release/sshtest"
WAIT="${1:-1000}"  # Default 1 second wait

echo "==> Test 1: Initial screen (lobby)"
$SSHTEST --wait "$WAIT" --cmd "/look"

echo ""
echo "==> Test 2: Join room"
$SSHTEST --wait "$WAIT" --cmd "/join test"

echo ""
echo "==> Test 3: Send message"
$SSHTEST --wait "$WAIT" --cmd "/join test" --cmd "hello from sshtest"
