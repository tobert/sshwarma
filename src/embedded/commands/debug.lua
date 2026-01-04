-- Debug command handlers for sshwarma
--
-- Commands for debugging and introspection.
-- Each handler receives args (string) and returns {text, mode, title?}

local M = {}

-- Estimate token count (rough approximation: ~4 chars per token)
local function estimate_tokens(text)
    if not text then return 0 end
    return math.ceil(#text / 4)
end

-- /wrap [model] - Preview context composition
-- Shows what would be sent to a model on @mention
function M.wrap(args)
    local model_name = args:match("^%s*(.-)%s*$")

    -- Default token budget for preview
    local target_tokens = 8192

    -- Try to call default_wrap with model info if available
    local ok, builder = pcall(function()
        return default_wrap(target_tokens)
    end)

    if not ok then
        return {
            text = "Error: Could not initialize wrap builder.\r\n" ..
                   "Make sure you are in a room with a model.",
            mode = "overlay",
            title = "Wrap Error"
        }
    end

    -- Get the composed output
    local system_ok, system_prompt = pcall(function()
        return builder:system_prompt()
    end)

    local context_ok, context = pcall(function()
        return builder:context()
    end)

    if not system_ok or not context_ok then
        return {
            text = "Error: Could not compose context.\r\n" ..
                   tostring(system_prompt) .. "\r\n" .. tostring(context),
            mode = "overlay",
            title = "Wrap Error"
        }
    end

    -- Format output with token counts
    local lines = {}

    table.insert(lines, "=== WRAP PREVIEW ===\r\n\r\n")

    if model_name and model_name ~= "" then
        table.insert(lines, string.format("Model: %s\r\n", model_name))
    end
    table.insert(lines, string.format("Target tokens: %d\r\n\r\n", target_tokens))

    -- System prompt section
    local sys_tokens = estimate_tokens(system_prompt)
    table.insert(lines, string.format("--- SYSTEM PROMPT (~%d tokens) ---\r\n\r\n", sys_tokens))

    if system_prompt and #system_prompt > 0 then
        -- Replace newlines for display
        local display_system = system_prompt:gsub("\n", "\r\n")
        table.insert(lines, display_system)
        table.insert(lines, "\r\n\r\n")
    else
        table.insert(lines, "(empty)\r\n\r\n")
    end

    -- Context section
    local ctx_tokens = estimate_tokens(context)
    table.insert(lines, string.format("--- CONTEXT (~%d tokens) ---\r\n\r\n", ctx_tokens))

    if context and #context > 0 then
        -- Replace newlines for display
        local display_context = context:gsub("\n", "\r\n")
        table.insert(lines, display_context)
        table.insert(lines, "\r\n\r\n")
    else
        table.insert(lines, "(empty)\r\n\r\n")
    end

    -- Summary
    local total_tokens = sys_tokens + ctx_tokens
    table.insert(lines, "---\r\n")
    table.insert(lines, string.format("Total: ~%d tokens (%d chars)\r\n",
        total_tokens, #(system_prompt or "") + #(context or "")))

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Wrap Preview"
    }
end

return M
