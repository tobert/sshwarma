--- commands/inventory.lua - Inventory and equipment command handlers
---
--- Containment commands (what's IN containers):
---   inv      - View contents of a container (me, room, shared, @agent)
---   take     - Copy a thing into your inventory (CoW)
---   drop     - Move a thing from your inventory to current room
---   destroy  - Delete a thing (must specify owner)
---
--- Equipment commands (what's ACTIVE):
---   equip    - Equip things to user or room with optional slot
---   unequip  - Unequip things from user or room
---
--- Slot types:
---   nil (no slot) - General availability (visible to LLM)
---   command:*     - Slash command binding (e.g., command:fish)
---   hook:wrap     - Context composition hook
---   hook:background - Background execution hook

local page = require('page')
local str = require('str')
local fun = require('fun')
local util = require('util')

local M = {}

--------------------------------------------------------------------------------
-- Helpers
--------------------------------------------------------------------------------

--- Resolve target string to parent_id
--- @param target string "me", "room", "shared", or "@agent_name"
--- @return string|nil parent_id, string|nil error
local function resolve_target(target)
    if target == "me" then
        local id = tools.get_agent_thing_id()
        if id then return id, nil end
        return nil, "Not logged in"
    elseif target == "room" then
        local id = util.get_room_id()
        if id then return id, nil end
        return nil, "Not in a room"
    elseif target == "shared" or target == "world" then
        return "shared", nil
    elseif target:match("^@") then
        local agent_name = target:sub(2)
        return "agent_" .. agent_name, nil
    else
        return nil, "Invalid target: use me, room, shared, or @agent"
    end
end

--- Format equipped item line (for equipment display)
local function format_equipped_line(item)
    local status = item.available and "+" or "o"
    local qname = item.qualified_name or item.name
    local slot_info = item.slot and (" [" .. item.slot .. "]") or ""
    return string.format("  %s %s%s", status, qname, slot_info)
end

--------------------------------------------------------------------------------
-- /inv [target] - View contents of a container
--
-- target: (empty)=me, room, shared, @agent
--------------------------------------------------------------------------------

function M.inv(args)
    local target = args and args:match("^%s*(%S+)") or "me"

    local parent_id, err = resolve_target(target)
    if not parent_id then
        return { text = "Error: " .. err, mode = "notification" }
    end

    local title
    if target == "me" then
        title = "Your Inventory"
    elseif target == "room" then
        title = "Room Contents"
    elseif target == "shared" or target == "world" then
        title = "Shared Resources"
    else
        title = string.format("%s's Inventory", target)
    end

    local children = tools.things_children(parent_id) or {}
    local lines = {title .. ":"}

    if #children == 0 then
        table.insert(lines, "  (empty)")
    else
        fun.iter(children):each(function(_, thing)
            local icon = thing.kind == "container" and "[+]" or " - "
            local name = thing.qualified_name or thing.name
            table.insert(lines, string.format("  %s %s", icon, name))
        end)
    end

    page.show("Inventory", table.concat(lines, "\n"))
    return {}
end

-- Alias
M.inventory = M.inv

--------------------------------------------------------------------------------
-- /take <thing> - Copy a thing into your inventory (CoW)
--------------------------------------------------------------------------------

function M.take(args)
    if not args or args:match("^%s*$") then
        return { text = "Usage: /take <thing>", mode = "notification" }
    end

    local thing_name = args:match("^%s*(%S+)")

    -- Try to find the thing by qualified name first
    local thing = tools.things_get(thing_name)

    if not thing then
        -- Try pattern search
        local matches = tools.things_find(thing_name) or {}
        if #matches == 1 then
            thing = matches[1]
        elseif #matches > 1 then
            return { text = string.format("Ambiguous: %d matches for '%s'", #matches, thing_name), mode = "notification" }
        else
            return { text = "Not found: " .. thing_name, mode = "notification" }
        end
    end

    local my_thing_id = tools.get_agent_thing_id()
    if not my_thing_id then
        return { text = "Error: not logged in", mode = "notification" }
    end

    local copy = tools.thing_copy(thing.id, my_thing_id)
    if copy then
        return { text = "Took " .. (thing.qualified_name or thing.name), mode = "notification" }
    else
        return { text = "Failed to take " .. thing_name, mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /drop <thing> - Move a thing from your inventory to current room
--------------------------------------------------------------------------------

function M.drop(args)
    if not args or args:match("^%s*$") then
        return { text = "Usage: /drop <thing>", mode = "notification" }
    end

    local thing_name = args:match("^%s*(%S+)")
    local my_thing_id = tools.get_agent_thing_id()
    local room_id = util.get_room_id()

    if not my_thing_id then
        return { text = "Error: not logged in", mode = "notification" }
    end
    if not room_id then
        return { text = "Error: not in a room", mode = "notification" }
    end

    -- Find thing in my inventory
    local my_things = tools.things_children(my_thing_id) or {}
    local thing = nil
    for _, t in ipairs(my_things) do
        if t.name == thing_name or t.qualified_name == thing_name then
            thing = t
            break
        end
    end

    if not thing then
        return { text = "Not in your inventory: " .. thing_name, mode = "notification" }
    end

    if tools.thing_move(thing.id, room_id) then
        return { text = "Dropped " .. (thing.qualified_name or thing.name), mode = "notification" }
    else
        return { text = "Failed to drop " .. thing_name, mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /destroy <owner>:<thing> - Delete a thing
-- Must specify owner to prevent accidents
--------------------------------------------------------------------------------

function M.destroy(args)
    if not args or args:match("^%s*$") then
        return { text = "Usage: /destroy <owner>:<thing>", mode = "notification" }
    end

    local owner, thing_name = args:match("^%s*([^:]+):(.+)%s*$")
    if not owner or not thing_name then
        return { text = "Must specify owner:thing (e.g., me:old-note)", mode = "notification" }
    end

    -- Resolve owner to parent_id
    local parent_id, err = resolve_target(owner)
    if not parent_id then
        return { text = "Error: " .. err, mode = "notification" }
    end

    -- Find thing under owner
    local children = tools.things_children(parent_id) or {}
    local thing = nil
    for _, t in ipairs(children) do
        if t.name == thing_name or t.qualified_name == thing_name then
            thing = t
            break
        end
    end

    if not thing then
        return { text = "Not found: " .. owner .. ":" .. thing_name, mode = "notification" }
    end

    if tools.thing_delete(thing.id) then
        return { text = "Destroyed " .. thing_name, mode = "notification" }
    else
        return { text = "Failed to destroy " .. thing_name, mode = "notification" }
    end
end

--------------------------------------------------------------------------------
-- /equip <context> [slot] <pattern>
--
-- Examples:
--   /equip me sshwarma:*           - Equip all sshwarma tools to user
--   /equip room holler:sample      - Equip tool to room
--   /equip me command:fish atobey:fish  - Equip with specific slot
--   /equip @qwenl holler:*         - Equip holler tools to agent qwenl
--------------------------------------------------------------------------------

function M.equip(args)
    if not args or args:match("^%s*$") then
        page.show("Equip", [[
Usage: /equip <context> [slot] <pattern>

Context: me | room | @agent_name
Pattern: qualified name or glob (e.g., sshwarma:*, holler:sample)
Slot (optional): command:*, hook:wrap, hook:background

Examples:
  /equip me sshwarma:*              Equip all sshwarma tools to yourself
  /equip room holler:sample         Equip tool to room (for LLM)
  /equip @qwenl holler:*            Equip holler tools to agent qwenl
  /equip me command:fish atobey:fish   Bind /fish to atobey:fish
  /equip room hook:wrap myns:wrap   Add wrap hook to room
]])
        return {}
    end

    local parts = str.split(args, "%s+")
    local context = parts[1]  -- 'me', 'room', or '@agent_name'

    -- Parse context: me, room, or @agent_name
    local context_type, agent_name
    if context == "me" then
        context_type = "agent"
    elseif context == "room" then
        context_type = "room"
    elseif context:match("^@") then
        context_type = "agent"
        agent_name = context:sub(2)
    else
        return { text = "Error: context must be 'me', 'room', or '@agent_name'", mode = "notification" }
    end

    -- Parse remaining args: could be [slot] <pattern> or just <pattern>
    local slot, pattern
    if #parts >= 3 then
        local maybe_slot = parts[2]
        if maybe_slot:match("^command:") or maybe_slot:match("^hook:") or maybe_slot:match("^hotkey:") then
            slot = maybe_slot
            pattern = parts[3]
        else
            pattern = parts[2]
        end
    elseif #parts == 2 then
        pattern = parts[2]
    else
        return { text = "Error: missing pattern argument", mode = "notification" }
    end

    -- Parse slot config (e.g., hook:background:1000 -> slot=hook:background, config)
    local config = nil
    if slot then
        local base, extra = slot:match("^([^:]+:[^:]+):(.+)$")
        if base then
            slot = base
            config = string.format('{"interval_ms":%s}', extra)
        end
    end

    -- Expand pattern to matching things
    local things = tools.things_match(pattern) or {}
    if #things == 0 then
        return { text = string.format("No things match pattern: %s", pattern), mode = "notification" }
    end

    -- Get target ID
    local target_id, display_name
    if context_type == "agent" then
        if agent_name then
            local agent = tools.get_agent(agent_name)
            if not agent then
                return { text = string.format("Error: agent '%s' not found", agent_name), mode = "notification" }
            end
            target_id = agent.id
            display_name = "@" .. agent_name
        else
            target_id = util.get_agent_id()
            if not target_id then
                return { text = "Error: not logged in", mode = "notification" }
            end
            display_name = "me"
        end
    else
        target_id = util.get_room_id()
        if not target_id then
            return { text = "Error: not in a room", mode = "notification" }
        end
        display_name = "room"
    end

    -- Equip each matching thing
    local equipped = {}
    fun.iter(things):enumerate():each(function(i, thing)
        local actual_slot = slot or thing.default_slot
        local success
        if context_type == "agent" then
            success = tools.agent_equip(target_id, thing.id, actual_slot, config, i)
        else
            success = tools.room_equip(target_id, thing.id, actual_slot, i)
        end
        if success then
            table.insert(equipped, thing.qualified_name or thing.name)
        end
    end)

    if #equipped == 0 then
        return { text = "Error: could not equip any items", mode = "notification" }
    end

    local lines = {string.format("Equipped %d item(s) to %s:", #equipped, display_name)}
    fun.iter(equipped):each(function(_, name)
        table.insert(lines, "  " .. name)
    end)

    page.show("Equip", table.concat(lines, "\n"))
    return {}
end

--------------------------------------------------------------------------------
-- /unequip <context> [slot] <pattern>
--
-- Examples:
--   /unequip me sshwarma:*         - Unequip all sshwarma tools from user
--   /unequip room holler:sample    - Unequip tool from room
--   /unequip @qwenl holler:*       - Unequip holler tools from agent qwenl
--------------------------------------------------------------------------------

function M.unequip(args)
    if not args or args:match("^%s*$") then
        page.show("Unequip", [[
Usage: /unequip <context> [slot] <pattern>

Context: me | room | @agent_name
Pattern: qualified name or glob
Slot (optional): command:*, hook:wrap, hook:background

Examples:
  /unequip me sshwarma:*            Unequip all sshwarma tools
  /unequip room holler:sample       Unequip from room
  /unequip @qwenl holler:*          Unequip holler tools from agent qwenl
]])
        return {}
    end

    local parts = str.split(args, "%s+")
    local context = parts[1]

    -- Parse context: me, room, or @agent_name
    local context_type, agent_name
    if context == "me" then
        context_type = "agent"
    elseif context == "room" then
        context_type = "room"
    elseif context:match("^@") then
        context_type = "agent"
        agent_name = context:sub(2)
    else
        return { text = "Error: context must be 'me', 'room', or '@agent_name'", mode = "notification" }
    end

    -- Parse remaining args: could be [slot] <pattern> or just <pattern>
    local slot, pattern
    if #parts >= 3 then
        local maybe_slot = parts[2]
        if maybe_slot:match("^command:") or maybe_slot:match("^hook:") or maybe_slot:match("^hotkey:") then
            slot = maybe_slot
            pattern = parts[3]
        else
            pattern = parts[2]
        end
    elseif #parts == 2 then
        pattern = parts[2]
    else
        return { text = "Error: missing pattern argument", mode = "notification" }
    end

    -- Expand pattern to matching things
    local things = tools.things_match(pattern) or {}
    if #things == 0 then
        return { text = string.format("No things match pattern: %s", pattern), mode = "notification" }
    end

    -- Get target ID
    local target_id, display_name
    if context_type == "agent" then
        if agent_name then
            local agent = tools.get_agent(agent_name)
            if not agent then
                return { text = string.format("Error: agent '%s' not found", agent_name), mode = "notification" }
            end
            target_id = agent.id
            display_name = "@" .. agent_name
        else
            target_id = util.get_agent_id()
            if not target_id then
                return { text = "Error: not logged in", mode = "notification" }
            end
            display_name = "me"
        end
    else
        target_id = util.get_room_id()
        if not target_id then
            return { text = "Error: not in a room", mode = "notification" }
        end
        display_name = "room"
    end

    -- Unequip each matching thing
    local unequipped = {}
    fun.iter(things):each(function(_, thing)
        local success
        if context_type == "agent" then
            success = tools.agent_unequip(target_id, thing.id, slot)
        else
            success = tools.room_unequip(target_id, thing.id, slot)
        end
        if success then
            table.insert(unequipped, thing.qualified_name or thing.name)
        end
    end)

    if #unequipped == 0 then
        return { text = "Error: could not unequip any items", mode = "notification" }
    end

    local lines = {string.format("Unequipped %d item(s) from %s:", #unequipped, display_name)}
    fun.iter(unequipped):each(function(_, name)
        table.insert(lines, "  " .. name)
    end)

    page.show("Unequip", table.concat(lines, "\n"))
    return {}
end

return M
