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

--- Format the profiles system layer (system_prompt additions from profiles)
local function format_profiles_system_layer()
    local model = tools.current_model()
    if not model then return layer_result("") end

    local profiles = tools.get_profiles(model.name)
    if not profiles or #profiles == 0 then
        return layer_result("")
    end

    local parts = {}
    for _, profile in ipairs(profiles) do
        if profile.system_prompt then
            table.insert(parts, profile.system_prompt)
        end
    end

    if #parts == 0 then
        return layer_result("")
    end

    return layer_result(table.concat(parts, "\n\n"))
end

--- Format the profiles context prefix layer
local function format_profiles_prefix_layer()
    local model = tools.current_model()
    if not model then return layer_result("") end

    local profiles = tools.get_profiles(model.name)
    if not profiles or #profiles == 0 then
        return layer_result("")
    end

    local parts = {}
    for _, profile in ipairs(profiles) do
        if profile.context_prefix then
            table.insert(parts, profile.context_prefix)
        end
    end

    if #parts == 0 then
        return layer_result("")
    end

    return layer_result(table.concat(parts, "\n\n"))
end

--- Format the profiles context suffix layer
local function format_profiles_suffix_layer()
    local model = tools.current_model()
    if not model then return layer_result("") end

    local profiles = tools.get_profiles(model.name)
    if not profiles or #profiles == 0 then
        return layer_result("")
    end

    local parts = {}
    for _, profile in ipairs(profiles) do
        if profile.context_suffix then
            table.insert(parts, profile.context_suffix)
        end
    end

    if #parts == 0 then
        return layer_result("")
    end

    return layer_result(table.concat(parts, "\n\n"))
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

-- Built-in source: Room profiles
-- Adds three layers:
--   profiles_system (priority 15, is_system) - system_prompt additions
--   profiles_prefix (priority 35, context) - context prefix
--   profiles_suffix (priority 95, context) - context suffix
function WrapBuilder:profiles()
    self:add_source("profiles_system", 15, format_profiles_system_layer, true)
    self:add_source("profiles_prefix", 35, format_profiles_prefix_layer, false)
    self:add_source("profiles_suffix", 95, format_profiles_suffix_layer, false)
    return self
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
        :profiles()  -- Room-specific profile customizations
        :user()
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
