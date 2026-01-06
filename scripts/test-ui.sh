#!/bin/bash
# Basic UI smoke tests
set -e

SSHTEST="${SSHTEST:-./target/debug/sshtest}"

echo "==> Test 1: Initial screen (lobby)"
$SSHTEST --cmd "/look" --wait-for "lobby"

echo ""
echo "==> Test 2: Join room"
$SSHTEST --cmd "/join test" --wait-for "test>"

echo ""
echo "==> Test 3: Send message"
$SSHTEST --cmd "/join test" --wait-for "test>" --cmd "hello from sshtest"
