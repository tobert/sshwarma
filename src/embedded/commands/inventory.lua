-- sshwarma inventory command handlers
--
-- Lua command handlers for inventory management:
--   inventory/inv - List inventory
--   equip        - Equip a tool
--   unequip      - Unequip a tool
--
-- Commands that display content use page.show() directly.

local page = require('page')
local M = {}

--------------------------------------------------------------------------------
-- Helper: format inventory line
--------------------------------------------------------------------------------

local function format_equipped_line(tool, show_status)
    local status = tool.available and "+" or "o"
    local qname = tool.qualified_name or tool.name
    if show_status then
        return string.format("  %s %s", status, qname)
    else
        return string.format("  %s", qname)
    end
end

--------------------------------------------------------------------------------
-- /inv [all] - List inventory
--------------------------------------------------------------------------------

function M.inventory(args)
    local inv = tools.inventory()

    if not inv then
        return {
            text = "Error: Could not load inventory",
            mode = "notification"
        }
    end

    local show_all = args and args:match("^%s*all%s*$")
    local lines = {}

    -- Equipped tools
    table.insert(lines, "Equipped:")
    if inv.equipped and #inv.equipped > 0 then
        for _, tool in ipairs(inv.equipped) do
            table.insert(lines, format_equipped_line(tool, true))
        end
    else
        table.insert(lines, "  (none)")
    end

    -- Available tools (if requested)
    if show_all then
        table.insert(lines, "")
        table.insert(lines, "Available to equip:")

        if inv.available and #inv.available > 0 then
            -- Build set of equipped IDs for filtering
            local equipped_ids = {}
            if inv.equipped then
                for _, tool in ipairs(inv.equipped) do
                    equipped_ids[tool.id] = true
                end
            end

            local shown = 0
            for _, tool in ipairs(inv.available) do
                if not equipped_ids[tool.id] then
                    table.insert(lines, format_equipped_line(tool, false))
                    shown = shown + 1
                end
            end

            if shown == 0 then
                table.insert(lines, "  (all tools equipped)")
            end
        else
            table.insert(lines, "  (none)")
        end
    end

    page.show("Inventory", table.concat(lines, "\n"))
    return {}
end

-- Alias
M.inv = M.inventory

--------------------------------------------------------------------------------
-- /equip <qualified_name> - Equip a tool
--------------------------------------------------------------------------------

function M.equip(args)
    if not args or args:match("^%s*$") then
        page.show("Equip", "Usage: /equip <qualified_name>\n" ..
                   "Example: /equip holler:sample\n" ..
                   "Use /inv all to see available tools.")
        return {}
    end

    local name = args:match("^%s*(%S+)")
    local result = tools.equip_tool(name)

    if not result then
        return {
            text = "Error: Could not equip tool",
            mode = "notification"
        }
    end

    if result.success then
        local lines = {}
        table.insert(lines, string.format("Equipped %s", name))

        if result.equipped and #result.equipped > 0 then
            table.insert(lines, "")
            table.insert(lines, "Currently equipped:")
            for _, qname in ipairs(result.equipped) do
                table.insert(lines, string.format("  %s", qname))
            end
        end

        page.show("Equip", table.concat(lines, "\n"))
        return {}
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /unequip <qualified_name> - Unequip a tool
--------------------------------------------------------------------------------

function M.unequip(args)
    if not args or args:match("^%s*$") then
        page.show("Unequip", "Usage: /unequip <qualified_name>\n" ..
                   "Example: /unequip holler:sample\n" ..
                   "Use /inv to see equipped tools.")
        return {}
    end

    local name = args:match("^%s*(%S+)")
    local result = tools.unequip_tool(name)

    if not result then
        return {
            text = "Error: Could not unequip tool",
            mode = "notification"
        }
    end

    if result.success then
        local lines = {}
        table.insert(lines, string.format("Unequipped %s", name))

        if result.equipped and #result.equipped > 0 then
            table.insert(lines, "")
            table.insert(lines, "Remaining equipped:")
            for _, qname in ipairs(result.equipped) do
                table.insert(lines, string.format("  %s", qname))
            end
        else
            table.insert(lines, "")
            table.insert(lines, "No tools equipped.")
        end

        page.show("Unequip", table.concat(lines, "\n"))
        return {}
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

return M
