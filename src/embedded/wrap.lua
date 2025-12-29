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
    return self:add_source("system", 0, function()
        return tools.wrap.system_layer()
    end, true)
end

-- Built-in source: Model identity and personality
function WrapBuilder:model_identity()
    return self:add_source("model", 10, function()
        return tools.wrap.model_layer()
    end, true)
end

-- Built-in source: Current room info
function WrapBuilder:room()
    return self:add_source("room", 20, function()
        return tools.wrap.room_layer()
    end, false)
end

-- Built-in source: Users and models present
function WrapBuilder:participants()
    return self:add_source("participants", 30, function()
        return tools.wrap.participants_layer()
    end, false)
end

-- Built-in source: Current user info
function WrapBuilder:user()
    return self:add_source("user", 25, function()
        return tools.wrap.user_layer()
    end, false)
end

-- Built-in source: Recent conversation history
function WrapBuilder:history(limit)
    limit = limit or 30
    return self:add_source("history", 100, function()
        return tools.wrap.history_layer(limit)
    end, false)
end

-- Built-in source: Journal entries
function WrapBuilder:journal(limit, kind)
    limit = limit or 5
    return self:add_source("journal", 80, function()
        return tools.wrap.journal_layer(kind, limit)
    end, false)
end

-- Built-in source: Inspiration board
function WrapBuilder:inspirations()
    return self:add_source("inspirations", 70, function()
        return tools.wrap.inspirations_layer()
    end, false)
end

-- Add a custom source with explicit content
function WrapBuilder:custom(name, content, priority, is_system)
    priority = priority or 50
    return self:add_source(name, priority, function()
        local tokens = tools.wrap.estimate_tokens(content)
        return { content = content, tokens = tokens }
    end, is_system)
end

-- Render sources of a specific type (system or context)
-- Returns the concatenated content within the token budget
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
    if is_system_type then
        budget = math.floor(self.target_tokens * 0.25)
    else
        budget = math.floor(self.target_tokens * 0.75)
    end

    local parts = {}
    local used_tokens = 0

    for _, source in ipairs(filtered) do
        local result = source.fetcher()
        if result and result.content and result.content ~= "" then
            local content = result.content
            local tokens = result.tokens or tools.wrap.estimate_tokens(content)

            -- Check if we can fit this source
            if used_tokens + tokens <= budget then
                table.insert(parts, content)
                used_tokens = used_tokens + tokens
            else
                -- Try to truncate to fit remaining budget
                local remaining = budget - used_tokens
                if remaining > 100 then  -- Only truncate if worth it
                    local truncated = tools.wrap.truncate(content, remaining)
                    table.insert(parts, truncated)
                    used_tokens = budget  -- At budget now
                end
                break  -- No more room
            end
        end
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
