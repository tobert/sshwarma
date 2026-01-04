-- sshwarma inventory command handlers
--
-- Lua command handlers for inventory management:
--   inventory/inv - List inventory
--   equip        - Equip a tool
--   unequip      - Unequip a tool
--   bring        - Bind asset to room
--   drop         - Unbind asset from room
--   examine      - Inspect a bound asset
--
-- Each handler receives args string and returns:
--   {text = "output", mode = "overlay"|"notification", title = "Title"}

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

    return {
        text = table.concat(lines, "\n"),
        mode = "overlay",
        title = "Inventory"
    }
end

-- Alias
M.inv = M.inventory

--------------------------------------------------------------------------------
-- /equip <qualified_name> - Equip a tool
--------------------------------------------------------------------------------

function M.equip(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /equip <qualified_name>\n" ..
                   "Example: /equip holler:sample\n" ..
                   "Use /inv all to see available tools.",
            mode = "overlay",
            title = "Equip"
        }
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

        return {
            text = table.concat(lines, "\n"),
            mode = "overlay",
            title = "Equip"
        }
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
        return {
            text = "Usage: /unequip <qualified_name>\n" ..
                   "Example: /unequip holler:sample\n" ..
                   "Use /inv to see equipped tools.",
            mode = "overlay",
            title = "Unequip"
        }
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

        return {
            text = table.concat(lines, "\n"),
            mode = "overlay",
            title = "Unequip"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /bring <artifact_id> as <role> - Bind artifact to room
--------------------------------------------------------------------------------

function M.bring(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /bring <artifact_id> as <role>\n" ..
                   "Example: /bring abc123 as reference",
            mode = "overlay",
            title = "Bring"
        }
    end

    -- Parse: <artifact_id> as <role>
    local artifact_id, role = args:match("^%s*(%S+)%s+as%s+(.+)%s*$")

    if not artifact_id or not role then
        return {
            text = "Usage: /bring <artifact_id> as <role>\n" ..
                   "Example: /bring abc123 as reference",
            mode = "overlay",
            title = "Bring"
        }
    end

    role = role:match("^%s*(.-)%s*$")  -- trim

    local result = tools.bring(artifact_id, role)

    if not result then
        return {
            text = "Error: Could not bind asset",
            mode = "notification"
        }
    end

    if result.success then
        return {
            text = string.format("Bound '%s' as '%s'", artifact_id, role),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /drop <role> - Unbind asset from room
--------------------------------------------------------------------------------

function M.drop(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /drop <role>\n" ..
                   "Example: /drop reference",
            mode = "overlay",
            title = "Drop"
        }
    end

    local role = args:match("^%s*(.-)%s*$")  -- trim

    local result = tools.drop_asset(role)

    if not result then
        return {
            text = "Error: Could not unbind asset",
            mode = "notification"
        }
    end

    if result.success then
        return {
            text = string.format("Unbound '%s'", role),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /examine <role> - Inspect a bound asset
--------------------------------------------------------------------------------

function M.examine(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /examine <role>\n" ..
                   "Example: /examine reference",
            mode = "overlay",
            title = "Examine"
        }
    end

    local role = args:match("^%s*(.-)%s*$")  -- trim

    local result = tools.examine(role)

    if not result then
        return {
            text = "Error: Could not examine asset",
            mode = "notification"
        }
    end

    if result.error then
        return {
            text = string.format("Error: %s", result.error),
            mode = "notification"
        }
    end

    if not result.asset then
        return {
            text = string.format("No asset bound as '%s'", role),
            mode = "notification"
        }
    end

    local asset = result.asset
    local lines = {}

    table.insert(lines, string.format("=== %s ===", asset.role))
    table.insert(lines, string.format("Artifact: %s", asset.artifact_id))

    if asset.notes then
        table.insert(lines, string.format("Notes: %s", asset.notes))
    end

    table.insert(lines, string.format("Bound by %s at %s",
        asset.bound_by or "unknown",
        asset.bound_at or "unknown"))

    return {
        text = table.concat(lines, "\n"),
        mode = "overlay",
        title = asset.role
    }
end

return M
