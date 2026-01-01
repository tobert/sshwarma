-- sshwarma HUD renderer (new API)
--
-- Interface:
--   Rust calls: on_tick(tick, ctx) where ctx is a LuaDrawContext
--   Lua calls:  sshwarma.call("status", {}) for app state
--               sshwarma.call("time", {}) for current time
--
-- The ctx object provides drawing methods:
--   ctx:print(x, y, text, {fg, bg, bold, dim})
--   ctx:fill(x, y, w, h, char, {fg, bg})
--   ctx:hline(x, y, len, {fg, bg})
--   ctx:draw_box(x, y, w, h, {fg, bg})
--   ctx:clear()
--   ctx.w, ctx.h - dimensions

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
    notifications  = {},  -- {message, created_ms, ttl_ms}
}

--------------------------------------------------------------------------------
-- Helper Functions
--------------------------------------------------------------------------------

--- Get current spinner character based on tick
local function get_spinner(tick)
    return spinner_frames[(tick % #spinner_frames) + 1]
end

--- Get current tick character
local function get_tick(tick)
    local idx = math.floor(tick / 2) % #tick_chars
    return tick_chars[idx + 1]
end

--- Convert exits table {n = "room", e = "other"} to arrow string "↑→"
local function exits_to_arrows(exits)
    if not exits then return "" end
    local arrows = ""
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

--------------------------------------------------------------------------------
-- Drawing Helpers
--------------------------------------------------------------------------------

local border_style = { fg = colors.cyan }
local dim_style = { fg = colors.dim }

--- Draw the HUD box frame
local function draw_frame(ctx)
    local w, h = ctx.w, ctx.h

    -- Top border
    ctx:print(0, 0, box.tl, border_style)
    for x = 1, w - 2 do
        ctx:print(x, 0, box.h, border_style)
    end
    ctx:print(w - 1, 0, box.tr, border_style)

    -- Side borders
    for y = 1, h - 2 do
        ctx:print(0, y, box.v, border_style)
        ctx:print(w - 1, y, box.v, border_style)
    end

    -- Bottom border
    ctx:print(0, h - 1, box.bl, border_style)
    for x = 1, w - 2 do
        ctx:print(x, h - 1, box.h, border_style)
    end
    ctx:print(w - 1, h - 1, box.br, border_style)
end

--- Draw participants on row 1 (inside frame, y=1)
local function draw_participants(ctx, participants, tick)
    local x = 2  -- inside left border + padding

    -- Sort: users first, then models
    local users = {}
    local models = {}
    for _, p in ipairs(participants or {}) do
        if p.kind == "model" then
            table.insert(models, p)
        else
            table.insert(users, p)
        end
    end

    -- Draw users
    for i, p in ipairs(users) do
        if i > 1 then x = x + 2 end
        ctx:print(x, 1, p.name)
        x = x + #p.name
    end

    -- Draw models
    for _, p in ipairs(models) do
        if x > 2 then x = x + 2 end

        -- Determine glyph based on status
        local glyph = status_glyphs.idle
        local glyph_color = colors.dim

        if p.active then
            glyph = get_spinner(tick)
            glyph_color = colors.cyan
        end

        ctx:print(x, 1, glyph, { fg = glyph_color })
        x = x + 2
        ctx:print(x, 1, p.name, { fg = colors.magenta })
        x = x + #p.name
    end
end

--- Draw status line on row 2 (y=2)
local function draw_status(ctx, participants)
    local status_text = nil

    for _, p in ipairs(participants or {}) do
        if p.status and p.status ~= "" then
            status_text = p.status
            break
        end
    end

    if status_text then
        ctx:print(14, 2, status_text, dim_style)
    end
end

--- Draw room details on row 3 (y=3)
local function draw_room_details(ctx, room, participants)
    local x = 2

    -- Count participants
    local user_count = 0
    local model_count = 0
    for _, p in ipairs(participants or {}) do
        if p.kind == "model" then
            model_count = model_count + 1
        else
            user_count = user_count + 1
        end
    end

    local counts = string.format("%d user%s, %d model%s",
        user_count, user_count == 1 and "" or "s",
        model_count, model_count == 1 and "" or "s")
    ctx:print(x, 3, counts, dim_style)
    x = x + #counts

    -- Vibe if set
    local vibe = room and room.vibe
    if vibe and vibe ~= "" then
        local vibe_preview = vibe
        if #vibe_preview > 30 then
            vibe_preview = vibe_preview:sub(1, 29) .. "…"
        end
        ctx:print(x, 3, " │ ", dim_style)
        x = x + 3
        ctx:print(x, 3, "♪ ", { fg = colors.cyan })
        x = x + 2
        ctx:print(x, 3, vibe_preview, dim_style)
    end
end

--- Draw MCP connections on row 4 (y=4)
local function draw_mcp(ctx, mcp)
    local x = 2

    -- Get MCP state from tools namespace (legacy for now)
    if tools and tools.mcp_connections then
        mcp = tools.mcp_connections() or {}
    end

    if not mcp or #mcp == 0 then
        ctx:print(x, 4, "no MCP connections", dim_style)
        return
    end

    ctx:print(x, 4, "mcp: ", dim_style)
    x = x + 5

    for i, conn in ipairs(mcp) do
        if i > 1 then
            ctx:print(x, 4, "  ")
            x = x + 2
        end

        -- Connection indicator
        if conn.connected then
            ctx:print(x, 4, "●", { fg = colors.green })
        else
            ctx:print(x, 4, "○", { fg = colors.red })
        end
        x = x + 1

        -- Name
        ctx:print(x, 4, " " .. conn.name .. " ")
        x = x + 2 + #conn.name

        -- Stats
        local stats = string.format("(%d/%d)", conn.tools or 0, conn.calls or 0)
        ctx:print(x, 4, stats, dim_style)
        x = x + #stats

        -- Last tool
        if conn.last_tool then
            ctx:print(x, 4, " ")
            x = x + 1
            ctx:print(x, 4, conn.last_tool, { fg = colors.cyan })
            x = x + #conn.last_tool
        end
    end
end

--- Draw room info on row 5 (y=5)
local function draw_room_info(ctx, room, session, exits, tick)
    local x = 2

    -- Room name
    local room_name = (room and room.name) or "lobby"
    ctx:print(x, 5, room_name, { fg = colors.cyan })
    x = x + #room_name

    -- Exits
    local arrows = exits_to_arrows(exits)
    if arrows ~= "" then
        ctx:print(x, 5, " │ ", dim_style)
        x = x + 3
        ctx:print(x, 5, arrows)
        x = x + #arrows
    end

    -- Duration
    local duration = (session and session.duration) or "0:00:00"
    ctx:print(x, 5, " │ ", dim_style)
    x = x + 3
    ctx:print(x, 5, duration, dim_style)
    x = x + #duration

    -- Tick indicator
    ctx:print(x, 5, " ")
    x = x + 1
    ctx:print(x, 5, get_tick(tick), dim_style)
end

--------------------------------------------------------------------------------
-- Main Entry Point
--------------------------------------------------------------------------------

--- Called every 100ms by the ticker
--- @param tick number Monotonic tick counter
--- @param ctx userdata LuaDrawContext for the HUD region
function on_tick(tick, ctx)
    -- Clear the buffer
    ctx:clear()

    -- Get state
    local status = {}
    if sshwarma and sshwarma.call then
        status = sshwarma.call("status", {}) or {}
    end

    local room = status.room or {}
    local participants = status.participants or {}
    local session = status.session or {}
    local exits = status.exits or {}

    -- Draw frame (row 0 and row 6 are borders)
    draw_frame(ctx)

    -- Draw content rows (inside frame)
    draw_participants(ctx, participants, tick)      -- y=1
    draw_status(ctx, participants)                   -- y=2
    draw_room_details(ctx, room, participants)       -- y=3
    draw_mcp(ctx, nil)                               -- y=4
    draw_room_info(ctx, room, session, exits, tick)  -- y=5
    -- y=6 is bottom border
    -- y=7 is input area (not drawn by HUD)
end

