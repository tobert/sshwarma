#!/bin/bash
# Test chat rendering - send messages and verify they appear
set -e

SSHTEST="${SSHTEST:-./target/debug/sshtest}"

echo "==> Sending messages and checking for chat content..."

# Join room and send messages, wait for each to appear
OUTPUT=$($SSHTEST \
    --cmd "/join test" --wait-for "test>" \
    --cmd "message one" --wait-for "message one" \
    --cmd "message two" --wait-for "message two" \
    --cmd "message three" --wait-for "message three" \
    2>&1)

echo "$OUTPUT"

echo ""
echo "==> Checking for message content in output..."
if echo "$OUTPUT" | grep -q "message one" && \
   echo "$OUTPUT" | grep -q "message two" && \
   echo "$OUTPUT" | grep -q "message three"; then
    echo "SUCCESS: All messages found in output"
else
    echo "FAIL: Some messages missing"
    exit 1
fi
