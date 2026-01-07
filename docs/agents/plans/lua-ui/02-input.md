# 02: Raw Byte Input Handling

**File:** `src/ssh/handler.rs`, `src/embedded/ui/input.lua`
**Focus:** Route raw SSH bytes to Lua, parse in Lua
**Dependencies:** None
**Unblocks:** 07-streaming (needs input system for @mentions)

---

## Task

Make Lua receive raw bytes from SSH channel and handle all input parsing.

**Why this first?** Input handling is foundational. Commands, chat, hotkeys all need it.

**Deliverables:**
1. Rust passes raw bytes to `lua.on_input(bytes)`
2. Lua parses escape sequences (arrows, function keys, etc.)
3. Input buffer management in Lua
4. Cursor movement, backspace, delete work
5. Enter submits input, calls command dispatch
6. Ctrl+C, Ctrl+D, Escape handled

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- Tab completion — that's enhancement after basic input works
- Command dispatch — that's 05-commands
- History (up/down through previous commands) — future enhancement

Focus ONLY on receiving bytes, parsing keys, managing input buffer.

---

## Rust Changes

```rust
// In ssh/handler.rs - simplify to just forward bytes

impl Handler for SshHandler {
    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        // Forward raw bytes to Lua
        if let Some(ref lua_runtime) = self.lua_runtime {
            let lua = lua_runtime.lock().await;
            if let Err(e) = lua.call_on_input(data) {
                tracing::error!("Lua input error: {}", e);
            }
        }
        Ok(())
    }
}
```

```rust
// In lua/mod.rs - add on_input entry point

impl LuaRuntime {
    pub fn call_on_input(&self, bytes: &[u8]) -> Result<()> {
        let on_input: Function = self.lua.globals().get("on_input")?;
        on_input.call::<()>(bytes.to_vec())?;
        Ok(())
    }
}
```

---

## Escape Sequence Parsing

Common terminal escape sequences:

```lua
local ESC = "\x1b"

local SEQUENCES = {
    [ESC .. "[A"] = { type = "arrow", dir = "up" },
    [ESC .. "[B"] = { type = "arrow", dir = "down" },
    [ESC .. "[C"] = { type = "arrow", dir = "right" },
    [ESC .. "[D"] = { type = "arrow", dir = "left" },
    [ESC .. "[H"] = { type = "home" },
    [ESC .. "[F"] = { type = "end" },
    [ESC .. "[3~"] = { type = "delete" },
    [ESC .. "[5~"] = { type = "pageup" },
    [ESC .. "[6~"] = { type = "pagedown" },
    [ESC .. "OP"] = { type = "f1" },
    [ESC .. "OQ"] = { type = "f2" },
    -- etc.
}

-- Control characters
local CTRL = {
    ["\x01"] = { type = "ctrl", char = "a" },  -- Ctrl+A (home)
    ["\x02"] = { type = "ctrl", char = "b" },  -- Ctrl+B (back)
    ["\x03"] = { type = "ctrl", char = "c" },  -- Ctrl+C (interrupt)
    ["\x04"] = { type = "ctrl", char = "d" },  -- Ctrl+D (EOF)
    ["\x05"] = { type = "ctrl", char = "e" },  -- Ctrl+E (end)
    ["\x06"] = { type = "ctrl", char = "f" },  -- Ctrl+F (forward)
    ["\x0b"] = { type = "ctrl", char = "k" },  -- Ctrl+K (kill to end)
    ["\x0c"] = { type = "ctrl", char = "l" },  -- Ctrl+L (clear)
    ["\x15"] = { type = "ctrl", char = "u" },  -- Ctrl+U (kill line)
    ["\x17"] = { type = "ctrl", char = "w" },  -- Ctrl+W (kill word)
    ["\x7f"] = { type = "backspace" },
    ["\x1b"] = { type = "escape" },
    ["\r"] = { type = "enter" },
    ["\n"] = { type = "enter" },
    ["\t"] = { type = "tab" },
}
```

---

## Types (Lua)

