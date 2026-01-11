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

-- Powerline / Nerd Font glyphs via Unicode escapes
local ARROW_RIGHT = "\u{E0B0}"  --
local ARROW_LEFT  = "\u{E0B2}"  --

-- Colors - vibrant palette with dark purple chrome
M.colors = {
    -- Base
    fg       = "#c0caf5",
    dim      = "#565f89",
    bg       = "#1a1b26",

    -- Nicks & identities
    nick     = "#7aa2f7",
    model    = "#bb9af7",
    self     = "#9ece6a",
    system   = "#7dcfff",

    -- Bar backgrounds - dark purple gradient
    topbar   = "#1e1528",    -- darkest purple (top edge)
    topbar2  = "#241a30",    -- slightly lighter for sections
    status   = "#2a1f38",    -- medium purple (bottom bar)
    status2  = "#352845",    -- lighter purple for accents
    statusfg = "#a9b1d6",

    -- Accents
    error    = "#f7768e",
    warning  = "#e0af68",
    success  = "#9ece6a",
    info     = "#7dcfff",

    -- Vibrant highlights
    cyan     = "#7dcfff",
    magenta  = "#bb9af7",
    orange   = "#ff9e64",
    pink     = "#ff007c",
    green    = "#9ece6a",
    yellow   = "#e0af68",
    blue     = "#7aa2f7",
    red      = "#f7768e",
    teal     = "#1abc9c",
    purple   = "#9d7cd8",

    -- Input/mode
    cursor   = "#ff9e64",
    normal   = "#414868",
    insert   = "#73daca",

    -- MCP/tools
    mcp      = "#1abc9c",
    tools    = "#e0af68",

    -- Navigation
    exits    = "#bb9af7",
    vibe     = "#7dcfff",
}

-- ==========================================================================
-- Bar Definitions
-- ==========================================================================

-- Top bar: room context, vibe, navigation
bars.define("topbar", {
    position = "top",
    priority = 100,
    height = 1,
    items = {"room_badge", "vibe_snippet", "spacer", "exits_compass", "world_stats"},
    style = {bg = M.colors.topbar},
})

-- Bottom status: identity, tools, activity, participants, time, mode
bars.define("status", {
    position = "bottom",
    priority = 50,
    height = 1,
    items = {"username", "mcp_status", "tool_stats", "spacer", "participants", "spinner", "duration", "mode_indicator"},
    style = {bg = M.colors.status},
})

