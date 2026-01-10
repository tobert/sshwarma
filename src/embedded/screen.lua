-- screen.lua - Full-screen renderer using bar system
--
-- Entry points (called by Rust):
--   on_tick(dirty_tags, tick, ctx)
--   background(tick)

local bars = require 'ui.bars'
local pages = require 'ui.pages'
local scroll = require 'ui.scroll'
local mode = require 'ui.mode'
local input = require 'ui.input'

local M = {}

-- Colors
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
    normal   = "#565f89",
    insert   = "#9ece6a",
}

-- ==========================================================================
-- Bar Definitions
-- ==========================================================================

bars.define("status", {
    position = "bottom",
    priority = 50,  -- above input but at bottom
    height = 1,
    items = {"room_name", "spacer", "participants", "duration", "mode_indicator"},
    style = {bg = M.colors.status},
})

bars.define("input", {
    position = "bottom",
    priority = 100,  -- closest to bottom edge
    height = 1,
    items = {"prompt", "input_text"},
})

-- ==========================================================================
-- Bar Items
-- ==========================================================================

bars.item("room_name", function(state, _width)
    local room = state.room or {}
    return {
        {text = "[", style = {fg = M.colors.dim, bg = M.colors.status}},
        {text = room.name or "lobby", style = {fg = M.colors.system, bg = M.colors.status}},
        {text = "] ", style = {fg = M.colors.dim, bg = M.colors.status}},
    }
end)

bars.item("participants", function(state, _width)
    local participants = state.participants or {}
    local segs = {}

    local users = 0
    local models = 0
    local active_model = nil

    for _, p in ipairs(participants) do
        if p.kind == "model" then
            models = models + 1
            if p.status ~= "idle" then
                active_model = p.name
            end
        else
            users = users + 1
        end
    end

    table.insert(segs, {text = string.format("%d/%d ", users, models), style = {fg = M.colors.statusfg, bg = M.colors.status}})

    if active_model then
        table.insert(segs, {text = "◈ ", style = {fg = M.colors.model, bg = M.colors.status}})
        table.insert(segs, {text = active_model .. " ", style = {fg = M.colors.model, bg = M.colors.status}})
    end

    return segs
end)

bars.item("duration", function(state, _width)
    local session = state.session or {}
    local dur = session.duration or "0:00"
    return {{text = dur .. " ", style = {fg = M.colors.dim, bg = M.colors.status}}}
end)

bars.item("mode_indicator", function(_state, _width)
    local m = mode.indicator()
    local color = mode.is_normal() and M.colors.normal or M.colors.insert
    return {{text = " " .. m .. " ", style = {fg = color, bg = M.colors.status, bold = true}}}
end)

bars.item("prompt", function(state, _width)
    local room = state.room or {}
    local room_name = room.name or "lobby"

    if mode.is_normal() then
        return {{text = room_name .. "│", style = {fg = M.colors.dim}}}
    else
        return {{text = room_name .. "> ", style = {fg = M.colors.system}}}
    end
end)

