#!/bin/bash
# Hot reload integration tests
#
# Tests the filesystem watcher and module invalidation.
# Requires sshwarma server running on localhost:2222.
set -e

SSHTEST="${SSHTEST:-./target/debug/sshtest}"
LUA_DIR="$HOME/.config/sshwarma/lua"
TEST_MODULE="$LUA_DIR/hottest.lua"

# Setup
mkdir -p "$LUA_DIR"
trap "rm -f '$TEST_MODULE'" EXIT

echo "==> Test 1: Create module, verify available"
cat > "$TEST_MODULE" << 'EOF'
local fun = require 'fun'
local M = {}
function M.greet() return "hello from hottest v1" end
return M
EOF
sleep 0.5  # Wait for inotify
$SSHTEST --cmd "/lua require('hottest').greet()" --wait-for "hello from hottest v1"

echo ""
echo "==> Test 2: Modify module, verify reload notification"
cat > "$TEST_MODULE" << 'EOF'
local fun = require 'fun'
local M = {}
function M.greet() return "hello from hottest v2" end
return M
EOF
sleep 0.5
# Should see new version after invalidation
$SSHTEST --cmd "/lua require('hottest').greet()" --wait-for "hello from hottest v2"

echo ""
echo "==> Test 3: Touch file triggers reload"
touch "$TEST_MODULE"
sleep 0.5
$SSHTEST --cmd "/lua require('hottest').greet()" --wait-for "hottest"

echo ""
echo "==> Test 4: Syntax error shows error, doesn't fall back"
cat > "$TEST_MODULE" << 'EOF'
local M = {}
function M.greet() return "broken  -- missing end
return M
EOF
sleep 0.5
# Should see error, not silently work
$SSHTEST --cmd "/lua pcall(require, 'hottest')" --wait-for "false" --timeout 2000

echo ""
echo "==> Test 5: Fix syntax, module works again"
cat > "$TEST_MODULE" << 'EOF'
local fun = require 'fun'
local M = {}
function M.greet() return "hello from hottest v3" end
return M
EOF
sleep 0.5
$SSHTEST --cmd "/lua require('hottest').greet()" --wait-for "hello from hottest v3"

echo ""
echo "==> Test 6: Delete module, falls back to embedded/nil"
rm "$TEST_MODULE"
sleep 0.5
# Should get nil or error (no hottest in embedded)
$SSHTEST --cmd "/lua tostring(package.loaded.hottest)" --wait-for "nil"

echo ""
echo "==> All hot reload tests passed!"
