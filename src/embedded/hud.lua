-- sshwarma screen renderer (irssi-style)
--
-- Full-screen layout (Lua owns everything):
--   Rows 0 to (h-3): Chat buffer
--   Row (h-2): Status line
--   Row (h-1): Input line (prompt + text + cursor)
--
-- API:
--   ctx:print(x, y, text, {fg, bg, bold, dim})
--   ctx:clear()
--   ctx.w, ctx.h - dimensions
--   tools.history(n) - get chat messages
--   tools.input() - get {text, cursor, prompt}
--   tools.mcp_connections() - get MCP server status
--   sshwarma.call("status") - get room/participant info

--------------------------------------------------------------------------------
-- Colors (simple palette)
--------------------------------------------------------------------------------

local C = {
    fg       = "#c0caf5",
    dim      = "#565f89",
    nick     = "#7aa2f7",
    model    = "#bb9af7",
    self     = "#9ece6a",
    system   = "#7dcfff",
    error    = "#f7768e",
    status   = "#1a1b26",
    statusfg = "#a9b1d6",
    cursor   = "#ff9e64",
}

--------------------------------------------------------------------------------
-- Render Chat Buffer
--------------------------------------------------------------------------------

local function render_chat(ctx, chat_height)
    local history = tools.history(chat_height) or {}

    -- Get current username for highlighting own messages
    local status = sshwarma and sshwarma.call and sshwarma.call("status", {}) or {}
    local session = status.session or {}
    local my_name = session.username or ""

    -- Calculate starting row (messages flow up from bottom of chat area)
    local msg_count = #history
    local start_row = math.max(0, chat_height - msg_count)

    for i, msg in ipairs(history) do
        local y = start_row + (i - 1)
        if y >= 0 and y < chat_height then
            local author = msg.author or "???"
            local content = msg.content or ""
            local is_model = msg.is_model

            -- Choose nick color
            local nick_color = C.nick
            if author == my_name then
                nick_color = C.self
            elseif is_model then
                nick_color = C.model
            elseif author == "system" then
                nick_color = C.system
            end

            -- Format: <nick> message
            local x = 0
            ctx:print(x, y, "<", {fg = C.dim})
            x = x + 1
            ctx:print(x, y, author, {fg = nick_color})
            x = x + #author
            ctx:print(x, y, "> ", {fg = C.dim})
            x = x + 2

            -- Truncate message to fit
            local max_len = ctx.w - x
            if #content > max_len then
                content = content:sub(1, max_len - 1) .. "…"
            end
            ctx:print(x, y, content)
        end
    end
end

--------------------------------------------------------------------------------
-- Render Status Line (irssi-style bar)
--------------------------------------------------------------------------------

local function render_status(ctx, y)
    -- Fill status bar background
    for x = 0, ctx.w - 1 do
        ctx:print(x, y, " ", {bg = C.status})
    end

    -- Get state
    local status = sshwarma and sshwarma.call and sshwarma.call("status", {}) or {}
    local room = status.room or {}
    local participants = status.participants or {}
    local session = status.session or {}

    local x = 1

    -- Room name
    local room_name = room.name or "lobby"
    ctx:print(x, y, "[", {fg = C.dim, bg = C.status})
    x = x + 1
    ctx:print(x, y, room_name, {fg = C.system, bg = C.status})
    x = x + #room_name
    ctx:print(x, y, "]", {fg = C.dim, bg = C.status})
    x = x + 2

    -- User count
    local user_count = 0
    local model_count = 0
    local active_model = nil
    for _, p in ipairs(participants) do
        if p.kind == "model" then
            model_count = model_count + 1
            if p.active then
                active_model = p.name
            end
        else
            user_count = user_count + 1
        end
    end

    local counts = string.format("%d/%d", user_count, model_count)
    ctx:print(x, y, counts, {fg = C.statusfg, bg = C.status})
    x = x + #counts + 1

    -- Active model indicator
    if active_model then
        ctx:print(x, y, "◈", {fg = C.model, bg = C.status})
        x = x + 2
        ctx:print(x, y, active_model, {fg = C.model, bg = C.status})
        x = x + #active_model + 1
    end

    -- MCP status (right side)
    local mcp = tools.mcp_connections and tools.mcp_connections() or {}
    local mcp_str = ""
    for _, conn in ipairs(mcp) do
        local indicator = conn.connected and "●" or "○"
        mcp_str = mcp_str .. indicator .. conn.name .. " "
    end
    if #mcp_str > 0 then
        local mcp_x = ctx.w - #mcp_str - 1
        if mcp_x > x then
            ctx:print(mcp_x, y, mcp_str, {fg = C.dim, bg = C.status})
        end
    end

    -- Duration (far right)
    local duration = session.duration or "0:00"
    local dur_x = ctx.w - #duration - 1
    if dur_x > x + #mcp_str then
        ctx:print(dur_x, y, duration, {fg = C.dim, bg = C.status})
    end
end

--------------------------------------------------------------------------------
-- Render Input Line
--------------------------------------------------------------------------------

local function render_input(ctx, y)
    local input = tools.input() or {}
    local prompt = input.prompt or "> "
    local text = input.text or ""
    local cursor = input.cursor or 0

    local x = 0

    -- Draw prompt
    ctx:print(x, y, prompt, {fg = C.system})
    x = x + #prompt

    -- Draw text with cursor
    if #text == 0 then
        -- Empty input, just show cursor block
        ctx:print(x, y, "▌", {fg = C.cursor})
    else
        -- Split text at cursor position
        local before = text:sub(1, cursor)
        local after = text:sub(cursor + 1)

        -- Text before cursor
        ctx:print(x, y, before)
        x = x + #before

        -- Cursor indicator (block on current char or at end)
        if #after > 0 then
            -- Cursor on a character - highlight it
            local cursor_char = after:sub(1, 1)
            ctx:print(x, y, cursor_char, {fg = C.status, bg = C.cursor})
            x = x + 1
            -- Rest of text after cursor
            ctx:print(x, y, after:sub(2))
        else
            -- Cursor at end - show block
            ctx:print(x, y, "▌", {fg = C.cursor})
        end
    end
end

--------------------------------------------------------------------------------
-- Main Entry Point
--------------------------------------------------------------------------------

function on_tick(tick, ctx)
    ctx:clear()

    local h = ctx.h
    local w = ctx.w

    -- Layout:
    --   [0 to h-3] Chat buffer
    --   [h-2]      Status line
    --   [h-1]      Input line

    local chat_height = h - 2
    local status_row = h - 2
    local input_row = h - 1

    -- Debug: show dimensions and history count at top
    local history = tools.history(50) or {}
    local status = sshwarma and sshwarma.call and sshwarma.call("status", {}) or {}
    local room = status.room or {}
    local room_name = room.name or "nil"
    local debug_str = string.format("h=%d w=%d msgs=%d tick=%d room=%s", h, w, #history, tick, room_name)
    ctx:print(0, 0, debug_str, {fg = C.dim})

    if chat_height > 1 then
        render_chat(ctx, chat_height)
    end

    if status_row >= 0 then
        render_status(ctx, status_row)
    end

    if input_row >= 0 then
        render_input(ctx, input_row)
    end
end

-- Optional background function (called every 500ms)
function background(tick)
    -- Available for polling, caching, etc.
end
