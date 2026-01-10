--- commands/inventory.lua - Inventory command handlers
---
--- Slot-based equipment management:
---   inventory/inv - List equipment (user and room)
---   equip        - Equip things to user or room with optional slot
---   unequip      - Unequip things from user or room
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

--- Format equipped item line
local function format_equipped_line(item)
    local status = item.available and "+" or "o"
    local qname = item.qualified_name or item.name
    local slot_info = item.slot and (" [" .. item.slot .. "]") or ""
    return string.format("  %s %s%s", status, qname, slot_info)
end

--------------------------------------------------------------------------------
-- /inv [me|room|all] - List equipment
--------------------------------------------------------------------------------

function M.inventory(args)
    local filter = args and args:match("^%s*(%S+)") or "all"
    local lines = {}
    local agent_id = util.get_agent_id()
    local room_id = util.get_room_id()

    -- User equipment (if showing "me" or "all")
    if filter == "me" or filter == "all" then
        table.insert(lines, "Your Equipment:")
        if agent_id then
            local user_equip = tools.get_agent_equipment(agent_id, nil) or {}
            if #user_equip > 0 then
                fun.iter(user_equip):each(function(_, item)
                    table.insert(lines, format_equipped_line(item))
                end)
            else
                table.insert(lines, "  (none)")
            end
        else
            table.insert(lines, "  (not logged in)")
        end
        table.insert(lines, "")
    end

    -- Room equipment (if showing "room" or "all")
    if filter == "room" or filter == "all" then
        table.insert(lines, "Room Equipment:")
        if room_id then
            local room_equip = tools.get_room_equipment(room_id, nil) or {}
            if #room_equip > 0 then
                fun.iter(room_equip):each(function(_, item)
                    table.insert(lines, format_equipped_line(item))
                end)
            else
                table.insert(lines, "  (none)")
            end
        else
            table.insert(lines, "  (not in a room)")
        end
        table.insert(lines, "")
    end

    -- Available (unequipped) things
    table.insert(lines, "Available to Equip:")
    local all_things = tools.things_match("*:*") or {}

    -- Collect IDs of equipped things
    local equipped_ids = {}
    if agent_id then
        fun.iter(tools.get_agent_equipment(agent_id, nil) or {}):each(function(_, item)
            equipped_ids[item.thing_id] = true
        end)
    end
    if room_id then
        fun.iter(tools.get_room_equipment(room_id, nil) or {}):each(function(_, item)
            equipped_ids[item.thing_id] = true
        end)
    end

    -- Filter to available, unequipped things
    local available = fun.iter(all_things)
        :filter(function(_, thing) return thing.available and not equipped_ids[thing.id] end)
        :totable()

    if #available > 0 then
        fun.iter(available):each(function(_, thing)
            local qname = thing.qualified_name or thing.name
            table.insert(lines, string.format("  o %s", qname))
        end)
    else
        table.insert(lines, "  (none)")
    end

    page.show("Inventory", table.concat(lines, "\n"))
    return {}
end

-- Alias
M.inv = M.inventory

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
