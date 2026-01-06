-- Navigation command handlers for sshwarma
--
-- Commands for moving between rooms and viewing room state.
-- Each handler receives args (string) and returns {text, mode, title?}

local M = {}

-- Direction arrow glyphs for exits display
local DIR_GLYPHS = {
    north = "^",
    south = "v",
    east  = ">",
    west  = "<",
    up    = "^",
    down  = "v",
    n = "^", s = "v", e = ">", w = "<", u = "^", d = "v"
}

-- /rooms - List all rooms
function M.rooms(args)
    local rooms = tools.rooms()

    if not rooms or #rooms == 0 then
        return {
            text = "No rooms yet. Use /create <name> to start one.",
            mode = "notification"
        }
    end

    local lines = {"Rooms:\r\n"}
    for _, room in ipairs(rooms) do
        local desc = room.description or ""
        if #desc > 40 then
            desc = desc:sub(1, 37) .. "..."
        end
        local user_plural = room.user_count == 1 and "user" or "users"
        table.insert(lines, string.format("  %s ... %d %s%s\r\n",
            room.name,
            room.user_count,
            user_plural,
            desc ~= "" and string.format(" (%s)", desc) or ""))
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Rooms"
    }
end

-- /join <room> - Enter a room
function M.join(args)
    local room_name = args:match("^%s*(.-)%s*$")

    if room_name == "" then
        return {
            text = "Usage: /join <room>",
            mode = "notification"
        }
    end

    local result = tools.join(room_name)

    if result.success then
        -- Brief notification - chat area stays visible
        local room = result.room
        local msg = string.format("Joined %s", room and room.name or room_name)
        if room and room.vibe then
            msg = msg .. string.format(" (%s)", room.vibe)
        end
        return {
            text = msg,
            mode = "notification"
        }
    else
        local err = result.error or "unknown error"
        if err:find("not found") or err:find("doesn't exist") then
            return {
                text = string.format("No room named '%s'. Use /create %s to make one.",
                    room_name, room_name),
                mode = "notification"
            }
        end
        return {
            text = string.format("Error: %s", err),
            mode = "notification"
        }
    end
end

-- /leave - Return to lobby
function M.leave(args)
    local result = tools.leave()

    if result.success then
        return {
            text = "You are now in the lobby.",
            mode = "notification"
        }
    else
        local err = result.error or "unknown error"
        if err:find("not in a room") then
            return {
                text = "You're already in the lobby.",
                mode = "notification"
            }
        end
        return {
            text = string.format("Error: %s", err),
            mode = "notification"
        }
    end
end

-- /go <direction> - Navigate through exit
function M.go(args)
    local direction = args:match("^%s*(.-)%s*$")

    if direction == "" then
        return {
            text = "Usage: /go <direction>",
            mode = "notification"
        }
    end

    local result = tools.go(direction)

    if result.success then
        return format_room_entry(result.room)
    else
        local err = result.error or "unknown error"
        if err:find("no exit") or err:find("No exit") then
            return {
                text = string.format("No exit '%s' from here. Use /exits to see available exits.",
                    direction),
                mode = "notification"
            }
        end
        return {
            text = string.format("Error: %s", err),
            mode = "notification"
        }
    end
end

-- /exits - List room exits
function M.exits(args)
    local exits = tools.exits()

    if not exits or next(exits) == nil then
        return {
            text = "No exits. Use /dig <direction> to <room> to create one.",
            mode = "notification"
        }
    end

    local lines = {"Exits:\r\n"}
    for dir, dest in pairs(exits) do
        local glyph = DIR_GLYPHS[dir] or "-"
        table.insert(lines, string.format("  %s %s -> %s\r\n", glyph, dir, dest))
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Exits"
    }
end

-- /look [target] - Room summary
function M.look(args)
    local target = args:match("^%s*(.-)%s*$")

    if target ~= "" then
        -- Looking at something specific (future: examine items, exits, etc.)
        return {
            text = string.format("You look at '%s'. (detailed inspection coming soon)", target),
            mode = "notification"
        }
    end

    local room = tools.look()

    if not room.room then
        return {
            text = "=== Lobby ===\r\n\r\nYou're in the lobby. Use /rooms to see rooms, /join <room> to enter one.",
            mode = "overlay",
            title = "Lobby"
        }
    end

    return format_look_output(room)
end

-- /who - Who's in room
function M.who(args)
    local participants = tools.who()
    local room = tools.look()
    local room_name = room.room or "lobby"

    if not participants or #participants == 0 then
        return {
            text = string.format("In %s: (empty)", room_name),
            mode = "notification"
        }
    end

    local users = {}
    local models = {}

    for _, p in ipairs(participants) do
        if p.is_model then
            table.insert(models, string.format("%s %s", p.glyph or "o", p.name))
        else
            table.insert(users, string.format("%s %s", p.glyph or "o", p.name))
        end
    end

    local lines = {string.format("=== %s ===\r\n\r\n", room_name)}

    if #users > 0 then
        table.insert(lines, "Users:\r\n")
        for _, u in ipairs(users) do
            table.insert(lines, string.format("  %s\r\n", u))
        end
    end

    if #models > 0 then
        if #users > 0 then
            table.insert(lines, "\r\n")
        end
        table.insert(lines, "Models:\r\n")
        for _, m in ipairs(models) do
            table.insert(lines, string.format("  %s\r\n", m))
        end
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Who"
    }
end

-- Helper: Format room data for /look output
function format_look_output(room)
    local lines = {string.format("=== %s ===\r\n", room.room)}

    if room.description then
        table.insert(lines, string.format("\r\n%s\r\n", room.description))
    end

    if room.vibe then
        table.insert(lines, string.format("\r\nVibe: %s\r\n", room.vibe))
    end

    -- Users
    if room.users and #room.users > 0 then
        table.insert(lines, string.format("\r\nUsers: %s\r\n", table.concat(room.users, ", ")))
    end

    -- Models
    if room.models and #room.models > 0 then
        table.insert(lines, string.format("Models: %s\r\n", table.concat(room.models, ", ")))
    end

    -- Exits
    if room.exits and next(room.exits) then
        local exit_parts = {}
        for dir, dest in pairs(room.exits) do
            local glyph = DIR_GLYPHS[dir] or "-"
            table.insert(exit_parts, string.format("%s %s", glyph, dir))
        end
        table.insert(lines, string.format("\r\nExits: %s\r\n", table.concat(exit_parts, ", ")))
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = room.room
    }
end

-- Helper: Format room entry message (after join/go)
function format_room_entry(room)
    if not room then
        return {
            text = "Entered room.",
            mode = "notification"
        }
    end

    local lines = {string.format("=== %s ===\r\n", room.name)}

    if room.description then
        table.insert(lines, string.format("\r\n%s\r\n", room.description))
    end

    if room.vibe then
        table.insert(lines, string.format("\r\nVibe: %s\r\n", room.vibe))
    end

    if room.users and #room.users > 0 then
        table.insert(lines, string.format("\r\nUsers: %s\r\n", table.concat(room.users, ", ")))
    end

    if room.models and #room.models > 0 then
        table.insert(lines, string.format("Models: %s\r\n", table.concat(room.models, ", ")))
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = room.name
    }
end

return M
