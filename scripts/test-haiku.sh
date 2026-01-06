#!/bin/bash
# Test chat rendering with a haiku
set -e

# Use release binary (systemd runs release)
SSHTEST="./target/release/sshtest"

echo "==> Sending haiku to chat..."
$SSHTEST --wait 2000 \
    --cmd "/join test" \
    --cmd "Terminal glows bright" \
    --cmd "Messages flow through the void" \
    --cmd "Chat works at last!" \
    2>&1

echo ""
echo "==> Checking if haiku text appears..."
OUTPUT=$($SSHTEST --wait 1000 --cmd "/join test" 2>&1)
if echo "$OUTPUT" | grep -a "Terminal\|Messages\|Chat works"; then
    echo "SUCCESS: Found haiku text in chat!"
else
    echo "FAIL: Haiku not visible in chat output"
    echo "Raw output (looking for content):"
    echo "$OUTPUT" | strings | grep -v "^$" | head -20
fi
