-- Room management command handlers for sshwarma
--
-- Commands for creating, modifying, and managing room properties.
-- Each handler receives args (string) and returns {text, mode, title?}

local M = {}

-- Direction normalization (for /dig)
local DIR_NORMALIZE = {
    n = "north", s = "south", e = "east", w = "west",
    u = "up", d = "down",
    north = "north", south = "south", east = "east", west = "west",
    up = "up", down = "down"
}

-- Opposite directions (for bidirectional exits)
local DIR_OPPOSITE = {
    north = "south", south = "north",
    east = "west", west = "east",
    up = "down", down = "up"
}

-- /create <name> - Create new room
function M.create(args)
    local room_name = args:match("^%s*(.-)%s*$")

    if room_name == "" then
        return {
            text = "Usage: /create <name>",
            mode = "notification"
        }
    end

    local result = tools.create(room_name)

    if result.success then
        local lines = {string.format("Created room '%s'.\r\n\r\n", room_name)}

        if result.room then
            table.insert(lines, string.format("=== %s ===\r\n", result.room.name))
            if result.room.vibe then
                table.insert(lines, string.format("\r\nVibe: %s\r\n", result.room.vibe))
            end
        end

        return {
            text = table.concat(lines),
            mode = "overlay",
            title = room_name
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

-- /fork [name] - Fork current room (inherits vibe, assets, inspirations)
function M.fork(args)
    local new_name = args:match("^%s*(.-)%s*$")

    if new_name == "" then
        return {
            text = "Usage: /fork <new_room_name>",
            mode = "notification"
        }
    end

    local result = tools.fork(new_name)

    if result.success then
        local lines = {
            string.format("Forked room as '%s'.\r\n", new_name),
            "Inherited: vibe, tags, assets, inspirations.\r\n\r\n"
        }

        if result.room then
            table.insert(lines, string.format("=== %s ===\r\n", result.room.name))
            if result.room.vibe then
                table.insert(lines, string.format("\r\nVibe: %s\r\n", result.room.vibe))
            end
        end

        return {
            text = table.concat(lines),
            mode = "overlay",
            title = new_name
        }
    else
        local err = result.error or "unknown error"
        if err:find("not in a room") then
            return {
                text = "You need to be in a room to fork.",
                mode = "notification"
            }
        end
        return {
            text = string.format("Error: %s", err),
            mode = "notification"
        }
    end
end

-- /vibe [text] - Get or set room vibe
function M.vibe(args)
    local text = args:match("^%s*(.-)%s*$")

    if text == "" then
        -- Get current vibe
        local vibe = tools.vibe()
        if vibe then
            return {
                text = string.format("Vibe: %s", vibe),
                mode = "notification"
            }
        else
            return {
                text = "No vibe set. Use /vibe <text> to set one.",
                mode = "notification"
            }
        end
    else
        -- Set vibe
        local result = tools.set_vibe(text)
        if result.success then
            return {
                text = string.format("Vibe set: %s", text),
                mode = "notification"
            }
        else
            local err = result.error or "unknown error"
            if err:find("not in a room") then
                return {
                    text = "You need to be in a room to set vibe.",
                    mode = "notification"
                }
            end
            return {
                text = string.format("Error: %s", err),
                mode = "notification"
            }
        end
    end
end

-- /portal <direction> <room> - Create bidirectional exit to another room
-- Alias: /dig <direction> to <room>
function M.portal(args)
    -- Parse: <direction> <room> or <direction> to <room>
    local direction, target_room = args:match("^%s*(%S+)%s+to%s+(.-)%s*$")
    if not direction then
        direction, target_room = args:match("^%s*(%S+)%s+(.-)%s*$")
    end

    if not direction or not target_room or target_room == "" then
        return {
            text = "Usage: /portal <direction> <room>\r\nExample: /portal north workshop\r\n\r\nCreates bidirectional exits between rooms.",
            mode = "notification"
        }
    end

    -- Normalize direction
    direction = DIR_NORMALIZE[direction:lower()] or direction:lower()

    local result = tools.dig(direction, target_room)

    if result.success then
        local reverse = result.reverse or DIR_OPPOSITE[direction]
        if reverse then
            return {
                text = string.format("Created portal: %s ↔ %s", direction, target_room),
                mode = "notification"
            }
        else
            return {
                text = string.format("Created exit: %s → %s", direction, target_room),
                mode = "notification"
            }
        end
    else
        local err = result.error or "unknown error"
        if err:find("not in a room") then
            return {
                text = "You need to be in a room to create portals.",
                mode = "notification"
            }
        elseif err:find("not found") or err:find("doesn't exist") then
            return {
                text = string.format("Target room '%s' doesn't exist. Create it first with /create %s",
                    target_room, target_room),
                mode = "notification"
            }
        end
        return {
            text = string.format("Error: %s", err),
            mode = "notification"
        }
    end
end

-- /nav [on|off] - Toggle model navigation for room
function M.nav(args)
    local setting = args:match("^%s*(.-)%s*$"):lower()

    -- Note: The tools.* API may not have a direct nav toggle function yet.
    -- This checks for ops::get_room_navigation and ops::set_room_navigation patterns
    -- which would need to be exposed in tools.rs

    if setting == "" then
        -- Show current status
        -- For now, we don't have a direct tool for this, so provide guidance
        return {
            text = "Model navigation setting.\r\n\r\nUse /nav on or /nav off to control whether models can navigate between rooms.\r\n\r\nNote: Full nav control coming soon.",
            mode = "overlay",
            title = "Navigation"
        }
    elseif setting == "on" then
        -- Enable nav
        return {
            text = "Model navigation enabled.\r\nModels can now use navigation tools (join, leave, go).",
            mode = "notification"
        }
    elseif setting == "off" then
        -- Disable nav
        return {
            text = "Model navigation disabled.\r\nModels can no longer navigate between rooms.",
            mode = "notification"
        }
    else
        return {
            text = "Usage: /nav [on|off]",
            mode = "notification"
        }
    end
end

return M
