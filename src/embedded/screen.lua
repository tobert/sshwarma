-- sshwarma screen renderer
--
-- Full-screen layout with testable components.
--
-- Module structure:
--   M.wrap_text(text, width) -> {lines}
--   M.build_display_lines(messages, width, my_name) -> {display_lines}
--   M.render_chat(ctx, lines, scroll, height) -> nil
--   M.render_status(ctx, y, data) -> nil
--   M.render_input(ctx, y, data) -> nil
--
-- Entry points (called by Rust):
--   on_tick(dirty_tags, tick, ctx)
--   background(tick)

local M = {}

--------------------------------------------------------------------------------
-- Colors
--------------------------------------------------------------------------------

M.colors = {
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
-- Pure Functions (testable without mocks)
--------------------------------------------------------------------------------

--- Get display width of a string (delegates to Rust unicode-width)
--- @param str string
--- @return number
function M.display_width(str)
    if not str or str == "" then return 0 end
    -- Use Rust's unicode-width via tools, fallback to utf8.len
    if tools and tools.display_width then
        return tools.display_width(str)
    end
    return utf8.len(str) or #str
end

--- Wrap text to fit within width, returning array of lines
--- @param text string
--- @param width number
--- @return string[]
function M.wrap_text(text, width)
    if width <= 0 then return {""} end
    if not text or text == "" then return {""} end

    local lines = {}

    -- Split by existing newlines first
    for segment in (text .. "\n"):gmatch("([^\n]*)\n") do
        if #segment == 0 then
            table.insert(lines, "")
        elseif M.display_width(segment) <= width then
            table.insert(lines, segment)
        else
            -- Word wrap this segment
            local pos = 1
            while pos <= #segment do
                -- Find how many bytes fit in width
                local end_pos = pos
                local current_width = 0

                while end_pos <= #segment do
                    local next_pos = utf8.offset(segment, 2, end_pos) or (#segment + 1)
                    local char = segment:sub(end_pos, next_pos - 1)
                    local char_width = M.display_width(char)

                    if current_width + char_width > width then
                        break
                    end

                    current_width = current_width + char_width
                    end_pos = next_pos
                end

                if end_pos == pos then
                    -- Single char wider than width, force include it
                    end_pos = utf8.offset(segment, 2, pos) or (#segment + 1)
                end

                local chunk = segment:sub(pos, end_pos - 1)

                -- Try to break at word boundary if not at end
                if end_pos <= #segment then
                    local last_space = chunk:match(".*()%s")
                    if last_space and last_space > #chunk / 2 then
                        chunk = chunk:sub(1, last_space - 1)
                        end_pos = pos + last_space
                        -- Skip whitespace
                        while end_pos <= #segment and segment:sub(end_pos, end_pos):match("%s") do
                            end_pos = end_pos + 1
                        end
                    end
                end

                table.insert(lines, (chunk:gsub("%s+$", "")))
                pos = end_pos
            end
        end
    end

    return #lines > 0 and lines or {""}
end

--- Build display lines from message history
--- @param messages table[] Array of {author, content, is_model, is_streaming}
--- @param width number Terminal width
--- @param my_name string Current user's name (for highlighting)
--- @return table[] Array of display line entries
function M.build_display_lines(messages, width, my_name)
    local display_lines = {}
    local prefix_width = 0
    local C = M.colors

    -- Calculate max nick width for alignment
    for _, msg in ipairs(messages) do
        local author = msg.author or "???"
        prefix_width = math.max(prefix_width, M.display_width(author) + 3) -- "<nick> "
    end

    -- Ensure reasonable content width
    local content_width = width - prefix_width
    if content_width < 10 then
        prefix_width = 0
        content_width = width
    end

    for _, msg in ipairs(messages) do
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
        local wrapped = M.wrap_text(content, content_width)

        for i, line_text in ipairs(wrapped) do
            local entry = {
                text = line_text,
                nick_color = nick_color,
                is_first_line = (i == 1),
                is_last_line = (i == #wrapped),
                is_streaming = is_streaming,
                prefix_width = prefix_width,
                -- Only first line gets author
                author = (i == 1) and author or nil,
            }
            table.insert(display_lines, entry)
        end
    end

    return display_lines
end

--------------------------------------------------------------------------------
-- Rendering Functions (take ctx and prepared data)
--------------------------------------------------------------------------------

--- Render chat buffer
--- @param ctx table Draw context with print(), w, h
--- @param display_lines table[] From build_display_lines
--- @param scroll table Scroll state with visible_range(), at_bottom, percent
--- @param chat_height number Height of chat region
function M.render_chat(ctx, display_lines, scroll, chat_height)
    local C = M.colors
    local total_lines = #display_lines

    -- Update scroll state
    scroll:set_content_height(total_lines)
    scroll:set_viewport_height(chat_height)

    -- Get visible range (0-indexed from Rust)
    local start_line, end_line = scroll:visible_range()

    -- Clamp to content bounds
    if end_line > total_lines then
        end_line = total_lines
    end
    if start_line < 0 then
        start_line = 0
    end

    -- Draw visible lines
    for i = start_line, end_line - 1 do
        local line_idx = i + 1  -- Convert to 1-indexed for Lua
        local line = display_lines[line_idx]

        if line then
            local y = i - start_line  -- Screen row (0-indexed)
            local x = 0

            if line.is_first_line and line.author then
                -- Draw prefix: <nick>
                ctx:print(x, y, "<", {fg = C.dim})
                x = x + 1
                ctx:print(x, y, line.author, {fg = line.nick_color})
                x = x + M.display_width(line.author)
                ctx:print(x, y, "> ", {fg = C.dim})
                x = x + 2
            else
                -- Continuation line - indent to align
                x = line.prefix_width or 0
            end

            -- Draw content
            local text = line.text
            if line.is_streaming and line.is_last_line then
                text = text .. " ◌"
            end
            ctx:print(x, y, text)
        end
    end

    -- Scroll indicator
    if not scroll.at_bottom and total_lines > chat_height then
        local pct = math.floor(scroll.percent * 100)
        local indicator = string.format("── %d%% ──", pct)
        local ix = ctx.w - M.display_width(indicator)
        ctx:print(ix, chat_height - 1, indicator, {fg = C.dim})
    end
end

--- Render status bar
--- @param ctx table Draw context
--- @param y number Row to render at
--- @param data table {room_name, user_count, model_count, active_model, mcp_connections, duration}
function M.render_status(ctx, y, data)
    local C = M.colors

    -- Fill background
    for x = 0, ctx.w - 1 do
        ctx:print(x, y, " ", {bg = C.status})
    end

    local left_x = 1

    -- Room name
    local room_name = data.room_name or "lobby"
    ctx:print(left_x, y, "[", {fg = C.dim, bg = C.status})
    left_x = left_x + 1
    ctx:print(left_x, y, room_name, {fg = C.system, bg = C.status})
    left_x = left_x + M.display_width(room_name)
    ctx:print(left_x, y, "]", {fg = C.dim, bg = C.status})
    left_x = left_x + 2

    -- User/model counts
    local counts = string.format("%d/%d", data.user_count or 0, data.model_count or 0)
    ctx:print(left_x, y, counts, {fg = C.statusfg, bg = C.status})
    left_x = left_x + #counts + 1

    -- Active model indicator
    if data.active_model then
        ctx:print(left_x, y, "◈", {fg = C.model, bg = C.status})
        left_x = left_x + 2
        ctx:print(left_x, y, data.active_model, {fg = C.model, bg = C.status})
        left_x = left_x + M.display_width(data.active_model) + 1
    end

    -- Right side: MCP status and duration
    local duration = data.duration or "0:00"
    local dur_width = #duration

    -- Build MCP string
    local mcp_parts = {}
    for _, conn in ipairs(data.mcp_connections or {}) do
        local indicator = conn.connected and "●" or "○"
        table.insert(mcp_parts, indicator .. conn.name)
    end
    local mcp_str = table.concat(mcp_parts, " ")
    local mcp_width = M.display_width(mcp_str)

    -- Position from right edge
    local right_margin = 1
    local dur_x = ctx.w - dur_width - right_margin
    local mcp_x = dur_x - mcp_width - 2

    if dur_x > left_x then
        ctx:print(dur_x, y, duration, {fg = C.dim, bg = C.status})
    end

    if mcp_width > 0 and mcp_x > left_x then
        ctx:print(mcp_x, y, mcp_str, {fg = C.dim, bg = C.status})
    end
end

--- Render input line
--- @param ctx table Draw context
--- @param y number Row to render at
--- @param data table {prompt, text, cursor}
function M.render_input(ctx, y, data)
    local C = M.colors
    local prompt = data.prompt or "> "
    local text = data.text or ""
    local cursor = data.cursor or 0

    local x = 0

    -- Prompt
    ctx:print(x, y, prompt, {fg = C.system})
    x = x + M.display_width(prompt)

    if #text == 0 then
        ctx:print(x, y, "▌", {fg = C.cursor})
    else
        -- Split at cursor (byte offset)
        local before = text:sub(1, cursor)
        local after = text:sub(cursor + 1)

        ctx:print(x, y, before)
        x = x + M.display_width(before)

        if #after > 0 then
            -- Get first UTF-8 char
            local char_end = utf8.offset(after, 2) or (#after + 1)
            local cursor_char = after:sub(1, char_end - 1)
            ctx:print(x, y, cursor_char, {fg = C.status, bg = C.cursor})
            x = x + M.display_width(cursor_char)
            ctx:print(x, y, after:sub(char_end))
        else
            ctx:print(x, y, "▌", {fg = C.cursor})
        end
    end
end

--------------------------------------------------------------------------------
-- Data Fetching (isolate global access)
--------------------------------------------------------------------------------

--- Fetch all data needed for rendering
--- @return table {history, status, input, scroll, mcp}
function M.fetch_render_data()
    local data = {
        history = {},
        status = {},
        input = {},
        scroll = nil,
        mcp = {},
    }

    -- History
    data.history = (tools and tools.history and tools.history(100)) or {}

    -- Status
    local ok, status = pcall(function()
        return sshwarma and sshwarma.call and sshwarma.call("status", {})
    end)
    data.status = (ok and status) or {}

    -- Input
    data.input = (tools and tools.input and tools.input()) or {}

    -- Scroll
    data.scroll = tools and tools.scroll and tools.scroll()

    -- MCP
    data.mcp = (tools and tools.mcp_connections and tools.mcp_connections()) or {}

    return data
end

--- Extract status bar data from raw status
--- @param status table Raw status from sshwarma.call("status")
--- @param mcp table[] MCP connections
--- @return table Prepared status bar data
function M.prepare_status_data(status, mcp)
    local room = status.room or {}
    local participants = status.participants or {}
    local session = status.session or {}

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

    return {
        room_name = room.name,
        user_count = user_count,
        model_count = model_count,
        active_model = active_model,
        duration = session.duration,
        mcp_connections = mcp,
    }
end

--------------------------------------------------------------------------------
-- Entry Points
--------------------------------------------------------------------------------

--- Main render entry point (called by Rust)
--- @param dirty_tags table Set of dirty region tags
--- @param tick number Current tick count
--- @param ctx table Draw context with w, h, print(), clear()
function on_tick(dirty_tags, tick, ctx)
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

    -- Fetch data
    local data = M.fetch_render_data()
    local session = data.status.session or {}
    local my_name = session.username or ""

    -- Render chat
    if chat_height > 0 and data.scroll then
        local display_lines = M.build_display_lines(data.history, w, my_name)
        M.render_chat(ctx, display_lines, data.scroll, chat_height)
    end

    -- Render status
    if status_row >= 0 then
        local status_data = M.prepare_status_data(data.status, data.mcp)
        M.render_status(ctx, status_row, status_data)
    end

    -- Render input
    if input_row >= 0 then
        M.render_input(ctx, input_row, data.input)
    end
end

--- Background tick (called every 500ms)
--- @param tick number Background tick count
function background(tick)
    -- Refresh status every 2 seconds for duration counter
    if tick % 4 == 0 then
        tools.mark_dirty("status")
    end
end

-- Export module globally for testing and extensibility
screen = M
return M
