#!/bin/bash
# Test input handling by sending various text patterns
set -e

# Use release binary (systemd runs release)
SSHTEST="./target/release/sshtest"

echo "==> Test 1: Basic text input"
$SSHTEST --wait 1000 \
    --cmd "/join test" \
    --cmd "hello world"

echo ""
echo "==> Test 2: Unicode input"
$SSHTEST --wait 1000 \
    --cmd "/join test" \
    --cmd "emoji: 🎵🎹🎸"

echo ""
echo "==> Test 3: Long line input"
$SSHTEST --wait 1000 \
    --cmd "/join test" \
    --cmd "This is a longer message that tests how the input buffer handles text that spans a reasonable length"

echo ""
echo "==> Test 4: Special characters"
$SSHTEST --wait 1000 \
    --cmd "/join test" \
    --cmd "special: /path/to/file.txt (with parens) [and brackets] {and braces}"

echo ""
echo "NOTE: To test backspace/arrow keys, SSH in manually and verify:"
echo "  1. Type 'abc', press left arrow twice, type 'X' -> should get 'aXbc'"
echo "  2. Type 'abc', press backspace -> should get 'ab'"
echo "  3. Type text, press Home, type 'Y' -> should get 'Y' at start"
