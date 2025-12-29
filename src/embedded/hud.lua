-- sshwarma HUD renderer
-- Default HUD rendering script for the terminal display
--
-- Interface:
--   Rust calls: render_hud(now_ms, width, height)
--   Lua calls:  tools.hud_state() to get raw state
--               tools.clear_notifications() to drain pending notifications
--
-- Returns array of 8 rows, each row is array of segments:
--   { {Text = "...", Fg = "#rrggbb", Bg = "#rrggbb"}, ... }

--------------------------------------------------------------------------------
-- Tokyo Night Color Palette
--------------------------------------------------------------------------------

local colors = {
    fg      = "#a9b1d6",
    dim     = "#565f89",
    border  = "#7dcfff",
    cyan    = "#7dcfff",
    blue    = "#7aa2f7",
    green   = "#9ece6a",
    yellow  = "#e0af68",
    red     = "#f7768e",
    magenta = "#bb9af7",
    orange  = "#ff9e64",
}

--------------------------------------------------------------------------------
-- Box Drawing Characters (heavy style)
--------------------------------------------------------------------------------

local box = {
    tl = "┏",  -- top-left
    tr = "┓",  -- top-right
    bl = "┗",  -- bottom-left
    br = "┛",  -- bottom-right
    h  = "━",  -- horizontal
    v  = "┃",  -- vertical
}

--------------------------------------------------------------------------------
-- Spinner Frames (braille animation)
--------------------------------------------------------------------------------

