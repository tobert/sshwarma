-- sshwarma input handler
--
-- Handles raw byte input from SSH, parses escape sequences,
-- and manages the input buffer with cursor movement.
--
-- Module structure:
--   M.parse(bytes) -> ParsedKey[]
--   M.next_utf8_char(s, i) -> char, len
--   M.prev_utf8_start(s, i) -> pos
--   M.insert(char) -> nil
--   M.backspace() -> nil
--   M.delete() -> nil
--   M.left() -> nil
--   M.right() -> nil
--   M.home() -> nil
--   M.end_of_line() -> nil
--   M.clear() -> nil
--   M.submit() -> string
--   M.handle_key(key) -> action|nil
--
-- Entry point (called by Rust):
--   on_input(bytes) -> action|nil

local M = {}

--------------------------------------------------------------------------------
-- Escape Sequence Tables
--------------------------------------------------------------------------------

local ESC = "\x1b"

-- CSI sequences: ESC [ <sequence>
local CSI_SEQUENCES = {
    ["A"] = { type = "arrow", dir = "up" },
    ["B"] = { type = "arrow", dir = "down" },
    ["C"] = { type = "arrow", dir = "right" },
    ["D"] = { type = "arrow", dir = "left" },
    ["H"] = { type = "home" },
    ["F"] = { type = "end" },
    ["1~"] = { type = "home" },
    ["3~"] = { type = "delete" },
    ["4~"] = { type = "end" },
    ["5~"] = { type = "pageup" },
    ["6~"] = { type = "pagedown" },
    ["7~"] = { type = "home" },  -- rxvt
    ["8~"] = { type = "end" },   -- rxvt
}

-- SS3 sequences: ESC O <char> (some terminals use these for arrows/F-keys)
local SS3_SEQUENCES = {
    ["A"] = { type = "arrow", dir = "up" },
    ["B"] = { type = "arrow", dir = "down" },
    ["C"] = { type = "arrow", dir = "right" },
    ["D"] = { type = "arrow", dir = "left" },
    ["H"] = { type = "home" },
    ["F"] = { type = "end" },
    ["P"] = { type = "f1" },
    ["Q"] = { type = "f2" },
    ["R"] = { type = "f3" },
    ["S"] = { type = "f4" },
}

-- Control characters (single-byte)
local CTRL = {
    ["\x01"] = { type = "ctrl", char = "a" },  -- Ctrl+A (home)
    ["\x02"] = { type = "ctrl", char = "b" },  -- Ctrl+B (back char)
    ["\x03"] = { type = "ctrl", char = "c" },  -- Ctrl+C (interrupt)
    ["\x04"] = { type = "ctrl", char = "d" },  -- Ctrl+D (EOF)
    ["\x05"] = { type = "ctrl", char = "e" },  -- Ctrl+E (end)
    ["\x06"] = { type = "ctrl", char = "f" },  -- Ctrl+F (forward char)
    ["\x0b"] = { type = "ctrl", char = "k" },  -- Ctrl+K (kill to end)
    ["\x0c"] = { type = "ctrl", char = "l" },  -- Ctrl+L (clear)
    ["\x15"] = { type = "ctrl", char = "u" },  -- Ctrl+U (kill line)
    ["\x17"] = { type = "ctrl", char = "w" },  -- Ctrl+W (kill word)
    ["\x7f"] = { type = "backspace" },
    ["\x08"] = { type = "backspace" },
    ["\r"] = { type = "enter" },
    ["\n"] = { type = "enter" },
    ["\t"] = { type = "tab" },
}

--------------------------------------------------------------------------------
-- Input State
--------------------------------------------------------------------------------

---@class InputState
---@field text string Current input text
---@field cursor number Cursor position (byte offset)
---@field prompt string Prompt string (for display)
---@field history string[] Command history
---@field history_pos number|nil Current position in history (nil = editing new input)
---@field saved_input string Saved input when navigating history
---@field kill_ring string Last killed text for yanking

local state = {
    text = "",
    cursor = 0,
    prompt = "> ",
    history = {},
    history_pos = nil,
    saved_input = "",
    kill_ring = "",
}

--- Get current state (for external access)
--- @return InputState
function M.get_state()
    return state
end

--- Set the prompt
--- @param prompt string
function M.set_prompt(prompt)
    state.prompt = prompt
end

--------------------------------------------------------------------------------
-- UTF-8 Handling
--------------------------------------------------------------------------------

--- Get next UTF-8 character and its byte length
--- @param s string
--- @param i number Starting byte position (1-indexed)
--- @return string char, number len
function M.next_utf8_char(s, i)
    if i > #s then return "", 0 end
    local b = s:byte(i)
    if not b then return "", 0 end

    local len = 1
    if b >= 0xF0 then
        len = 4
    elseif b >= 0xE0 then
        len = 3
    elseif b >= 0xC0 then
        len = 2
    end

    -- Clamp to available bytes
    if i + len - 1 > #s then
        len = #s - i + 1
    end

    return s:sub(i, i + len - 1), len
end

--- Find the start byte position of the previous UTF-8 character
--- @param s string
--- @param i number Current byte position (1-indexed, points after current char)
--- @return number pos Start of previous character (1-indexed)
function M.prev_utf8_start(s, i)
    if i <= 1 then return 1 end

    -- Move back and find a valid UTF-8 start byte
    local pos = i - 1
    while pos >= 1 do
        local b = s:byte(pos)
        -- UTF-8 continuation bytes are 10xxxxxx (0x80-0xBF)
        -- Start bytes are 0xxxxxxx, 110xxxxx, 1110xxxx, 11110xxx
        if b < 0x80 or b >= 0xC0 then
            return pos
        end
        pos = pos - 1
    end
    return 1
end

--- Count UTF-8 characters in a string (for display)
--- @param s string
--- @return number
function M.char_count(s)
    if not s or s == "" then return 0 end
    local count = 0
    local i = 1
    while i <= #s do
        local _, len = M.next_utf8_char(s, i)
        if len == 0 then break end
        count = count + 1
        i = i + len
    end
    return count
end

--------------------------------------------------------------------------------
-- Escape Sequence Parsing
--------------------------------------------------------------------------------

---@class ParsedKey
---@field type string "char"|"arrow"|"ctrl"|"enter"|"backspace"|"delete"|"home"|"end"|"tab"|"escape"|"pageup"|"pagedown"|"f1".."f12"|"unknown"
---@field char? string For type="char" or type="ctrl"
---@field dir? string For type="arrow": "up"|"down"|"left"|"right"

--- Parse raw bytes into key events
--- @param bytes string Raw bytes from SSH
--- @return ParsedKey[]
function M.parse(bytes)
    local keys = {}
    local i = 1

    while i <= #bytes do
        local b = bytes:byte(i)

        -- Check for ESC
        if b == 0x1b then
            -- Look ahead for sequence
            if i + 1 <= #bytes then
                local next_byte = bytes:byte(i + 1)

                if next_byte == 0x5b then  -- '['
                    -- CSI sequence: ESC [ params final
                    local seq_start = i + 2
                    local params = ""
                    local j = seq_start

                    -- Collect parameter bytes (0-9, ;, :)
                    while j <= #bytes do
                        local c = bytes:byte(j)
                        if c >= 0x30 and c <= 0x3f then  -- 0-? range
                            params = params .. string.char(c)
                            j = j + 1
                        else
                            break
                        end
                    end

                    -- Get final byte
                    if j <= #bytes then
                        local final = string.char(bytes:byte(j))
                        local seq_key = params .. final

                        if CSI_SEQUENCES[seq_key] then
                            table.insert(keys, CSI_SEQUENCES[seq_key])
                        elseif CSI_SEQUENCES[final] then
                            -- Simple sequence without params
                            table.insert(keys, CSI_SEQUENCES[final])
                        else
                            table.insert(keys, { type = "unknown", sequence = "CSI " .. seq_key })
                        end
                        i = j + 1
                    else
                        -- Incomplete sequence, treat as bare escape
                        table.insert(keys, { type = "escape" })
                        i = i + 1
                    end

                elseif next_byte == 0x4f then  -- 'O'
                    -- SS3 sequence: ESC O <char>
                    if i + 2 <= #bytes then
                        local char = string.char(bytes:byte(i + 2))
                        if SS3_SEQUENCES[char] then
                            table.insert(keys, SS3_SEQUENCES[char])
                        else
                            table.insert(keys, { type = "unknown", sequence = "SS3 " .. char })
                        end
                        i = i + 3
                    else
                        -- Incomplete, treat as bare escape
                        table.insert(keys, { type = "escape" })
                        i = i + 1
                    end

                else
                    -- Unknown sequence after ESC, treat as bare escape
                    table.insert(keys, { type = "escape" })
                    i = i + 1
                end
            else
                -- Bare ESC at end of input
                table.insert(keys, { type = "escape" })
                i = i + 1
            end

        -- Check control characters
        elseif CTRL[string.char(b)] then
            table.insert(keys, CTRL[string.char(b)])
            i = i + 1

        -- Printable ASCII or UTF-8
        elseif b >= 0x20 then
            local char, len = M.next_utf8_char(bytes, i)
            if len > 0 then
                table.insert(keys, { type = "char", char = char })
                i = i + len
            else
                i = i + 1  -- Skip invalid byte
            end

        else
            -- Unknown control byte
            table.insert(keys, { type = "unknown", byte = b })
            i = i + 1
        end
    end

    return keys
end

--------------------------------------------------------------------------------
-- Input Buffer Operations
--------------------------------------------------------------------------------

--- Sync Lua input state to Rust state
--- Called after each buffer modification so tools.input() returns current state
local function sync_state()
    if tools and tools.set_input then
        tools.set_input(state.text, state.cursor, state.prompt)
    end
end

--- Insert character at cursor
--- @param char string Character to insert (may be multi-byte UTF-8)
function M.insert(char)
    state.text = state.text:sub(1, state.cursor) .. char .. state.text:sub(state.cursor + 1)
    state.cursor = state.cursor + #char
    state.history_pos = nil  -- Cancel history navigation
    sync_state()
end

--- Delete character before cursor (backspace)
function M.backspace()
    if state.cursor > 0 then
        -- prev_utf8_start expects 1-indexed position, cursor is 0-indexed byte offset
        local prev_start = M.prev_utf8_start(state.text, state.cursor + 1)
        state.text = state.text:sub(1, prev_start - 1) .. state.text:sub(state.cursor + 1)
        state.cursor = prev_start - 1
        state.history_pos = nil
        sync_state()
    end
end

--- Delete character at cursor (delete key)
function M.delete()
    if state.cursor < #state.text then
        local _, char_len = M.next_utf8_char(state.text, state.cursor + 1)
        state.text = state.text:sub(1, state.cursor) .. state.text:sub(state.cursor + 1 + char_len)
        sync_state()
    end
end

--- Move cursor left one character
function M.left()
    if state.cursor > 0 then
        -- prev_utf8_start expects 1-indexed position, cursor is 0-indexed byte offset
        state.cursor = M.prev_utf8_start(state.text, state.cursor + 1) - 1
        sync_state()
    end
end

--- Move cursor right one character
function M.right()
    if state.cursor < #state.text then
        local _, char_len = M.next_utf8_char(state.text, state.cursor + 1)
        state.cursor = state.cursor + char_len
        sync_state()
    end
end

--- Move cursor to start of line
function M.home()
    state.cursor = 0
    sync_state()
end

--- Move cursor to end of line
function M.end_of_line()
    state.cursor = #state.text
    sync_state()
end

--- Clear input line
function M.clear()
    state.text = ""
    state.cursor = 0
    state.history_pos = nil
    state.saved_input = ""
    sync_state()
end

--- Submit input (enter pressed)
--- @return string The submitted text
function M.submit()
    local text = state.text

    -- Add to history (skip empty and duplicates at end)
    if #text > 0 then
        if #state.history == 0 or state.history[#state.history] ~= text then
            table.insert(state.history, text)
            -- Limit history size
            while #state.history > 500 do
                table.remove(state.history, 1)
            end
        end
    end

    M.clear()
    return text
end

--- Kill from cursor to end of line (Ctrl+K)
function M.kill_to_end()
    if state.cursor < #state.text then
        state.kill_ring = state.text:sub(state.cursor + 1)
        state.text = state.text:sub(1, state.cursor)
        sync_state()
    end
end

--- Kill from start of line to cursor (Ctrl+U)
function M.kill_to_start()
    if state.cursor > 0 then
        state.kill_ring = state.text:sub(1, state.cursor)
        state.text = state.text:sub(state.cursor + 1)
        state.cursor = 0
        sync_state()
    end
end

--- Kill word backward (Ctrl+W)
function M.kill_word_back()
    if state.cursor == 0 then return end

    local text = state.text
    local pos = state.cursor

    -- Skip trailing spaces
    while pos > 0 and text:sub(pos, pos) == " " do
        pos = pos - 1
    end

    -- Find start of word
    while pos > 0 and text:sub(pos, pos) ~= " " do
        pos = pos - 1
    end

    -- Kill from pos to cursor
    state.kill_ring = text:sub(pos + 1, state.cursor)
    state.text = text:sub(1, pos) .. text:sub(state.cursor + 1)
    state.cursor = pos
    state.history_pos = nil
    sync_state()
end

--- Navigate to previous history entry (Up arrow)
function M.history_prev()
    if #state.history == 0 then return end

    if state.history_pos == nil then
        -- Starting navigation, save current input
        state.saved_input = state.text
        state.history_pos = #state.history
    elseif state.history_pos > 1 then
        state.history_pos = state.history_pos - 1
    else
        return  -- Already at oldest
    end

    state.text = state.history[state.history_pos] or ""
    state.cursor = #state.text
    sync_state()
end

--- Navigate to next history entry (Down arrow)
function M.history_next()
    if state.history_pos == nil then return end

    if state.history_pos >= #state.history then
        -- Return to saved input
        state.text = state.saved_input
        state.cursor = #state.text
        state.history_pos = nil
        state.saved_input = ""
    else
        state.history_pos = state.history_pos + 1
        state.text = state.history[state.history_pos] or ""
        state.cursor = #state.text
    end
    sync_state()
end

--------------------------------------------------------------------------------
-- Key Handler (maps parsed keys to actions)
--------------------------------------------------------------------------------

--- Action returned by key handler
---@alias InputAction
---| {type: "none"}
---| {type: "redraw"}
---| {type: "execute", text: string}
---| {type: "tab"}
---| {type: "clear_screen"}
---| {type: "quit"}
---| {type: "escape"}
---| {type: "page_up"}
---| {type: "page_down"}

--- Handle a single parsed key event
--- @param key ParsedKey
--- @return InputAction|nil
function M.handle_key(key)
    if key.type == "char" then
        M.insert(key.char)
        return { type = "redraw" }

    elseif key.type == "backspace" then
        M.backspace()
        return { type = "redraw" }

    elseif key.type == "delete" then
        M.delete()
        return { type = "redraw" }

    elseif key.type == "arrow" then
        if key.dir == "left" then
            M.left()
            return { type = "redraw" }
        elseif key.dir == "right" then
            M.right()
            return { type = "redraw" }
        elseif key.dir == "up" then
            M.history_prev()
            return { type = "redraw" }
        elseif key.dir == "down" then
            M.history_next()
            return { type = "redraw" }
        end

    elseif key.type == "home" then
        M.home()
        return { type = "redraw" }

    elseif key.type == "end" then
        M.end_of_line()
        return { type = "redraw" }

    elseif key.type == "enter" then
        local text = M.submit()
        if #text > 0 then
            return { type = "execute", text = text }
        else
            return { type = "none" }
        end

    elseif key.type == "ctrl" then
        if key.char == "c" then
            M.clear()
            return { type = "redraw" }
        elseif key.char == "u" then
            M.kill_to_start()
            return { type = "redraw" }
        elseif key.char == "k" then
            M.kill_to_end()
            return { type = "redraw" }
        elseif key.char == "w" then
            M.kill_word_back()
            return { type = "redraw" }
        elseif key.char == "a" then
            M.home()
            return { type = "redraw" }
        elseif key.char == "e" then
            M.end_of_line()
            return { type = "redraw" }
        elseif key.char == "b" then
            M.left()
            return { type = "redraw" }
        elseif key.char == "f" then
            M.right()
            return { type = "redraw" }
        elseif key.char == "l" then
            return { type = "clear_screen" }
        elseif key.char == "d" then
            if #state.text == 0 then
                return { type = "quit" }
            else
                return { type = "none" }
            end
        end

    elseif key.type == "tab" then
        return { type = "tab" }

    elseif key.type == "escape" then
        return { type = "escape" }

    elseif key.type == "pageup" then
        return { type = "page_up" }

    elseif key.type == "pagedown" then
        return { type = "page_down" }
    end

    return nil
end

--------------------------------------------------------------------------------
-- Global Entry Point
--------------------------------------------------------------------------------

--- Main entry point for raw byte input
--- Called by Rust via LuaRuntime.call_on_input()
---
--- @param bytes string Raw bytes from SSH channel
--- @return table|nil Action to take, or nil for no action
function on_input(bytes)
    local keys = M.parse(bytes)

    for _, key in ipairs(keys) do
        local action = M.handle_key(key)
        if action and action.type ~= "none" and action.type ~= "redraw" then
            -- Non-trivial action, return it immediately
            -- (Rust will handle execute, quit, tab, etc.)
            return action
        end
    end

    -- Default: just redraw if any keys were processed
    if #keys > 0 then
        return { type = "redraw" }
    end

    return nil
end

-- Export module globally for other scripts to use
input = M
return M
