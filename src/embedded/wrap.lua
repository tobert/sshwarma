-- wrap.lua - Context composition for LLM interactions
--
-- Provides a lazy builder pattern for composing context from multiple sources.
-- Each source has a priority (lower = more important, included first) and
-- can be marked as system (stable, for preamble) or dynamic (for context).
--
-- Usage:
--   local w = wrap(30000)  -- 30K token budget
--       :system()          -- Global sshwarma environment
--       :model_identity()  -- Model personality
--       :room()            -- Room context
--       :history(30)       -- Recent messages
--
--   local system_prompt = w:system_prompt()  -- Stable (for .preamble())
--   local context = w:context()              -- Dynamic (prepend to message)

--------------------------------------------------------------------------------
-- Tokyo Night color palette (hex for segment rendering)
--------------------------------------------------------------------------------

local colors = {
    dim = "#565f89",
    fg = "#a9b1d6",
    cyan = "#7dcfff",
    blue = "#7aa2f7",
    green = "#9ece6a",
    yellow = "#e0af68",
    orange = "#ff9e64",
    magenta = "#bb9af7",
    red = "#f7768e",
}

-- Box drawing characters
local box = {
    tl = "\226\149\173",  -- ╭
    tr = "\226\149\174",  -- ╮
    bl = "\226\149\176",  -- ╰
    br = "\226\149\175",  -- ╯
    h = "\226\148\128",   -- ─
    v = "\226\148\130",   -- │
}

--------------------------------------------------------------------------------
-- Formatting helpers (produce markdown from unified tools data)
--------------------------------------------------------------------------------

