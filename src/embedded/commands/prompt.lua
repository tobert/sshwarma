-- Prompt management command handlers for sshwarma
--
-- Commands for managing named prompts and target slots.
-- Commands that display content use page.show() directly.

local page = require('page')
local M = {}

-- Valid targets for prompt slots
local VALID_TARGETS = {
    system = true,
    room = true,
    model = true,
    user = true
}

-- Parse quoted content from args: /prompt name "content here"
local function parse_quoted(args)
    local name, content = args:match('^%s*(%S+)%s+"(.-)"')
    if not name then
        -- Try single quotes
        name, content = args:match("^%s*(%S+)%s+'(.-)'")
    end
    return name, content
end

-- Format prompt content for display (truncate long ones)
local function format_content(content, max)
    max = max or 60
    if not content then return "(empty)" end
    local preview = content:gsub("\n", " "):gsub("\r", "")
    if #preview > max then
        return preview:sub(1, max - 3) .. "..."
    end
    return preview
end

-- /prompt - Main prompt command dispatcher
-- Subcommands: list, show, push, pop, rm, insert, delete, or set
function M.prompt(args)
    local trimmed = args:match("^%s*(.-)%s*$")

    if trimmed == "" or trimmed == "list" then
        return M.prompt_list("")
    end

    -- Check for subcommands
    local subcmd, rest = trimmed:match("^(%S+)%s*(.*)")

    if subcmd == "show" then
        return M.prompt_show(rest)
    elseif subcmd == "delete" then
        return M.prompt_delete_cmd(rest)
    elseif subcmd == "push" then
        return M.prompt_push_cmd(rest)
    elseif subcmd == "pop" then
        return M.prompt_pop_cmd(rest)
    elseif subcmd == "rm" then
        return M.prompt_rm_cmd(rest)
    elseif subcmd == "insert" then
        return M.prompt_insert_cmd(rest)
    else
        -- Try to parse as: name "content"
        local name, content = parse_quoted(trimmed)
        if name and content then
            return M.prompt_set(name, content)
        else
            -- Show specific prompt or target
            return M.prompt_show(trimmed)
        end
    end
end

-- /prompt list - List all prompts and targets
function M.prompt_list(args)
    local prompts_data = tools.prompts()

    local lines = {}
    table.insert(lines, "=== Named Prompts ===\r\n\r\n")

    local prompts = prompts_data.prompts or {}
    if #prompts == 0 then
        table.insert(lines, "(no prompts defined)\r\n\r\n")
    else
        for _, p in ipairs(prompts) do
            local preview = format_content(p.content, 50)
            table.insert(lines, string.format("  %s: %s\r\n", p.name, preview))
        end
        table.insert(lines, "\r\n")
    end

    table.insert(lines, "=== Targets ===\r\n\r\n")

    -- Show slots for each target type
    local targets = {"system", "room", "model", "user"}
    local has_slots = false

    for _, target in ipairs(targets) do
        local slots = tools.target_slots(target)
        if #slots > 0 then
            has_slots = true
            table.insert(lines, string.format("%s:\r\n", target))
            for _, slot in ipairs(slots) do
                table.insert(lines, string.format("  [%d] %s\r\n", slot.slot, slot.prompt_name))
            end
        end
    end

    if not has_slots then
        table.insert(lines, "(no slots assigned)\r\n")
    end

    table.insert(lines, "\r\n")
    table.insert(lines, "Commands:\r\n")
    table.insert(lines, "  /prompt <name> \"<content>\"  Create/update prompt\r\n")
    table.insert(lines, "  /prompt show <name|target>   Show content\r\n")
    table.insert(lines, "  /prompt push <target> <name> Add prompt to target\r\n")
    table.insert(lines, "  /prompt pop <target>         Remove last slot\r\n")
    table.insert(lines, "  /prompt rm <target> <slot>   Remove by index\r\n")
    table.insert(lines, "  /prompt delete <name>        Delete prompt\r\n")

    page.show("Prompts", table.concat(lines))
    return {}
end