local spinner_frames = {"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

--------------------------------------------------------------------------------
-- Status Glyphs
--------------------------------------------------------------------------------

local status_glyphs = {
    idle       = "◇",
    thinking   = "◈",
    running    = "◈",
    error      = "◉",
    offline    = "◌",
}

--------------------------------------------------------------------------------
-- Exit Direction Arrows
--------------------------------------------------------------------------------

local exit_arrows = {
    n  = "↑", north     = "↑",
    e  = "→", east      = "→",
    s  = "↓", south     = "↓",
    w  = "←", west      = "←",
    u  = "↑", up        = "↑",
    d  = "↓", down      = "↓",
    ne = "↗", northeast = "↗",
    se = "↘", southeast = "↘",
    nw = "↖", northwest = "↖",
    sw = "↙", southwest = "↙",
}

--------------------------------------------------------------------------------
-- Tick Indicator (for refresh visualization)
--------------------------------------------------------------------------------

local tick_chars = {"·", ":", "·", " "}

--------------------------------------------------------------------------------
-- Lua-Managed State (persists across render calls)
--------------------------------------------------------------------------------

local state = {
    spinner_frame  = 0,
    last_render_ms = 0,
    notifications  = {},  -- {message, created_ms, ttl_ms}
}

--------------------------------------------------------------------------------
-- Helper Functions
--------------------------------------------------------------------------------

--- Count visible characters in a string (handles multi-byte UTF-8)
--- Note: This is a simplification - assumes most glyphs are width 1
--- For proper terminal width, we'd need wcwidth equivalent
local function visible_len(text)
    if not text then return 0 end
    local len = 0
    -- Count UTF-8 codepoints (not bytes)
    for _ in text:gmatch("[%z\1-\127\194-\244][\128-\191]*") do
        len = len + 1
    end
    return len
end

--- Pad text with spaces to reach target width
local function pad(text, width)
    local text_len = visible_len(text)
    if text_len >= width then
        return text
    end
    return text .. string.rep(" ", width - text_len)
end

--- Format milliseconds duration as H:MM:SS
local function format_duration(ms)
    local total_secs = math.floor(ms / 1000)
    local hours = math.floor(total_secs / 3600)
    local mins  = math.floor((total_secs % 3600) / 60)
    local secs  = total_secs % 60
    return string.format("%d:%02d:%02d", hours, mins, secs)
end

--- Convert exits table {n = "room", e = "other"} to arrow string "↑→"
local function exits_to_arrows(exits)
    if not exits then return "" end
    local arrows = ""
    -- Process in consistent order for stable display
    local order = {"n", "ne", "e", "se", "s", "sw", "w", "nw", "u", "d"}
    for _, dir in ipairs(order) do
        if exits[dir] then
            local arrow = exit_arrows[dir]
            if arrow then
                arrows = arrows .. arrow
            end
        end
    end
    return arrows
end

--- Get current spinner character
local function get_spinner()
    return spinner_frames[(state.spinner_frame % #spinner_frames) + 1]
end

--- Get current tick character
local function get_tick()
    local idx = math.floor(state.spinner_frame / 2) % #tick_chars
    return tick_chars[idx + 1]
end

--- Create a text segment with optional colors
local function seg(text, fg, bg)
    local segment = { Text = text }
    if fg then segment.Fg = fg end
    if bg then segment.Bg = bg end
    return segment
end

--- Create a colored segment shorthand
local function c(text, color)
    return seg(text, color)
end

--- Create a dim segment
local function dim(text)
    return seg(text, colors.dim)
end

--- Create a border segment (cyan)
local function border(text)
    return seg(text, colors.cyan)
end

--------------------------------------------------------------------------------
-- Row Rendering Functions
--------------------------------------------------------------------------------

--- Render top border (Row 1)
local function render_top_border(width)
    local inner = string.rep(box.h, width - 2)
    return { border(box.tl .. inner .. box.tr) }
end

--- Render participants row (Row 2)
local function render_participants(ctx, inner_width)
    local segments = {}

    -- Left border
    table.insert(segments, border(box.v))
    table.insert(segments, seg("  "))  -- padding

    local content_len = 2  -- account for padding

    -- Sort: users first, then models
    local users = {}
    local models = {}
    for _, p in ipairs(ctx.participants or {}) do
        if p.kind == "user" then
            table.insert(users, p)
        else
            table.insert(models, p)
        end
    end

    local is_first = true

    -- Render users (no glyph for idle)
    for _, p in ipairs(users) do
        if not is_first then
            table.insert(segments, seg("  "))
            content_len = content_len + 2
        end
        is_first = false

        -- Users just show name (maybe with emoji status)
        if p.status == "emoji" and p.status_detail then
            table.insert(segments, seg(p.status_detail .. " "))
            content_len = content_len + 2
        end
        table.insert(segments, seg(p.name))
        content_len = content_len + visible_len(p.name)
    end

    -- Render models (with glyph)
    for _, p in ipairs(models) do
        if not is_first then
            table.insert(segments, seg("  "))
            content_len = content_len + 2
        end
        is_first = false

        -- Determine glyph and color
        local glyph, glyph_color
        if p.status == "thinking" or p.status == "running_tool" then
            glyph = get_spinner()
            glyph_color = colors.cyan
        elseif p.status == "error" then
            glyph = status_glyphs.error
            glyph_color = colors.red
        elseif p.status == "offline" then
            glyph = status_glyphs.offline
            glyph_color = colors.dim
        elseif p.status == "emoji" and p.status_detail then
            glyph = p.status_detail
            glyph_color = nil
        else
            glyph = status_glyphs.idle
            glyph_color = colors.dim
        end

        table.insert(segments, c(glyph, glyph_color))
        table.insert(segments, seg(" "))
        table.insert(segments, c(p.name, colors.magenta))
        content_len = content_len + visible_len(glyph) + 1 + visible_len(p.name)
    end

    -- Pad to fill row
    local padding = inner_width - content_len
    if padding > 0 then
        table.insert(segments, seg(string.rep(" ", padding)))
    end

    -- Right border
    table.insert(segments, border(box.v))

    return segments
end

--- Render status row (Row 3)
local function render_status(ctx, inner_width)
    local segments = {}

    table.insert(segments, border(box.v))
    table.insert(segments, seg("  "))  -- padding to match participants

    local content_len = 2
    local status_text = nil

    -- Find first active participant with status
    for _, p in ipairs(ctx.participants or {}) do
        if p.status == "thinking" then
            status_text = "thinking"
            break
        elseif p.status == "running_tool" then
            status_text = p.status_detail and ("running " .. p.status_detail) or "running tool"
            break
        elseif p.status == "error" then
            status_text = p.status_detail and p.status_detail:sub(1, 20) or "error"
            break
        end
    end

    if status_text then
        -- Indent to roughly align with model names
        local indent = "            "  -- 12 spaces
        table.insert(segments, seg(indent))
        table.insert(segments, dim(status_text))
        content_len = content_len + visible_len(indent) + visible_len(status_text)
    end

    -- Pad to fill row
    local padding = inner_width - content_len
    if padding > 0 then
        table.insert(segments, seg(string.rep(" ", padding)))
    end

    table.insert(segments, border(box.v))

    return segments
end

--- Render reserved/empty row (Row 4)
local function render_reserved(inner_width)
    local segments = {}
    table.insert(segments, border(box.v))
    table.insert(segments, seg(string.rep(" ", inner_width)))
    table.insert(segments, border(box.v))
    return segments
end

--- Render MCP connections row (Row 5)
local function render_mcp(ctx, inner_width)
    local segments = {}

    table.insert(segments, border(box.v))
    table.insert(segments, seg("  "))

    local content_len = 2

    local mcp = ctx.mcp or {}

    if #mcp == 0 then
        table.insert(segments, dim("no MCP connections"))
        content_len = content_len + 18
    else
        table.insert(segments, dim("mcp: "))
        content_len = content_len + 5

        for i, conn in ipairs(mcp) do
            if i > 1 then
                table.insert(segments, seg("  "))
                content_len = content_len + 2
            end

            -- Connection indicator
            if conn.connected then
                table.insert(segments, c("●", colors.green))
            else
                table.insert(segments, c("○", colors.red))
            end
            content_len = content_len + 1

            -- Name
            table.insert(segments, seg(" " .. conn.name .. " "))
            content_len = content_len + 2 + visible_len(conn.name)

            -- Stats (tools/calls)
            local stats = string.format("(%d/%d)", conn.tools or 0, conn.calls or 0)
            table.insert(segments, dim(stats))
            content_len = content_len + visible_len(stats)

            -- Last tool (if any)
            if conn.last_tool then
                table.insert(segments, seg(" "))
                table.insert(segments, c(conn.last_tool, colors.cyan))
                content_len = content_len + 1 + visible_len(conn.last_tool)
            end
        end
    end

    -- Pad to fill row
    local padding = inner_width - content_len
    if padding > 0 then
        table.insert(segments, seg(string.rep(" ", padding)))
    end

    table.insert(segments, border(box.v))

    return segments
end

--- Render room info row (Row 6)
local function render_room(ctx, now_ms, inner_width)
    local segments = {}

    table.insert(segments, border(box.v))
    table.insert(segments, seg("  "))

    local content_len = 2

    -- Room name
    local room_name = ctx.room or "lobby"
    table.insert(segments, c(room_name, colors.cyan))
    content_len = content_len + visible_len(room_name)

    -- Exits
    local arrows = exits_to_arrows(ctx.exits)
    if arrows ~= "" then
        table.insert(segments, seg(" │ "))
        table.insert(segments, seg(arrows))
        content_len = content_len + 3 + visible_len(arrows)
    end

    -- Duration
    local duration_ms = now_ms - (ctx.session_start_ms or now_ms)
    local duration_str = format_duration(duration_ms)
    table.insert(segments, seg(" │ "))
    table.insert(segments, dim(duration_str))
    content_len = content_len + 3 + visible_len(duration_str)

    -- Tick indicator
    table.insert(segments, seg(" "))
    table.insert(segments, dim(get_tick()))
    content_len = content_len + 2

    -- Pad to fill row
    local padding = inner_width - content_len
    if padding > 0 then
        table.insert(segments, seg(string.rep(" ", padding)))
    end

    table.insert(segments, border(box.v))

    return segments
end

--- Render bottom border with optional notification (Row 7)
local function render_bottom_border(width, now_ms)
    -- Get most recent unexpired notification
    local notif = nil
    for i = #state.notifications, 1, -1 do
        local n = state.notifications[i]
        local age = now_ms - n.created_ms
        if age < n.ttl_ms then
            notif = n
            break
        end
    end

    if notif then
        -- Build notification text
        local notif_text = " ⚡ " .. notif.message .. " "
        local notif_len = visible_len(notif_text)

        -- Calculate border lengths
        local inner_width = width - 2  -- minus corners
        local min_border = 4

        if notif_len + min_border < inner_width then
            local right_border_len = 2
            local left_border_len = inner_width - notif_len - right_border_len

            local left_border = string.rep(box.h, left_border_len)
            local right_border = string.rep(box.h, right_border_len)

            return {
                border(box.bl),
                border(left_border),
                c(notif_text, colors.yellow),
                border(right_border),
                border(box.br),
            }
        end
    end

    -- No notification - plain border
    local inner = string.rep(box.h, width - 2)
    return { border(box.bl .. inner .. box.br) }
end

--------------------------------------------------------------------------------
-- Main Render Function
--------------------------------------------------------------------------------

--- Main entry point called by Rust
--- @param now_ms number Current time in milliseconds
--- @param width number Terminal width
--- @param height number Terminal height (always 8 for HUD)
--- @return table Array of 8 rows, each row is array of segments
function render_hud(now_ms, width, height)
    -- 1. Advance spinner based on time delta (every 100ms)
    local delta = now_ms - state.last_render_ms
    if delta >= 100 then
        local frames_to_advance = math.floor(delta / 100)
        state.spinner_frame = (state.spinner_frame + frames_to_advance) % 10
        state.last_render_ms = now_ms
    end

    -- 2. Get raw state from Rust
    local ctx = {}
    if tools and tools.hud_state then
        ctx = tools.hud_state() or {}
    end

    -- 3. Drain pending notifications from Rust
    local pending = ctx.pending_notifications or {}
    for _, n in ipairs(pending) do
        table.insert(state.notifications, {
            message    = n.message,
            created_ms = n.created_ms,
            ttl_ms     = n.ttl_ms,
        })
    end
    if #pending > 0 and tools and tools.clear_notifications then
        tools.clear_notifications()
    end

    -- 4. Expire old notifications
    for i = #state.notifications, 1, -1 do
        local n = state.notifications[i]
        local age = now_ms - n.created_ms
        if age >= n.ttl_ms then
            table.remove(state.notifications, i)
        end
    end

    -- 5. Calculate inner width (account for borders)
    local inner_width = width - 2

    -- 6. Render each row
    return {
        render_top_border(width),               -- Row 1: top border
        render_participants(ctx, inner_width),  -- Row 2: participants
        render_status(ctx, inner_width),        -- Row 3: status
        render_reserved(inner_width),           -- Row 4: reserved/empty
        render_mcp(ctx, inner_width),           -- Row 5: MCP connections
        render_room(ctx, now_ms, inner_width),  -- Row 6: room info
        render_bottom_border(width, now_ms),    -- Row 7: bottom border + notification
        {},                                     -- Row 8: empty (input line handled by Rust)
    }
end

--------------------------------------------------------------------------------
-- Mock tools for standalone testing
--------------------------------------------------------------------------------

if not tools then
    tools = {
        hud_state = function()
            return {
                room = "hootenanny",
                session_start_ms = 0,
                participants = {
                    {name = "alice", kind = "user", status = "idle"},
                    {name = "bob", kind = "user", status = "idle"},
                    {name = "qwen-8b", kind = "model", status = "thinking"},
                    {name = "qwen-4b", kind = "model", status = "idle"},
                },
                mcp = {
                    {name = "holler", tools = 7, calls = 12, last_tool = "sample", connected = true},
                    {name = "exa", tools = 3, calls = 5, connected = true},
                },
                exits = {n = "kitchen", e = "garden"},
                pending_notifications = {},
            }
        end,
        clear_notifications = function() end,
    }
end

--------------------------------------------------------------------------------
-- Standalone test (run with: lua hud.lua)
--------------------------------------------------------------------------------

if arg and arg[0] then
    -- Running as script
    local function dump_segments(row)
        local line = ""
        for _, seg in ipairs(row) do
            line = line .. (seg.Text or "")
        end
        return line
    end

    -- Simulate some time passing with a notification
    state.notifications = {
        {message = "bob joined", created_ms = 0, ttl_ms = 5000},
    }

    local rows = render_hud(1000, 80, 8)

    print("=== HUD Output (80 columns) ===")
    for i, row in ipairs(rows) do
        local line = dump_segments(row)
        if line ~= "" then
            print(string.format("Row %d: %s", i, line))
        else
            print(string.format("Row %d: (empty)", i))
        end
    end

    print("\n=== Segment Detail (Row 2) ===")
    for i, seg in ipairs(rows[2]) do
        local fg = seg.Fg or "default"
        local text = seg.Text or ""
        print(string.format("  [%d] fg=%s text=%q", i, fg, text))
    end
end

return {
    render_hud = render_hud,
    colors = colors,
    box = box,
    spinner_frames = spinner_frames,
}
