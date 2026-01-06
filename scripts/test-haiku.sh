#!/bin/bash
# Test chat rendering with a haiku
set -e

SSHTEST="${SSHTEST:-./target/debug/sshtest}"

echo "==> Sending haiku to chat..."
$SSHTEST \
    --cmd "/join test" --wait-for "test>" \
    --cmd "Terminal glows bright" --wait-for "Terminal glows bright" \
    --cmd "Messages flow through the void" --wait-for "Messages flow" \
    --cmd "Chat works at last!" --wait-for "Chat works"

echo ""
echo "==> Verifying haiku is visible..."
OUTPUT=$($SSHTEST --cmd "/join test" --wait-for "test>" --timeout 2000 2>&1)
if echo "$OUTPUT" | grep -q "Terminal\|Messages\|Chat works"; then
    echo "SUCCESS: Found haiku text in chat!"
else
    echo "FAIL: Haiku not visible in chat output"
    exit 1
fi