--- Estimate tokens from text (simple heuristic: ~4 chars per token)
local function estimate_tokens(text)
    if not text then return 0 end
    return math.floor(#text / 4)
end

--- Create a layer result with content and token count
local function layer_result(content)
    return { content = content or "", tokens = estimate_tokens(content) }
end

--------------------------------------------------------------------------------
-- ANSI segment helpers (produce {Text, Fg, Bg} tables for TTY rendering)
--------------------------------------------------------------------------------

--- Create a segment with optional foreground color
local function seg(text, fg, bg)
    local s = { Text = text }
    if fg then s.Fg = fg end
    if bg then s.Bg = bg end
    return s
end

--- Pad a string to a given width (visible chars only)
local function pad(str, width)
    local len = #str  -- approximate for ASCII, emoji will be off
    if len >= width then
        return str
    end
    return str .. string.rep(" ", width - len)
end

--- Create a box row with left border, content segments, right border
local function box_row(inner_width, content_segments)
    local row = { seg(box.v, colors.dim) }

    -- Calculate content length and add segments
    local content_len = 0
    for _, s in ipairs(content_segments) do
        table.insert(row, s)
        content_len = content_len + #s.Text
    end

    -- Pad to inner width
    local padding = inner_width - content_len
    if padding > 0 then
        table.insert(row, seg(string.rep(" ", padding)))
    end

    table.insert(row, seg(box.v, colors.dim))
    return row
end

--- Format the global system layer (static sshwarma environment description)
local function format_system_layer()
    local content = [[You are an AI assistant in **sshwarma**, a collaborative SSH partyline where humans and AI models work together.

## Environment
- MUD-style text interface accessed via SSH
- Multiple users and models share rooms in real-time
- You have built-in functions for exploring rooms, navigating between them, and collaborating with users

## Communication Style
- Be conversational and collaborative
- Keep responses concise - this is a chat interface
- Use markdown sparingly (bold for emphasis, code blocks for code)

## Using Your Functions
- Your available functions are listed in "Your Functions" below
- Use them proactively when they help accomplish goals
- When asked what you can do, describe your capabilities based on those functions
- If a function fails, explain what went wrong and suggest alternatives]]

    return layer_result(content)
end

--- Format the model identity layer
local function format_model_layer()
    local model = tools.current_model()
    if not model then
        error("wrap: model identity layer requires a model in session context")
    end
    if not model.name or model.name == "" then
        error("wrap: model.name is required but empty")
    end

    local lines = {}
    table.insert(lines, "## Your Identity")
    table.insert(lines, "You are **@" .. model.name .. "**.")

    if model.system_prompt then
        table.insert(lines, "")
        table.insert(lines, model.system_prompt)
    end

    return layer_result(table.concat(lines, "\n"))
end

--- Format the current user layer
local function format_user_layer()
    local user = tools.current_user()
    if not user then
        return layer_result("## Current User\nUnknown user.\n")
    end

    return layer_result("## Current User\nYou are talking with **" .. user.name .. "**.\n")
end

--- Format the room context layer
local function format_room_layer()
    local look = tools.look()
    if not look or not look.room then
        return layer_result("**Location:** Lobby\n")
    end

    local lines = {}
    table.insert(lines, "## Room Context")
    table.insert(lines, "**Room:** " .. look.room)

    if look.description then
        table.insert(lines, "**Description:** " .. look.description)
    end

    if look.vibe then
        table.insert(lines, "**Vibe:** " .. look.vibe)
    end

    return layer_result(table.concat(lines, "\n"))
end

--- Format the participants layer
local function format_participants_layer()
    local who = tools.who()
    if not who or #who == 0 then
        return layer_result("")
    end

    local users = {}
    local models = {}

    for _, p in ipairs(who) do
        if p.is_model then
            table.insert(models, p.name)
        else
            table.insert(users, p.name)
        end
    end

    local parts = {}
    if #users > 0 then
        table.insert(parts, "**Users:** " .. table.concat(users, ", "))
    end
    if #models > 0 then
        table.insert(parts, "**Models:** " .. table.concat(models, ", "))
    end

    return layer_result(table.concat(parts, "\n"))
end

--- Format the conversation history layer
local function format_history_layer(limit)
    local messages = tools.history(limit)
    if not messages or #messages == 0 then
        return layer_result("")
    end

    local lines = {"## Recent History"}
    for _, msg in ipairs(messages) do
        table.insert(lines, msg.author .. ": " .. msg.content)
    end

    return layer_result(table.concat(lines, "\n"))
end

--- Format the journal layer
local function format_journal_layer(kind, limit)
    local entries = tools.journal(kind, limit)
    if not entries or #entries == 0 then
        return layer_result("")
    end

    local lines = {"## Journal"}
    for _, entry in ipairs(entries) do
        table.insert(lines, "[" .. entry.kind .. "] " .. entry.content)
    end

    return layer_result(table.concat(lines, "\n"))
end

--- Format the inspirations layer
local function format_inspirations_layer()
    local inspirations = tools.inspirations()
    if not inspirations or #inspirations == 0 then
        return layer_result("")
    end

    local lines = {"## Inspirations"}
    for _, insp in ipairs(inspirations) do
        table.insert(lines, "- " .. insp.content)
    end

    return layer_result(table.concat(lines, "\n"))
end

--------------------------------------------------------------------------------
-- ANSI formatters for /look command (TTY output)
--------------------------------------------------------------------------------

--- Format complete room info as ANSI segments for TTY display
--- Returns array of rows, each row is array of segments
local function format_look_ansi()
    local look = tools.look()
    local who = tools.who()
    local exits = tools.exits()

    -- Lobby case
    if not look or not look.room then
        return {
            { seg(box.tl, colors.dim), seg(string.rep(box.h, 20), colors.dim), seg(box.tr, colors.dim) },
            { seg(box.v, colors.dim), seg(" Lobby              ", colors.cyan), seg(box.v, colors.dim) },
            { seg(box.bl, colors.dim), seg(string.rep(box.h, 20), colors.dim), seg(box.br, colors.dim) },
        }
    end

    local inner = 40  -- inner width of box
    local rows = {}

    -- Top border with room name
    local title = " " .. look.room .. " "
    local title_len = #title
    local left_h = math.floor((inner - title_len) / 2)
    local right_h = inner - title_len - left_h
    table.insert(rows, {
        seg(box.tl, colors.dim),
        seg(string.rep(box.h, left_h), colors.dim),
        seg(title, colors.cyan),
        seg(string.rep(box.h, right_h), colors.dim),
        seg(box.tr, colors.dim),
    })

    -- Description
    if look.description then
        table.insert(rows, box_row(inner, { seg(" " .. look.description) }))
    end

    -- Empty line
    table.insert(rows, box_row(inner, { seg("") }))

    -- Users
    local users = {}
    local models = {}
    if who then
        for _, p in ipairs(who) do
            if p.is_model then
                table.insert(models, p.name)
            else
                table.insert(users, p.name)
            end
        end
    end

    if #users > 0 then
        table.insert(rows, box_row(inner, {
            seg(" "),
            seg(table.concat(users, ", "), colors.cyan),
        }))
    else
        table.insert(rows, box_row(inner, { seg(" Nobody else here.", colors.dim) }))
    end

    -- Models
    if #models > 0 then
        table.insert(rows, box_row(inner, {
            seg(" "),
            seg(table.concat(models, ", "), colors.magenta),
        }))
    end

    -- Vibe
    if look.vibe then
        table.insert(rows, box_row(inner, {
            seg(" vibe: ", colors.dim),
            seg(look.vibe, colors.green),
        }))
    end

    -- Exits
    if exits and next(exits) then
        table.insert(rows, box_row(inner, { seg("") }))
        for dir, room in pairs(exits) do
            local arrow = "->"
            if dir == "north" or dir == "up" then arrow = "^"
            elseif dir == "south" or dir == "down" then arrow = "v"
            elseif dir == "east" or dir == "in" then arrow = "->"
            elseif dir == "west" or dir == "out" then arrow = "<-"
            end
            table.insert(rows, box_row(inner, {
                seg(" " .. arrow .. " ", colors.yellow),
                seg(dir, colors.yellow),
                seg(" -> "),
                seg(room, colors.blue),
            }))
        end
    end

    -- Bottom border
    table.insert(rows, {
        seg(box.bl, colors.dim),
        seg(string.rep(box.h, inner), colors.dim),
        seg(box.br, colors.dim),
    })

    return rows
end

--- Format complete room info as markdown for model consumption
local function format_look_markdown()
    local look = tools.look()
    local who = tools.who()
    local exits = tools.exits()

    if not look or not look.room then
        return "## Location: Lobby\n\nYou are in the lobby. Use /rooms to see available rooms."
    end

    local lines = {}
    table.insert(lines, "## Room: " .. look.room)

    if look.description then
        table.insert(lines, look.description)
    end

    if look.vibe then
        table.insert(lines, "")
        table.insert(lines, "**Vibe:** " .. look.vibe)
    end

    -- Participants
    local users = {}
    local models = {}
    if who then
        for _, p in ipairs(who) do
            if p.is_model then
                table.insert(models, p.name)
            else
                table.insert(users, p.name)
            end
        end
    end

    table.insert(lines, "")
    table.insert(lines, "### Present")
    if #users > 0 then
        table.insert(lines, "- Users: " .. table.concat(users, ", "))
    else
        table.insert(lines, "- No other users")
    end
    if #models > 0 then
        table.insert(lines, "- Models: " .. table.concat(models, ", "))
    end

    -- Exits
    if exits and next(exits) then
        table.insert(lines, "")
        table.insert(lines, "### Exits")
        for dir, room in pairs(exits) do
            table.insert(lines, "- " .. dir .. " -> " .. room)
        end
    end

    return table.concat(lines, "\n")
end

--------------------------------------------------------------------------------
-- Prompt System Layers
--------------------------------------------------------------------------------

--- Format the model prompts layer (new prompt system)
--- Gets all prompts assigned to the current model via slots
local function format_model_prompts_layer()
    local model = tools.current_model()
    if not model then return layer_result("") end

    local prompts = tools.get_target_prompts(model.name)
    if not prompts or #prompts == 0 then
        return layer_result("")
    end

    local lines = {"## Model Instructions"}
    for _, slot in ipairs(prompts) do
        if slot.content then
            table.insert(lines, slot.content)
        end
    end

    if #lines == 1 then
        return layer_result("")  -- Only header, no content
    end

    return layer_result(table.concat(lines, "\n\n"))
end

--- Format user context layer (new prompt system)
--- Gets prompts assigned to the current user to help model understand them
local function format_user_prompts_layer()
    local user = tools.current_user()
    if not user then return layer_result("") end

    local prompts = tools.get_target_prompts(user.name)
    if not prompts or #prompts == 0 then
        return layer_result("")
    end

    local lines = {"### User Context"}
    for _, slot in ipairs(prompts) do
        if slot.content then
            table.insert(lines, "- " .. slot.prompt_name .. ": " .. slot.content)
        end
    end

    if #lines == 1 then
        return layer_result("")
    end

    return layer_result(table.concat(lines, "\n"))
end

--- Format the current user layer (enhanced with prompts)
local function format_user_with_prompts_layer()
    local user = tools.current_user()
    if not user then
        return layer_result("## Current User\nUnknown user.\n")
    end

    local lines = {"## Current User"}
    table.insert(lines, "You are talking with **" .. user.name .. "**.")

    -- Add user's prompts if any
    local prompts = tools.get_target_prompts(user.name)
    if prompts and #prompts > 0 then
        table.insert(lines, "")
        table.insert(lines, "### About " .. user.name)
        for _, slot in ipairs(prompts) do
            if slot.content then
                table.insert(lines, "- " .. slot.content)
            end
        end
    end

    return layer_result(table.concat(lines, "\n"))
end

--------------------------------------------------------------------------------
-- WrapBuilder class
--------------------------------------------------------------------------------

local WrapBuilder = {}
WrapBuilder.__index = WrapBuilder

-- Create a new WrapBuilder with a token budget
function WrapBuilder.new(target_tokens)
    local self = setmetatable({}, WrapBuilder)
    self.target_tokens = target_tokens or 30000
    self.sources = {}  -- {name, priority, fetcher, is_system}
    return self
end

-- Add a source with priority and system flag
-- Lower priority = more important, included first
-- is_system = true means it goes to system_prompt (stable, cacheable)
-- is_system = false means it goes to context (dynamic)
function WrapBuilder:add_source(name, priority, fetcher, is_system)
    table.insert(self.sources, {
        name = name,
        priority = priority,
        fetcher = fetcher,
        is_system = is_system or false,
    })
    return self
end

-- Built-in source: Global sshwarma environment
function WrapBuilder:system()
    return self:add_source("system", 0, format_system_layer, true)
end

-- Built-in source: Model identity and personality
function WrapBuilder:model_identity()
    return self:add_source("model", 10, format_model_layer, true)
end

-- Built-in source: Current room info
function WrapBuilder:room()
    return self:add_source("room", 20, format_room_layer, false)
end

-- Built-in source: Users and models present
function WrapBuilder:participants()
    return self:add_source("participants", 30, format_participants_layer, false)
end

-- Built-in source: Current user info
function WrapBuilder:user()
    return self:add_source("user", 25, format_user_layer, false)
end

-- Built-in source: Recent conversation history
function WrapBuilder:history(limit)
    limit = limit or 30
    return self:add_source("history", 100, function()
        return format_history_layer(limit)
    end, false)
end

-- Built-in source: Journal entries
function WrapBuilder:journal(limit, kind)
    limit = limit or 5
    return self:add_source("journal", 80, function()
        return format_journal_layer(kind, limit)
    end, false)
end

-- Built-in source: Inspiration board
function WrapBuilder:inspirations()
    return self:add_source("inspirations", 70, format_inspirations_layer, false)
end

-- Built-in source: Named prompts
-- Adds model prompts to system prompt section
function WrapBuilder:prompts()
    return self:add_source("prompts", 15, format_model_prompts_layer, true)
end

-- Built-in source: User with prompts (enhanced user info)
-- Replaces :user() with version that includes user prompts
function WrapBuilder:user_with_prompts()
    return self:add_source("user", 25, format_user_with_prompts_layer, false)
end

-- Add a custom source with explicit content
function WrapBuilder:custom(name, content, priority, is_system)
    priority = priority or 50
    return self:add_source(name, priority, function()
        return layer_result(content)
    end, is_system)
end

-- Render sources of a specific type (system or context)
-- Returns the concatenated content; errors if budget exceeded
function WrapBuilder:_render(is_system_type)
    -- Sort sources by priority
    local filtered = {}
    for _, source in ipairs(self.sources) do
        if source.is_system == is_system_type then
            table.insert(filtered, source)
        end
    end
    table.sort(filtered, function(a, b) return a.priority < b.priority end)

    -- Calculate budget for this type
    -- System prompt gets 1/4 of budget, context gets the rest
    local budget
    local budget_name
    if is_system_type then
        budget = math.floor(self.target_tokens * 0.25)
        budget_name = "system_prompt"
    else
        budget = math.floor(self.target_tokens * 0.75)
        budget_name = "context"
    end

    local parts = {}
    local used_tokens = 0

    for _, source in ipairs(filtered) do
        local result = source.fetcher()
        if result and result.content and result.content ~= "" then
            local content = result.content
            local tokens = result.tokens or estimate_tokens(content)
            table.insert(parts, content)
            used_tokens = used_tokens + tokens
        end
    end

    -- Error if budget exceeded (compaction required)
    if used_tokens > budget then
        error(string.format(
            "wrap: %s exceeds budget (%d tokens > %d budget). Compaction required.",
            budget_name, used_tokens, budget
        ))
    end

    return table.concat(parts, "\n\n")
end

-- Get the system prompt (stable, for .preamble())
function WrapBuilder:system_prompt()
    return self:_render(true)
end

-- Get the context (dynamic, prepend to message)
function WrapBuilder:context()
    return self:_render(false)
end

-- Constructor function
function wrap(target_tokens)
    return WrapBuilder.new(target_tokens)
end

-- Default wrap configuration
-- Called automatically on @mention with model's context_window
function default_wrap(target_tokens)
    return wrap(target_tokens)
        :system()
        :model_identity()
        :prompts()           -- Named prompts for this model (new system)
        :user_with_prompts() -- User info with their context prompts
        :room()
        :participants()
        :inspirations()
        :journal(5)
        :history(30)
end

-- Make available globally
_G.wrap = wrap
_G.default_wrap = default_wrap
_G.WrapBuilder = WrapBuilder

-- Look functions (for /look command and sshwarma_look tool)
_G.look_ansi = format_look_ansi
_G.look_markdown = format_look_markdown
