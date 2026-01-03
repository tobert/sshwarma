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
-- Word Wrap Helper
--------------------------------------------------------------------------------

-- Wrap text to fit within width, returning array of lines
local function wrap_text(text, width)
    if width <= 0 then return {""} end

    local lines = {}
    -- First, split by existing newlines
    for segment in (text .. "\n"):gmatch("([^\n]*)\n") do
        if #segment == 0 then
            table.insert(lines, "")
        elseif #segment <= width then
            table.insert(lines, segment)
        else
            -- Word wrap this segment
            local pos = 1
            while pos <= #segment do
                local chunk = segment:sub(pos, pos + width - 1)
                if #chunk < width or pos + width > #segment then
                    table.insert(lines, chunk)
                    pos = pos + #chunk
                else
                    -- Find last space to break at
                    local break_at = chunk:match(".*()%s") or width
                    if break_at < width / 2 then break_at = width end
                    -- Wrap gsub in parens to only get first return value
                    table.insert(lines, (segment:sub(pos, pos + break_at - 1):gsub("%s+$", "")))
                    pos = pos + break_at
                    -- Skip leading whitespace on next line
                    while pos <= #segment and segment:sub(pos, pos):match("%s") do
                        pos = pos + 1
                    end
                end
            end
        end
    end
    return lines
end

--------------------------------------------------------------------------------
-- Build Display Lines (message -> wrapped lines with metadata)
--------------------------------------------------------------------------------

local function build_display_lines(history, width, my_name)
    local display_lines = {}
    local prefix_width = 0

    -- Calculate max nick width for alignment
    for _, msg in ipairs(history) do
        local author = msg.author or "???"
        prefix_width = math.max(prefix_width, #author + 3) -- "<nick> "
    end

    local content_width = width - prefix_width
    if content_width < 10 then content_width = width end

    for _, msg in ipairs(history) do
        local author = msg.author or "???"
        local content = msg.content or ""
        local is_model = msg.is_model
        local is_streaming = msg.is_streaming

        -- Choose nick color
        local nick_color = C.nick
        if author == my_name then
            nick_color = C.self
        elseif is_model then
            nick_color = C.model
        elseif author == "system" then
            nick_color = C.system
        end

        -- Wrap the content
        local wrapped = wrap_text(content, content_width)

        for i, line_text in ipairs(wrapped) do
            local entry = {
                text = line_text,
                nick_color = nick_color,
                is_first_line = (i == 1),
                is_streaming = is_streaming,
            }
            if i == 1 then
                entry.author = author
                entry.prefix_width = prefix_width
            else
                entry.author = nil
                entry.prefix_width = prefix_width
            end
            table.insert(display_lines, entry)
        end
    end

    return display_lines
end

--------------------------------------------------------------------------------
-- Render Chat Buffer (with scroll)
--------------------------------------------------------------------------------

local function render_chat(ctx, chat_height)
    local history = tools.history(100) or {}  -- Get more messages for scroll

    -- Get current username for highlighting own messages
    local status = sshwarma and sshwarma.call and sshwarma.call("status", {}) or {}
    local session = status.session or {}
    local my_name = session.username or ""

    -- Build wrapped display lines
    local display_lines = build_display_lines(history, ctx.w, my_name)
    local total_lines = #display_lines

    -- Update scroll state
    local scroll = tools.scroll()
    scroll:set_content_height(total_lines)
    scroll:set_viewport_height(chat_height)

    -- Get visible range
    local start_line, end_line = scroll:visible_range()

    -- Clamp end_line to content bounds to prevent index issues
    if end_line > total_lines then
        end_line = total_lines
    end

    -- Draw visible lines
    -- Note: start_line and end_line are 0-indexed from Rust
    -- display_lines is 1-indexed (Lua tables)
    -- y is the screen row (0-indexed)
    for i = start_line, end_line - 1 do
        local line_idx = i + 1  -- Convert to 1-indexed for Lua table
        local line = display_lines[line_idx]
        if line then
            local y = i - start_line  -- Screen row
            local x = 0

            if line.is_first_line and line.author then
                -- Draw prefix: <nick>
                ctx:print(x, y, "<", {fg = C.dim})
                x = x + 1
                ctx:print(x, y, line.author, {fg = line.nick_color})
                x = x + #line.author
                ctx:print(x, y, "> ", {fg = C.dim})
                x = x + 2
            else
                -- Continuation line - indent to align
                x = line.prefix_width or 0
            end

            -- Draw content (with streaming indicator)
            local text = line.text
            if line.is_streaming and line.is_first_line then
                text = "◌ " .. text  -- Streaming indicator
            end
            ctx:print(x, y, text)
        end
    end

    -- Draw scroll indicator if not at bottom
    if not scroll.at_bottom and total_lines > chat_height then
        local pct = math.floor(scroll.percent * 100)
        local indicator = string.format("── %d%% ──", pct)
        local ix = ctx.w - #indicator
        ctx:print(ix, chat_height - 1, indicator, {fg = C.dim})
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

-- on_tick(dirty_tags, tick, ctx)
--
-- dirty_tags is a table where keys are tag names and values are true:
--   dirty_tags["status"] -> true if status region needs redraw
--   dirty_tags["chat"] -> true if chat region needs redraw
--   dirty_tags["input"] -> true if input region needs redraw
--
-- The Rust layer only redraws rows that actually changed (row diffing),
-- so even if we redraw everything here, only modified rows are sent to terminal.
-- This preserves text selection in unchanged regions.
--
-- For maximum efficiency, Lua can check dirty_tags and only render affected regions.
-- For simplicity, we render everything but let Rust diff handle the optimization.

function on_tick(dirty_tags, tick, ctx)
    ctx:clear()

    local h = ctx.h
    local w = ctx.w

    -- Layout (Lua controls this - can be changed):
    --   [0 to h-3] Chat buffer (tag: "chat")
    --   [h-2]      Status line (tag: "status")
    --   [h-1]      Input line (tag: "input")

    local chat_height = h - 2
    local status_row = h - 2
    local input_row = h - 1

    -- Render chat region
    -- Note: we always render for now; Rust's row diffing handles optimization
    if chat_height > 1 then
        render_chat(ctx, chat_height)
    end

    -- Render status region
    if status_row >= 0 then
        render_status(ctx, status_row)
    end

    -- Render input region
    if input_row >= 0 then
        render_input(ctx, input_row)
    end
end

-- Background function (called every 500ms)
-- Can use tools.mark_dirty("status") etc. to trigger redraws
function background(tick)
    -- Example: refresh status every 2 seconds for duration counter
    if tick % 4 == 0 then
        tools.mark_dirty("status")
    end
end
