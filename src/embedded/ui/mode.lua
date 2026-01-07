-- ui/mode.lua - Vim-style mode system
--
-- Modes:
--   normal - Navigate/scroll. hjkl/arrows work on content/pages.
--   insert - Edit input buffer. Standard readline bindings.
--
-- Mode transitions:
--   normal -> insert: i (insert), / (command), @ (mention)
--   insert -> normal: Escape, Enter (submits), Ctrl+C (clears)
--
-- In normal mode, raw characters are ignored (except mode-entry chars).
-- In insert mode, everything goes to the input buffer.

local pages = require 'ui.pages'
local scroll = require 'ui.scroll'
local input = require 'ui.input'

local M = {}

-- Current mode: "normal" or "insert"
M.current = "normal"

--- Get current page name for scroll operations
local function current_page()
    return pages.current_name()
end

-- ==========================================================================
-- Normal Mode Key Map
-- ==========================================================================

local normal_keys = {
    -- Scroll (arrows + vim)
    up = function()
        scroll.up(current_page())
        return { type = "redraw" }
    end,
    down = function()
        scroll.down(current_page())
        return { type = "redraw" }
    end,
    pageup = function()
        scroll.up(current_page(), 10)
        return { type = "redraw" }
    end,
    pagedown = function()
        scroll.down(current_page(), 10)
        return { type = "redraw" }
    end,

    -- Page navigation
    left = function()
        pages.nav_left()
        return { type = "redraw" }
    end,
    right = function()
        pages.nav_right()
        return { type = "redraw" }
    end,

    -- Enter insert mode
    ["i"] = function()
        M.current = "insert"
        return { type = "redraw" }
    end,
    ["/"] = function()
        M.current = "insert"
        input.insert("/")
        return { type = "redraw" }
    end,
    ["@"] = function()
        M.current = "insert"
        input.insert("@")
        return { type = "redraw" }
    end,

    -- Jump commands
    ["g"] = function()
        scroll.to_bottom(current_page())
        return { type = "redraw" }
    end,
    ["G"] = function()
        scroll.to_top(current_page())
        return { type = "redraw" }
    end,

    -- Page operations
    ["q"] = function()
        if pages.close() then
            return { type = "redraw" }
        end
        return nil
    end,
    ["?"] = function()
        pages.open("help")
        return { type = "redraw" }
    end,

    -- Escape closes non-chat pages
    escape = function()
        if not pages.is_chat() then
            pages.close()
        end
        return { type = "redraw" }
    end,

    -- Refresh
    ["r"] = function()
        return { type = "redraw" }
    end,

    -- Ctrl+D quit
    ctrl_d = function()
        return { type = "quit" }
    end,

    -- Ctrl+L clear screen
    ctrl_l = function()
        return { type = "clear_screen" }
    end,
}

-- Add vim keys (hjkl)
normal_keys["k"] = normal_keys.up
normal_keys["j"] = normal_keys.down
normal_keys["h"] = normal_keys.left
normal_keys["l"] = normal_keys.right

-- ==========================================================================
-- Insert Mode Key Map
-- ==========================================================================

local insert_keys = {
    -- History navigation (arrows)
    up = function()
        input.history_prev()
        return { type = "redraw" }
    end,
    down = function()
        input.history_next()
        return { type = "redraw" }
    end,

    -- Cursor movement
    left = function()
        input.left()
        return { type = "redraw" }
    end,
    right = function()
        input.right()
        return { type = "redraw" }
    end,
    home = function()
        input.home()
        return { type = "redraw" }
    end,
    ["end"] = function()
        input.end_of_line()
        return { type = "redraw" }
    end,

    -- Exit insert mode
    escape = function()
        M.current = "normal"
        return { type = "redraw" }
    end,

    -- Submit and exit
    enter = function()
        local text = input.submit()
        M.current = "normal"
        if text and #text > 0 then
            return { type = "send", text = text }
        end
        return { type = "redraw" }
    end,

    -- Clear and exit
    ctrl_c = function()
        input.clear()
        M.current = "normal"
        return { type = "redraw" }
    end,

    -- Editing
    backspace = function()
        input.backspace()
        return { type = "redraw" }
    end,
    delete = function()
        input.delete()
        return { type = "redraw" }
    end,

    -- Readline bindings
    ctrl_a = function()
        input.home()
        return { type = "redraw" }
    end,
    ctrl_e = function()
        input.end_of_line()
        return { type = "redraw" }
    end,
    ctrl_u = function()
        input.kill_to_start()
        return { type = "redraw" }
    end,
    ctrl_k = function()
        input.kill_to_end()
        return { type = "redraw" }
    end,
    ctrl_w = function()
        input.kill_word_back()
        return { type = "redraw" }
    end,
    ctrl_b = function()
        input.left()
        return { type = "redraw" }
    end,
    ctrl_f = function()
        input.right()
        return { type = "redraw" }
    end,
    ctrl_l = function()
        return { type = "clear_screen" }
    end,
    ctrl_d = function()
        local state = input.get_state()
        if #state.text == 0 then
            return { type = "quit" }
        end
        return nil
    end,

    -- Tab completion
    tab = function()
        return { type = "tab" }
    end,
}

-- ==========================================================================
-- Key Handling
-- ==========================================================================

--- Convert a ParsedKey to a key name for lookup
---@param key table ParsedKey from input.parse()
---@return string|nil key name
local function key_name(key)
    if key.type == "arrow" then
        return key.dir
    elseif key.type == "ctrl" then
        return "ctrl_" .. key.char
    elseif key.type == "char" then
        return key.char
    else
        return key.type
    end
end

--- Handle a single key event
---@param key table ParsedKey from input.parse()
---@return table|nil action
function M.handle_key(key)
    local name = key_name(key)
    if not name then return nil end

    if M.current == "normal" then
        local handler = normal_keys[name]
        if handler then
            return handler()
        end
        return nil
    else
        local handler = insert_keys[name]
        if handler then
            return handler()
        end
        -- Default: insert printable characters
        if key.type == "char" and key.char then
            input.insert(key.char)
            return { type = "redraw" }
        end
        return nil
    end
end

-- ==========================================================================
-- Mode API
-- ==========================================================================

--- Check if in insert mode
---@return boolean
function M.is_insert()
    return M.current == "insert"
end

--- Check if in normal mode
---@return boolean
function M.is_normal()
    return M.current == "normal"
end

--- Get mode indicator for status bar
---@return string
function M.indicator()
    return M.current == "normal" and "N" or "I"
end

--- Force mode change
---@param mode string "normal" or "insert"
function M.set(mode)
    M.current = mode
end

--- Reset to normal mode
function M.reset()
    M.current = "normal"
end

-- ==========================================================================
-- Global Entry Point
-- ==========================================================================

--- Main entry point for raw byte input
---@param bytes string Raw bytes from SSH channel
---@return table|nil Action to take
function on_input(bytes)
    local keys = input.parse(bytes)
    local last_action = nil

    for _, key in ipairs(keys) do
        local action = M.handle_key(key)
        if action then
            last_action = action
            -- Return immediately on non-trivial actions
            if action.type ~= "redraw" then
                return action
            end
        end
    end

    return last_action
end

return M
