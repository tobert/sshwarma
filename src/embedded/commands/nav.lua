--- commands/nav.lua - Navigation command handlers
---
--- Commands for moving between rooms and viewing room state.
--- Uses luafun for iteration and util for direction tables.

local page = require('page')
local fun = require('fun')
local util = require('util')

local M = {}

--------------------------------------------------------------------------------
-- /rooms - List all rooms
--------------------------------------------------------------------------------

function M.rooms(args)
    local rooms = tools.rooms()

    if not rooms or #rooms == 0 then
        return {
            text = "No rooms yet. Use /create <name> to start one.",
            mode = "notification"
        }
    end

    local lines = {"Rooms:\n"}

    fun.iter(rooms):each(function(_, room)
        local desc = room.description or ""
        if #desc > 40 then
            desc = desc:sub(1, 37) .. "..."
        end
        local user_plural = room.user_count == 1 and "user" or "users"
        local desc_part = desc ~= "" and string.format(" (%s)", desc) or ""
        table.insert(lines, string.format("  %s ... %d %s%s\n",
            room.name, room.user_count, user_plural, desc_part))
    end)

    page.show("Rooms", table.concat(lines))
    return {}
end

--------------------------------------------------------------------------------
-- /join <room> - Enter a room
--------------------------------------------------------------------------------

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
        local room = result.room
        local msg = string.format("Joined %s", room and room.name or room_name)
        if room and room.vibe then
            msg = msg .. string.format(" (%s)", room.vibe)
        end
        return { text = msg, mode = "notification" }
    else
        local err = result.error or "unknown error"
        if err:find("not found") or err:find("doesn't exist") then
            return {
                text = string.format("No room named '%s'. Use /create %s to make one.",
                    room_name, room_name),
                mode = "notification"
            }
        end
        return { text = string.format("Error: %s", err), mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /leave - Return to lobby
--------------------------------------------------------------------------------

function M.leave(args)
    local result = tools.leave()

    if result.success then
        return { text = "You are now in the lobby.", mode = "notification" }
    else
        local err = result.error or "unknown error"
        if err:find("not in a room") then
            return { text = "You're already in the lobby.", mode = "notification" }
        end
        return { text = string.format("Error: %s", err), mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /go <direction> - Navigate through exit
--------------------------------------------------------------------------------

function M.go(args)
    local direction = args:match("^%s*(.-)%s*$")

    if direction == "" then
        return { text = "Usage: /go <direction>", mode = "notification" }
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
        return { text = string.format("Error: %s", err), mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /exits - List room exits
--------------------------------------------------------------------------------

function M.exits(args)
    local exits = tools.exits()

    if not exits or next(exits) == nil then
        return {
            text = "No exits. Use /dig <direction> to <room> to create one.",
            mode = "notification"
        }
    end

    local lines = {"Exits:\n"}

    fun.iter(exits):each(function(dir, dest)
        local glyph = util.DIR_GLYPHS[dir] or "-"
        table.insert(lines, string.format("  %s %s -> %s\n", glyph, dir, dest))
    end)

    page.show("Exits", table.concat(lines))
    return {}
end

--------------------------------------------------------------------------------
-- /look [target] - Room summary
--------------------------------------------------------------------------------

function M.look(args)
    local target = args:match("^%s*(.-)%s*$")

    if target ~= "" then
        return {
            text = string.format("You look at '%s'. (detailed inspection coming soon)", target),
            mode = "notification"
        }
    end

    local room = tools.look()

    if not room.room then
        page.show("Lobby", "=== Lobby ===\n\nYou're in the lobby. Use /rooms to see rooms, /join <room> to enter one.")
        return {}
    end

    return format_look_output(room)
end

--------------------------------------------------------------------------------
-- /who - Who's in room
--------------------------------------------------------------------------------

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

    -- Partition into users and models using luafun
    local users = fun.iter(participants)
        :filter(function(_, p) return not p.is_model end)
        :map(function(_, p) return string.format("%s %s", p.glyph or "o", p.name) end)
        :totable()

    local models = fun.iter(participants)
        :filter(function(_, p) return p.is_model end)
        :map(function(_, p) return string.format("%s %s", p.glyph or "o", p.name) end)
        :totable()

    local lines = {string.format("=== %s ===\n\n", room_name)}

    if #users > 0 then
        table.insert(lines, "Users:\n")
        fun.iter(users):each(function(_, u)
            table.insert(lines, string.format("  %s\n", u))
        end)
    end

    if #models > 0 then
        if #users > 0 then table.insert(lines, "\n") end
        table.insert(lines, "Models:\n")
        fun.iter(models):each(function(_, m)
            table.insert(lines, string.format("  %s\n", m))
        end)
    end

    page.show("Who", table.concat(lines))
    return {}
end

--------------------------------------------------------------------------------
-- Helpers
--------------------------------------------------------------------------------

--- Format room data for /look output
function format_look_output(room)
    local lines = {string.format("=== %s ===\n", room.room)}

    if room.description then
        table.insert(lines, string.format("\n%s\n", room.description))
    end

    if room.vibe then
        table.insert(lines, string.format("\nVibe: %s\n", room.vibe))
    end

    if room.users and #room.users > 0 then
        table.insert(lines, string.format("\nUsers: %s\n", table.concat(room.users, ", ")))
    end

    if room.models and #room.models > 0 then
        table.insert(lines, string.format("Models: %s\n", table.concat(room.models, ", ")))
    end

    if room.exits and next(room.exits) then
        local exit_parts = fun.iter(room.exits)
            :map(function(dir, _)
                local glyph = util.DIR_GLYPHS[dir] or "-"
                return string.format("%s %s", glyph, dir)
            end)
            :totable()
        table.insert(lines, string.format("\nExits: %s\n", table.concat(exit_parts, ", ")))
    end

    page.show(room.room, table.concat(lines))
    return {}
end

--- Format room entry message (after join/go)
function format_room_entry(room)
    if not room then
        return { text = "Entered room.", mode = "notification" }
    end

    local lines = {string.format("=== %s ===\n", room.name)}

    if room.description then
        table.insert(lines, string.format("\n%s\n", room.description))
    end

    if room.vibe then
        table.insert(lines, string.format("\nVibe: %s\n", room.vibe))
    end

    if room.users and #room.users > 0 then
        table.insert(lines, string.format("\nUsers: %s\n", table.concat(room.users, ", ")))
    end

    if room.models and #room.models > 0 then
        table.insert(lines, string.format("Models: %s\n", table.concat(room.models, ", ")))
    end

    page.show(room.name, table.concat(lines))
    return {}
end

return M