-- /prompt show <name|target> - Show prompt content or target slots
function M.prompt_show(args)
    local name_or_target = args:match("^%s*(.-)%s*$")

    if name_or_target == "" then
        return {
            text = "Usage: /prompt show <name|target>",
            mode = "notification"
        }
    end

    -- Check if it's a target
    if VALID_TARGETS[name_or_target] then
        local slots = tools.target_slots(name_or_target)
        local lines = {string.format("=== %s slots ===\r\n\r\n", name_or_target)}

        if #slots == 0 then
            table.insert(lines, "(no slots assigned)\r\n")
        else
            for _, slot in ipairs(slots) do
                table.insert(lines, string.format("[%d] %s\r\n", slot.slot, slot.prompt_name))
                if slot.content then
                    local preview = format_content(slot.content, 70)
                    table.insert(lines, string.format("    %s\r\n\r\n", preview))
                end
            end
        end

        page.show(name_or_target, table.concat(lines))
        return {}
    else
        -- Try to get named prompt
        local prompt = tools.get_prompt(name_or_target)
        if prompt then
            local content = prompt.content or "(empty)"
            content = content:gsub("\n", "\r\n")  -- Fix newlines for display

            page.show(prompt.name, string.format("=== %s ===\r\n\r\n%s", prompt.name, content))
            return {}
        else
            return {
                text = string.format("Prompt '%s' not found", name_or_target),
                mode = "notification"
            }
        end
    end
end

-- /prompt <name> "<content>" - Create or update a named prompt
function M.prompt_set(name, content)
    local result = tools.prompt_set(name, content)

    if result.success then
        return {
            text = string.format("Prompt '%s' saved (%d chars)", name, #content),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

-- /prompt delete <name> - Delete a named prompt
function M.prompt_delete_cmd(args)
    local name = args:match("^%s*(.-)%s*$")

    if name == "" then
        return {
            text = "Usage: /prompt delete <name>",
            mode = "notification"
        }
    end

    local result = tools.prompt_delete(name)

    if result.success then
        return {
            text = string.format("Deleted prompt '%s'", name),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "prompt not found"),
            mode = "notification"
        }
    end
end

-- /prompt push <target> <name> - Add prompt to target
function M.prompt_push_cmd(args)
    local target, name = args:match("^%s*(%S+)%s+(.-)%s*$")

    if not target or not name or name == "" then
        return {
            text = "Usage: /prompt push <target> <name>\r\nTargets: system, room, model, user",
            mode = "notification"
        }
    end

    if not VALID_TARGETS[target] then
        return {
            text = string.format("Invalid target '%s'. Use: system, room, model, user", target),
            mode = "notification"
        }
    end

    local result = tools.prompt_push(target, name)

    if result.success then
        return {
            text = string.format("Pushed '%s' to %s", name, target),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

-- /prompt pop <target> - Remove last slot from target
function M.prompt_pop_cmd(args)
    local target = args:match("^%s*(.-)%s*$")

    if target == "" then
        return {
            text = "Usage: /prompt pop <target>\r\nTargets: system, room, model, user",
            mode = "notification"
        }
    end

    if not VALID_TARGETS[target] then
        return {
            text = string.format("Invalid target '%s'. Use: system, room, model, user", target),
            mode = "notification"
        }
    end

    local result = tools.prompt_pop(target)

    if result.success then
        local removed = result.removed or "(unknown)"
        return {
            text = string.format("Popped '%s' from %s", removed, target),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "no slots to pop"),
            mode = "notification"
        }
    end
end

-- /prompt rm <target> <slot> - Remove slot by index
function M.prompt_rm_cmd(args)
    local target, slot_str = args:match("^%s*(%S+)%s+(.-)%s*$")

    if not target or not slot_str then
        return {
            text = "Usage: /prompt rm <target> <slot_index>",
            mode = "notification"
        }
    end

    if not VALID_TARGETS[target] then
        return {
            text = string.format("Invalid target '%s'. Use: system, room, model, user", target),
            mode = "notification"
        }
    end

    local slot = tonumber(slot_str)
    if not slot then
        return {
            text = "Slot index must be a number",
            mode = "notification"
        }
    end

    local result = tools.prompt_rm(target, slot)

    if result.success then
        return {
            text = string.format("Removed slot %d from %s", slot, target),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "slot not found"),
            mode = "notification"
        }
    end
end

-- /prompt insert <target> <slot> <name> - Insert prompt at slot
function M.prompt_insert_cmd(args)
    local target, slot_str, name = args:match("^%s*(%S+)%s+(%d+)%s+(.-)%s*$")

    if not target or not slot_str or not name or name == "" then
        return {
            text = "Usage: /prompt insert <target> <slot_index> <name>",
            mode = "notification"
        }
    end

    if not VALID_TARGETS[target] then
        return {
            text = string.format("Invalid target '%s'. Use: system, room, model, user", target),
            mode = "notification"
        }
    end

    local slot = tonumber(slot_str)
    if not slot then
        return {
            text = "Slot index must be a number",
            mode = "notification"
        }
    end

    local result = tools.prompt_insert(target, slot, name)

    if result.success then
        return {
            text = string.format("Inserted '%s' at %s slot %d", name, target, slot),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

return M
