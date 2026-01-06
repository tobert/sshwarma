#!/bin/bash
# Test chat rendering specifically - send multiple messages and check output
set -e

SSHTEST="./target/debug/sshtest"

echo "==> Sending messages and checking for chat content..."

# Join room and send a few messages, longer wait to see if chat appears
OUTPUT=$($SSHTEST --wait 3000 \
    --cmd "/join test" \
    --cmd "message one" \
    --cmd "message two" \
    --cmd "message three" \
    2>&1)

echo "$OUTPUT"

echo ""
echo "==> Checking for message content in output..."
if echo "$OUTPUT" | grep -a "message"; then
    echo "SUCCESS: Found message text in output"
else
    echo "FAIL: No message text found - chat not rendering"
fi

echo ""
echo "==> Checking which rows have content..."
# Look for row positioning escapes [row;colH
echo "$OUTPUT" | grep -oE '\[([0-9]+);[0-9]+H' | sort -u | head -20