```lua
-- ui/input.lua

local M = {}

---@class InputState
---@field text string Current input text
---@field cursor number Cursor position (byte offset)
---@field prompt string Prompt string

---@class ParsedKey
---@field type string "char" | "arrow" | "ctrl" | "enter" | "backspace" | etc.
---@field char? string For type="char" or type="ctrl"
---@field dir? string For type="arrow"

local state = {
    text = "",
    cursor = 0,
    prompt = "> ",
}

--- Parse raw bytes into key events
---@param bytes string Raw bytes from SSH
---@return ParsedKey[]
function M.parse(bytes)
    local keys = {}
    local i = 1
    while i <= #bytes do
        -- Check for escape sequence
        if bytes:sub(i, i) == "\x1b" then
            local found = false
            for seq, key in pairs(SEQUENCES) do
                if bytes:sub(i, i + #seq - 1) == seq then
                    table.insert(keys, key)
                    i = i + #seq
                    found = true
                    break
                end
            end
            if not found then
                -- Just escape key
                table.insert(keys, { type = "escape" })
                i = i + 1
            end
        elseif CTRL[bytes:sub(i, i)] then
            table.insert(keys, CTRL[bytes:sub(i, i)])
            i = i + 1
        else
            -- Regular character (may be UTF-8 multi-byte)
            local char, len = M.next_utf8_char(bytes, i)
            table.insert(keys, { type = "char", char = char })
            i = i + len
        end
    end
    return keys
end

--- Get next UTF-8 character and its byte length
---@param s string
---@param i number Starting byte position
---@return string char, number len
function M.next_utf8_char(s, i)
    local b = s:byte(i)
    if not b then return "", 0 end
    local len = 1
    if b >= 0xF0 then len = 4
    elseif b >= 0xE0 then len = 3
    elseif b >= 0xC0 then len = 2
    end
    return s:sub(i, i + len - 1), len
end

return M
```

---

## Input Buffer Operations

```lua
-- Insert character at cursor
function M.insert(char)
    state.text = state.text:sub(1, state.cursor) .. char .. state.text:sub(state.cursor + 1)
    state.cursor = state.cursor + #char
    tools.mark_dirty('input')
end

-- Delete character before cursor (backspace)
function M.backspace()
    if state.cursor > 0 then
        -- Find start of previous UTF-8 char
        local prev_start = M.prev_utf8_start(state.text, state.cursor)
        state.text = state.text:sub(1, prev_start - 1) .. state.text:sub(state.cursor + 1)
        state.cursor = prev_start - 1
        tools.mark_dirty('input')
    end
end

-- Delete character at cursor (delete key)
function M.delete()
    if state.cursor < #state.text then
        local _, char_len = M.next_utf8_char(state.text, state.cursor + 1)
        state.text = state.text:sub(1, state.cursor) .. state.text:sub(state.cursor + 1 + char_len)
        tools.mark_dirty('input')
    end
end

-- Move cursor left
function M.left()
    if state.cursor > 0 then
        state.cursor = M.prev_utf8_start(state.text, state.cursor) - 1
        tools.mark_dirty('input')
    end
end

-- Move cursor right
function M.right()
    if state.cursor < #state.text then
        local _, char_len = M.next_utf8_char(state.text, state.cursor + 1)
        state.cursor = state.cursor + char_len
        tools.mark_dirty('input')
    end
end

-- Move to start
function M.home()
    state.cursor = 0
    tools.mark_dirty('input')
end

-- Move to end
function M.end_of_line()
    state.cursor = #state.text
    tools.mark_dirty('input')
end

-- Clear line
function M.clear()
    state.text = ""
    state.cursor = 0
    tools.mark_dirty('input')
end

-- Submit input (enter pressed)
function M.submit()
    local text = state.text
    M.clear()
    return text
end
```

---

## Global Entry Point

```lua
-- In screen.lua or init.lua

local input = require 'ui.input'

function on_input(bytes)
    local keys = input.parse(bytes)

    for _, key in ipairs(keys) do
        if key.type == "char" then
            input.insert(key.char)
        elseif key.type == "backspace" then
            input.backspace()
        elseif key.type == "delete" then
            input.delete()
        elseif key.type == "arrow" then
            if key.dir == "left" then input.left()
            elseif key.dir == "right" then input.right()
            elseif key.dir == "up" then input.history_prev()
            elseif key.dir == "down" then input.history_next()
            end
        elseif key.type == "home" then
            input.home()
        elseif key.type == "end" then
            input.end_of_line()
        elseif key.type == "enter" then
            local text = input.submit()
            if #text > 0 then
                commands.dispatch(text)
            end
        elseif key.type == "ctrl" then
            if key.char == "c" then
                input.clear()
            elseif key.char == "u" then
                input.clear()
            elseif key.char == "a" then
                input.home()
            elseif key.char == "e" then
                input.end_of_line()
            elseif key.char == "l" then
                tools.mark_dirty('chat', 'status', 'input')
            end
        elseif key.type == "escape" then
            -- Close overlay/popup if open
            regions.hide_top()
        elseif key.type == "pageup" then
            scroll.page_up()
        elseif key.type == "pagedown" then
            scroll.page_down()
        end
    end
end
```

---

## Acceptance Criteria

- [ ] Typing characters appears in input buffer
- [ ] Backspace deletes character before cursor
- [ ] Delete key deletes character at cursor
- [ ] Arrow keys move cursor
- [ ] Home/End move to start/end of line
- [ ] Ctrl+C clears input
- [ ] Ctrl+U clears input
- [ ] Enter submits and clears input
- [ ] UTF-8 characters work (emoji, CJK, etc.)
- [ ] Escape closes overlay (if any)
- [ ] PageUp/PageDown scroll chat
- [ ] Ctrl+L triggers full redraw
- [ ] Input state available via `tools.input()` for rendering