bars.define("input", {
    position = "bottom",
    priority = 100,
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

-- ==========================================================================
-- Top Bar Items (dark purple: topbar -> topbar2)
-- ==========================================================================

-- Room badge with accent background
bars.item("room_badge", function(state, _width)
    local room = state.room or {}
    local name = room.name or "lobby"
    local C = M.colors
    return {
        {text = " ⬢ ", style = {fg = C.cyan, bg = C.topbar2}},
        {text = name, style = {fg = C.cyan, bg = C.topbar2, bold = true}},
        {text = " ", style = {bg = C.topbar2}},
        {text = ARROW_RIGHT, style = {fg = C.topbar2, bg = C.topbar}},
    }
end)

-- Vibe snippet - first ~30 chars of room vibe
bars.item("vibe_snippet", function(state, _width)
    local room = state.room or {}
    local vibe = room.vibe or ""
    local C = M.colors

    if vibe == "" then
        return {}
    end

    -- Truncate to ~35 chars with ellipsis
    local snippet = vibe
    if #snippet > 35 then
        snippet = snippet:sub(1, 32) .. "..."
    end

    return {
        {text = " ", style = {bg = C.topbar}},
        {text = snippet, style = {fg = C.vibe, bg = C.topbar, italic = true}},
        {text = " ", style = {bg = C.topbar}},
    }
end)

-- Exits compass - show available directions with glyphs
bars.item("exits_compass", function(state, _width)
    local C = M.colors
    local segs = {}

    -- Get exits from state (populated by status call)
    local exits = state.exits or {}
    if #exits == 0 then
        return {}
    end

    -- Direction glyphs mapping
    local glyphs = {
        north = "↑", south = "↓", east = "→", west = "←",
        up = "⬆", down = "⬇",
        n = "↑", s = "↓", e = "→", w = "←",
    }

    table.insert(segs, {text = ARROW_LEFT, style = {fg = C.topbar2, bg = C.topbar}})
    table.insert(segs, {text = " ⚑ ", style = {fg = C.exits, bg = C.topbar2}})

    local exit_parts = {}
    for _, exit in ipairs(exits) do
        local dir = exit.direction or exit
        local glyph = glyphs[dir:lower()] or dir:sub(1,1)
        table.insert(exit_parts, glyph)
    end

    table.insert(segs, {text = table.concat(exit_parts, ""), style = {fg = C.exits, bg = C.topbar2}})
    table.insert(segs, {text = " ", style = {bg = C.topbar2}})

    return segs
end)

-- World stats - total rooms
bars.item("world_stats", function(_state, _width)
    local C = M.colors
    local rooms_list = tools and tools.rooms and tools.rooms() or {}
    local room_count = #rooms_list

    if room_count == 0 then
        return {{text = " ", style = {bg = C.topbar2}}}
    end

    return {
        {text = "🌐", style = {fg = C.purple, bg = C.topbar2}},
        {text = tostring(room_count), style = {fg = C.purple, bg = C.topbar2}},
        {text = " ", style = {bg = C.topbar2}},
    }
end)

-- ==========================================================================
-- Bottom Status Bar Items (medium purple: status -> status2)
-- ==========================================================================

-- Username with @ prefix - accented section
bars.item("username", function(_state, _width)
    local C = M.colors
    local user = tools and tools.current_user and tools.current_user()
    local name = user and user.name or "?"

    return {
        {text = " @", style = {fg = C.dim, bg = C.status2}},
        {text = name, style = {fg = C.self, bg = C.status2, bold = true}},
        {text = " ", style = {bg = C.status2}},
        {text = ARROW_RIGHT, style = {fg = C.status2, bg = C.status}},
    }
end)

-- MCP status - connected servers / total tools
bars.item("mcp_status", function(_state, _width)
    local C = M.colors
    local servers = tools and tools.mcp_list and tools.mcp_list() or {}

    local connected = 0
    local total_tools = 0
    local has_error = false

    for _, srv in ipairs(servers) do
        if srv.state == "connected" then
            connected = connected + 1
            total_tools = total_tools + (srv.tools or 0)
        elseif srv.error then
            has_error = true
        end
    end

    if #servers == 0 then
        return {}
    end

    local icon_color = has_error and C.warning or C.mcp
    return {
        {text = " ⚡", style = {fg = icon_color, bg = C.status}},
        {text = string.format("%d/%d", connected, total_tools), style = {fg = C.mcp, bg = C.status}},
        {text = " ", style = {bg = C.status}},
    }
end)

-- Tool call stats for current room
bars.item("tool_stats", function(_state, _width)
    local C = M.colors
    local stats = tools and tools.history_stats and tools.history_stats() or {}
    local total = stats.total or 0

    if total == 0 then
        return {}
    end

    return {
        {text = "⚙", style = {fg = C.tools, bg = C.status}},
        {text = tostring(total), style = {fg = C.tools, bg = C.status}},
        {text = " ", style = {bg = C.status}},
    }
end)

-- Activity spinner - animated when model is working
bars.item("spinner", function(state, _width)
    local C = M.colors
    local session = state.session or {}
    local participants = state.participants or {}

    -- Check if any model is active
    local model_active = false
    for _, p in ipairs(participants) do
        if p.kind == "model" and p.active then
            model_active = true
            break
        end
    end

    if not model_active then
        return {}
    end

    -- Spinner frames - braille dots animation
    local frames = {"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}
    local frame_idx = (session.spinner_frame or 0) % #frames + 1
    local spinner = frames[frame_idx]

    -- Cycle through colors for extra flair
    local spinner_colors = {C.cyan, C.magenta, C.orange, C.green, C.pink}
    local color_idx = math.floor((session.spinner_frame or 0) / 2) % #spinner_colors + 1

    return {
        {text = spinner .. " ", style = {fg = spinner_colors[color_idx], bg = C.status}},
    }
end)

bars.item("participants", function(state, _width)
    local participants = state.participants or {}
    local C = M.colors
    local segs = {}

    local users = 0
    local models = 0
    local active_model = nil

    for _, p in ipairs(participants) do
        if p.kind == "model" then
            models = models + 1
            if p.active then
                active_model = p.name
            end
        else
            users = users + 1
        end
    end

    -- Transition to accent section for participant counts
    table.insert(segs, {text = ARROW_LEFT, style = {fg = C.status2, bg = C.status}})
    table.insert(segs, {text = " 👤", style = {fg = C.blue, bg = C.status2}})
    table.insert(segs, {text = tostring(users), style = {fg = C.blue, bg = C.status2}})
    table.insert(segs, {text = " ", style = {bg = C.status2}})

    -- Model count with icon
    table.insert(segs, {text = "🤖", style = {fg = C.magenta, bg = C.status2}})
    table.insert(segs, {text = tostring(models), style = {fg = C.magenta, bg = C.status2}})
    table.insert(segs, {text = " ", style = {bg = C.status2}})

    -- Active model name if working
    if active_model then
        table.insert(segs, {text = "◈", style = {fg = C.orange, bg = C.status2}})
        table.insert(segs, {text = active_model, style = {fg = C.orange, bg = C.status2, bold = true}})
        table.insert(segs, {text = " ", style = {bg = C.status2}})
    end

    -- Transition back
    table.insert(segs, {text = ARROW_RIGHT, style = {fg = C.status2, bg = C.status}})

    return segs
end)

bars.item("duration", function(state, _width)
    local C = M.colors
    local session = state.session or {}
    local dur = session.duration or "0:00"
    return {
        {text = " ⏱", style = {fg = C.dim, bg = C.status}},
        {text = dur, style = {fg = C.yellow, bg = C.status}},
        {text = " ", style = {bg = C.status}},
    }
end)

bars.item("mode_indicator", function(_state, _width)
    local C = M.colors

    -- No trailing space - this is the rightmost item
    if mode.is_normal() then
        return {
            {text = " NOR", style = {fg = "#1a1b26", bg = C.normal, bold = true}},
        }
    else
        return {
            {text = " INS", style = {fg = "#1a1b26", bg = C.insert, bold = true}},
        }
    end
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
        exits = {},
    }

    local ok, status = pcall(function()
        return sshwarma and sshwarma.call and sshwarma.call("status", {})
    end)

    if ok and status then
        state.room = status.room or {}
        state.participants = status.participants or {}
        state.session = status.session or {}
        state.exits = status.exits or {}
    end

    -- Merge session data from tools.session() for spinner_frame etc
    local session_ok, session_data = pcall(function()
        return tools and tools.session and tools.session()
    end)
    if session_ok and session_data then
        for k, v in pairs(session_data) do
            state.session[k] = v
        end
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
                -- Debug: log bar positions
                if tools and tools.log_info then
                    tools.log_info(string.format(
                        "render bar %s: row=%d (0-idx=%d) h=%d ctx.h=%d",
                        name, info.row, info.row - 1, info.height, ctx.h
                    ))
                end
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
            local current_user = tools.current_user and tools.current_user()
            local my_name = current_user and current_user.name or ""
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
    if not session or not session.room_id then
        return
    end

    local hooks = tools.get_room_equipment and tools.get_room_equipment(session.room_id, "hook:background:ui")
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

-- Export to global for Rust access
_G.screen = M

return M