bars.item("input_text", function(_state, _width)
    local inp = input.get_state()
    local text = inp.text or ""
    local cursor = inp.cursor or 0
    local segs = {}

    if #text == 0 then
        if mode.is_insert() then
            table.insert(segs, {text = "▌", style = {fg = M.colors.cursor}})
        end
    else
        local before = text:sub(1, cursor)
        local after = text:sub(cursor + 1)

        if #before > 0 then
            table.insert(segs, {text = before})
        end

        if #after > 0 then
            local char_end = utf8.offset(after, 2) or (#after + 1)
            local cursor_char = after:sub(1, char_end - 1)
            table.insert(segs, {text = cursor_char, style = {fg = M.colors.status, bg = M.colors.cursor}})
            if char_end <= #after then
                table.insert(segs, {text = after:sub(char_end)})
            end
        elseif mode.is_insert() then
            table.insert(segs, {text = "▌", style = {fg = M.colors.cursor}})
        end
    end

    return segs
end)

-- ==========================================================================
-- Text Utilities
-- ==========================================================================

function M.display_width(str)
    if not str or str == "" then return 0 end
    if tools and tools.display_width then
        return tools.display_width(str)
    end
    return utf8.len(str) or #str
end

function M.wrap_text(text, width)
    if width <= 0 then return {""} end
    if not text or text == "" then return {""} end

    local lines = {}

    for segment in (text .. "\n"):gmatch("([^\n]*)\n") do
        if #segment == 0 then
            table.insert(lines, "")
        elseif M.display_width(segment) <= width then
            table.insert(lines, segment)
        else
            local pos = 1
            while pos <= #segment do
                local end_pos = pos
                local current_width = 0

                while end_pos <= #segment do
                    local next_pos = utf8.offset(segment, 2, end_pos) or (#segment + 1)
                    local char = segment:sub(end_pos, next_pos - 1)
                    local char_width = M.display_width(char)

                    if current_width + char_width > width then break end

                    current_width = current_width + char_width
                    end_pos = next_pos
                end

                if end_pos == pos then
                    end_pos = utf8.offset(segment, 2, pos) or (#segment + 1)
                end

                local chunk = segment:sub(pos, end_pos - 1)

                if end_pos <= #segment then
                    local last_space = chunk:match(".*()%s")
                    if last_space and last_space > #chunk / 2 then
                        chunk = chunk:sub(1, last_space - 1)
                        end_pos = pos + last_space
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

-- ==========================================================================
-- Chat Rendering
-- ==========================================================================

function M.build_display_lines(messages, width, my_name)
    local display_lines = {}
    local prefix_width = 0
    local C = M.colors

    for _, msg in ipairs(messages) do
        local author = msg.author or "???"
        prefix_width = math.max(prefix_width, M.display_width(author) + 3)
    end

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

        local nick_color = C.nick
        if author == my_name then
            nick_color = C.self
        elseif is_model then
            nick_color = C.model
        elseif author == "system" then
            nick_color = C.system
        end

        local wrapped = M.wrap_text(content, content_width)

        for i, line_text in ipairs(wrapped) do
            table.insert(display_lines, {
                text = line_text,
                nick_color = nick_color,
                is_first_line = (i == 1),
                is_last_line = (i == #wrapped),
                is_streaming = is_streaming,
                prefix_width = prefix_width,
                author = (i == 1) and author or nil,
            })
        end
    end

    return display_lines
end

function M.render_chat(ctx, display_lines, page_name, height)
    local C = M.colors
    local total_lines = #display_lines

    scroll.set_content_height(page_name, total_lines)
    scroll.set_viewport_height(page_name, height)

    local start_line, end_line = scroll.visible_range(page_name)

    if end_line > total_lines then end_line = total_lines end
    if start_line < 0 then start_line = 0 end

    for i = start_line, end_line - 1 do
        local line_idx = i + 1
        local line = display_lines[line_idx]

        if line then
            local y = i - start_line
            local x = 0

            if line.is_first_line and line.author then
                ctx:print(x, y, "<", {fg = C.dim})
                x = x + 1
                ctx:print(x, y, line.author, {fg = line.nick_color})
                x = x + M.display_width(line.author)
                ctx:print(x, y, "> ", {fg = C.dim})
                x = x + 2
            else
                x = line.prefix_width or 0
            end

            local text = line.text
            if line.is_streaming and line.is_last_line then
                text = text .. " ◌"
            end
            ctx:print(x, y, text)
        end
    end

    if not scroll.is_following(page_name) and total_lines > height then
        local pct = math.floor(scroll.percent(page_name) * 100)
        local indicator = string.format("── %d%% ──", pct)
        local ix = ctx.w - M.display_width(indicator)
        ctx:print(ix, height - 1, indicator, {fg = C.dim})
    end
end

-- ==========================================================================
-- Help Page Rendering
-- ==========================================================================

M.help_content = [[
sshwarma - collaborative rooms for humans and models

Navigation (Normal Mode):
  j/k or ↑/↓    Scroll content
  h/l or ←/→    Switch pages
  g             Jump to bottom (follow)
  G             Jump to top
  q             Close page
  ?             Open help
  i             Enter insert mode
  / @           Enter insert with prefix

Editing (Insert Mode):
  ↑/↓           History prev/next
  ←/→           Cursor movement
  Ctrl+A/E      Beginning/end of line
  Ctrl+W/U/K    Delete word/to-start/to-end
  Tab           Completion
  Enter         Send message
  Escape        Return to normal mode
  Ctrl+C        Clear and return to normal

Commands:
  /rooms        List available rooms
  /join <room>  Enter a room
  /leave        Return to lobby
  /look         Room summary
  /who          Who's in the room
  /history [n]  Recent messages

Press q to close this help.
]]

function M.render_help(ctx, height)
    M.render_page_content(ctx, "help", M.help_content, height)
end

-- ==========================================================================
-- Generic Page Content Rendering
-- ==========================================================================

function M.render_page_content(ctx, page_name, content, height)
    local C = M.colors
    -- Split content by newlines, preserving empty lines
    local lines = {}
    local pos = 1
    while true do
        local nl = content:find("\n", pos, true)
        if nl then
            table.insert(lines, content:sub(pos, nl - 1))
            pos = nl + 1
        else
            table.insert(lines, content:sub(pos))
            break
        end
    end

    scroll.set_content_height(page_name, #lines)
    scroll.set_viewport_height(page_name, height)

    local start_line, end_line = scroll.visible_range(page_name)
    if end_line > #lines then end_line = #lines end

    for i = start_line, end_line - 1 do
        local line = lines[i + 1] or ""
        local y = i - start_line
        ctx:print(0, y, line, {fg = C.fg})
    end

    if not scroll.is_following(page_name) and #lines > height then
        local pct = math.floor(scroll.percent(page_name) * 100)
        local indicator = string.format("── %d%% ──", pct)
        local ix = ctx.w - M.display_width(indicator)
        ctx:print(ix, height - 1, indicator, {fg = C.dim})
    end
end

-- ==========================================================================
-- Data Fetching
-- ==========================================================================

function M.fetch_state()
    local state = {
        room = {},
        participants = {},
        session = {},
        history = {},
    }

    local ok, status = pcall(function()
        return sshwarma and sshwarma.call and sshwarma.call("status", {})
    end)

    if ok and status then
        state.room = status.room or {}
        state.participants = status.participants or {}
        state.session = status.session or {}
    end

    state.history = (tools and tools.history and tools.history(100)) or {}

    return state
end

-- ==========================================================================
-- Main Render
-- ==========================================================================

function on_tick(dirty_tags, tick, ctx)
    ctx:clear()

    local state = M.fetch_state()
    local bar_layout = bars.compute_layout(ctx.w, ctx.h, state)

    -- Render bars (layout is 1-indexed, ctx:sub is 0-indexed)
    for name, info in pairs(bar_layout) do
        if name ~= "content" then
            local bar_def = bars.get(name)
            if bar_def then
                local bar_ctx = ctx:sub(0, info.row - 1, ctx.w, info.height)
                bars.render(bar_ctx, bar_def, state)
            end
        end
    end

    -- Render content area (layout is 1-indexed, ctx:sub is 0-indexed)
    local content = bar_layout.content
    if content and content.height > 0 then
        local content_ctx = ctx:sub(0, content.row - 1, ctx.w, content.height)

        -- Clear content area (fills with spaces) to remove stale content
        content_ctx:clear()

        local current_page = pages.current_name()

        if current_page == "chat" then
            local my_name = (state.session or {}).username or ""
            local display_lines = M.build_display_lines(state.history, content_ctx.w, my_name)
            M.render_chat(content_ctx, display_lines, "chat", content.height)
        elseif current_page == "help" then
            M.render_help(content_ctx, content.height)
        else
            -- Generic page with scroll support
            local page = pages.current()
            if page and page.content then
                M.render_page_content(content_ctx, page.name, page.content, content.height)
            end
        end
    end

    -- Report hardware cursor position for layered blink effect
    -- Layout is 1-indexed (matches ANSI), columns are 1-indexed
    if bar_layout["input"] and tools and tools.set_cursor_pos then
        local inp = input.get_state()
        local room = (state.room or {}).name or "lobby"

        -- Calculate prompt width (must match bars.item("prompt") output)
        local prompt_text = mode.is_normal() and (room .. "│") or (room .. "> ")
        local prompt_width = M.display_width(prompt_text)

        -- Calculate text before cursor width
        local text_before = (inp.text or ""):sub(1, inp.cursor or 0)
        local text_width = M.display_width(text_before)

        -- Layout row is 1-indexed, column needs +1 for ANSI 1-indexing
        local cursor_row = bar_layout["input"].row
        local cursor_col = prompt_width + text_width + 1

        tools.set_cursor_pos(cursor_row, cursor_col)
    end
end

-- Track last-run tick for each UI background hook
local hook_last_run = {}

function background(tick)
    -- Mark status dirty every 2 seconds (4 ticks at 500ms)
    if tick % 4 == 0 then
        if tools and tools.mark_dirty then
            tools.mark_dirty("status")
        end
    end

    -- Run hook:background:ui things (session-tied, only when room is viewed)
    local session = tools.session and tools.session()
    if not session or not session.room_name then
        return
    end

    local hooks = tools.get_room_equipment and tools.get_room_equipment(session.room_name, "hook:background:ui")
    if not hooks or #hooks == 0 then
        return
    end

    local tick_ms = 500  -- Each tick is 500ms
    local current_time = tick * tick_ms

    for _, hook in ipairs(hooks) do
        -- Parse interval from config (default 500ms = every tick)
        local interval_ms = 500
        if hook.config then
            local ok, config = pcall(function()
                return require("cjson") and require("cjson").decode(hook.config)
            end)
            if not ok then
                -- Try simpler JSON parsing
                local ms = hook.config:match('"interval_ms"%s*:%s*(%d+)')
                if ms then interval_ms = tonumber(ms) end
            elseif config and config.interval_ms then
                interval_ms = config.interval_ms
            end
        end

        -- Check if this hook should run this tick
        local last_run = hook_last_run[hook.thing_id] or 0
        if current_time - last_run >= interval_ms then
            hook_last_run[hook.thing_id] = current_time

            -- Get and execute the thing's code
            local thing = tools.thing_get_by_id and tools.thing_get_by_id(hook.thing_id)
            if thing and thing.code then
                local ok, result = pcall(function()
                    return tools.execute_code(thing.code, {tick = tick})
                end)
                if not ok then
                    tools.log_warn(string.format(
                        "UI background hook %s failed: %s",
                        hook.qualified_name or hook.thing_id,
                        tostring(result)
                    ))
                end
            end
        end
    end
end

screen = M
return M
