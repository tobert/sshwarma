--- commands/room.lua - Room management command handlers
---
--- Commands for creating, modifying, and managing room properties.
--- Uses util for direction tables.

local page = require('page')
local util = require('util')

local M = {}

--------------------------------------------------------------------------------
-- /create <name> - Create new room
--------------------------------------------------------------------------------

function M.create(args)
    local room_name = args:match("^%s*(.-)%s*$")

    if room_name == "" then
        return { text = "Usage: /create <name>", mode = "notification" }
    end

    local result = tools.create(room_name)

    if result.success then
        local lines = {string.format("Created room '%s'.\n\n", room_name)}

        if result.room then
            table.insert(lines, string.format("=== %s ===\n", result.room.name))
            if result.room.vibe then
                table.insert(lines, string.format("\nVibe: %s\n", result.room.vibe))
            end
        end

        page.show(room_name, table.concat(lines))
        return {}
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /fork [name] - Fork current room (inherits vibe, assets)
--------------------------------------------------------------------------------

function M.fork(args)
    local new_name = args:match("^%s*(.-)%s*$")

    if new_name == "" then
        return { text = "Usage: /fork <new_room_name>", mode = "notification" }
    end

    local result = tools.fork(new_name)

    if result.success then
        local lines = {
            string.format("Forked room as '%s'.\n", new_name),
            "Inherited: vibe, tags, assets.\n\n"
        }

        if result.room then
            table.insert(lines, string.format("=== %s ===\n", result.room.name))
            if result.room.vibe then
                table.insert(lines, string.format("\nVibe: %s\n", result.room.vibe))
            end
        end

        page.show(new_name, table.concat(lines))
        return {}
    else
        local err = result.error or "unknown error"
        if err:find("not in a room") then
            return { text = "You need to be in a room to fork.", mode = "notification" }
        end
        return { text = string.format("Error: %s", err), mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /vibe [text] - Get or set room vibe
--------------------------------------------------------------------------------

function M.vibe(args)
    local text = args:match("^%s*(.-)%s*$")

    if text == "" then
        local vibe = tools.vibe()
        if vibe then
            return { text = string.format("Vibe: %s", vibe), mode = "notification" }
        else
            return { text = "No vibe set. Use /vibe <text> to set one.", mode = "notification" }
        end
    else
        local result = tools.set_vibe(text)
        if result.success then
            return { text = string.format("Vibe set: %s", text), mode = "notification" }
        else
            local err = result.error or "unknown error"
            if err:find("not in a room") then
                return { text = "You need to be in a room to set vibe.", mode = "notification" }
            end
            return { text = string.format("Error: %s", err), mode = "notification" }
        end
    end
end

--------------------------------------------------------------------------------
-- /portal <direction> <room> - Create bidirectional exit to another room
--------------------------------------------------------------------------------

function M.portal(args)
    -- Parse: <direction> <room> or <direction> to <room>
    local direction, target_room = args:match("^%s*(%S+)%s+to%s+(.-)%s*$")
    if not direction then
        direction, target_room = args:match("^%s*(%S+)%s+(.-)%s*$")
    end

    if not direction or not target_room or target_room == "" then
        return {
            text = "Usage: /portal <direction> <room>\nExample: /portal north workshop\n\nCreates bidirectional exits between rooms.",
            mode = "notification"
        }
    end

    -- Normalize direction
    direction = util.DIR_NORMALIZE[direction:lower()] or direction:lower()

    local result = tools.dig(direction, target_room)

    if result.success then
        local reverse = result.reverse or util.DIR_OPPOSITE[direction]
        if reverse then
            return { text = string.format("Created portal: %s <-> %s", direction, target_room), mode = "notification" }
        else
            return { text = string.format("Created exit: %s -> %s", direction, target_room), mode = "notification" }
        end
    else
        local err = result.error or "unknown error"
        if err:find("not in a room") then
            return { text = "You need to be in a room to create portals.", mode = "notification" }
        elseif err:find("not found") or err:find("doesn't exist") then
            return {
                text = string.format("Target room '%s' doesn't exist. Create it first with /create %s",
                    target_room, target_room),
                mode = "notification"
            }
        end
        return { text = string.format("Error: %s", err), mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /nav [on|off] - Toggle model navigation for room
-- TODO: implement actual nav toggle when ops::set_room_navigation is exposed
--------------------------------------------------------------------------------

function M.nav(args)
    local setting = args:match("^%s*(.-)%s*$"):lower()

    if setting == "" then
        page.show("Navigation", "Model navigation setting.\n\nUse /nav on or /nav off to control whether models can navigate between rooms.\n\nNote: Full nav control coming soon.")
        return {}
    elseif setting == "on" then
        return {
            text = "Model navigation enabled.\nModels can now use navigation tools (join, leave, go).",
            mode = "notification"
        }
    elseif setting == "off" then
        return {
            text = "Model navigation disabled.\nModels can no longer navigate between rooms.",
            mode = "notification"
        }
    else
        return { text = "Usage: /nav [on|off]", mode = "notification" }
    end
end

return M
