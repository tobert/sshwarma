#!/bin/bash
# Test input handling by sending various text patterns
set -e

SSHTEST="${SSHTEST:-./target/debug/sshtest}"

echo "==> Test 1: Basic text input"
$SSHTEST --cmd "/join test" --wait-for "test>" \
    --cmd "hello world" --wait-for "hello world"

echo ""
echo "==> Test 2: Unicode input"
$SSHTEST --cmd "/join test" --wait-for "test>" \
    --cmd "emoji: 🎵🎹🎸" --wait-for "🎵"

echo ""
echo "==> Test 3: Long line input"
$SSHTEST --cmd "/join test" --wait-for "test>" \
    --cmd "This is a longer message that tests how the input buffer handles text that spans a reasonable length" \
    --wait-for "longer message"

echo ""
echo "==> Test 4: Special characters"
$SSHTEST --cmd "/join test" --wait-for "test>" \
    --cmd "special: /path/to/file.txt (with parens) [and brackets]" \
    --wait-for "/path/to/file.txt"

echo ""
echo "NOTE: To test backspace/arrow keys, SSH in manually and verify:"
echo "  1. Type 'abc', press left arrow twice, type 'X' -> should get 'aXbc'"
echo "  2. Type 'abc', press backspace -> should get 'ab'"
echo "  3. Type text, press Home, type 'Y' -> should get 'Y' at start"
