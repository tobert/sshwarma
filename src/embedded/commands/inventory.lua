-- sshwarma inventory command handlers
--
-- Lua command handlers for slot-based equipment management:
--   inventory/inv - List equipment (user and room)
--   equip        - Equip things to user or room with optional slot
--   unequip      - Unequip things from user or room
--
-- Slot types:
--   nil (no slot) - General availability (visible to LLM)
--   command:*     - Slash command binding (e.g., command:fish)
--   hook:wrap     - Context composition hook
--   hook:background - Background execution hook
--
-- Commands that display content use page.show() directly.

local page = require('page')
local str = require('str')
local fun = require('fun')
local M = {}

--------------------------------------------------------------------------------
-- Helper: format equipped item line
--------------------------------------------------------------------------------

local function format_equipped_line(item)
    local status = item.available and "+" or "o"
    local qname = item.qualified_name or item.name
    local slot_info = item.slot and (" [" .. item.slot .. "]") or ""
    return string.format("  %s %s%s", status, qname, slot_info)
end

--------------------------------------------------------------------------------
-- Helper: get current user agent ID
--------------------------------------------------------------------------------

local function get_agent_id()
    local session = tools.session()
    if session and session.agent_id then
        return session.agent_id  -- UUID from agents table
    end
    return nil
end

--------------------------------------------------------------------------------
-- Helper: get current room ID
--------------------------------------------------------------------------------

local function get_room_id()
    local session = tools.session()
    if session and session.room_id then
        return session.room_id  -- Use actual room ID from database
    end
    return nil
end

--------------------------------------------------------------------------------
-- /inv [me|room|all] - List equipment
--------------------------------------------------------------------------------

function M.inventory(args)
    local filter = args and args:match("^%s*(%S+)") or "all"
    local lines = {}
    local agent_id = get_agent_id()
    local room_id = get_room_id()

    -- User equipment (if showing "me" or "all")
    if filter == "me" or filter == "all" then
        table.insert(lines, "Your Equipment:")
        if agent_id then
            local user_equip = tools.get_agent_equipment(agent_id, nil) or {}
            if #user_equip > 0 then
                for _, item in ipairs(user_equip) do
                    table.insert(lines, format_equipped_line(item))
                end
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
                for _, item in ipairs(room_equip) do
                    table.insert(lines, format_equipped_line(item))
                end
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
    local equipped_ids = {}

    -- Collect IDs of equipped things
    if agent_id then
        local user_equip = tools.get_agent_equipment(agent_id, nil) or {}
        for _, item in ipairs(user_equip) do
            equipped_ids[item.thing_id] = true
        end
    end
    if room_id then
        local room_equip = tools.get_room_equipment(room_id, nil) or {}
        for _, item in ipairs(room_equip) do
            equipped_ids[item.thing_id] = true
        end
    end

    -- Filter to available, unequipped things
    local available_count = 0
    for _, thing in ipairs(all_things) do
        if thing.available and not equipped_ids[thing.id] then
            local status = "○"
            local qname = thing.qualified_name or thing.name
            table.insert(lines, string.format("  %s %s", status, qname))
            available_count = available_count + 1
        end
    end
    if available_count == 0 then
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
--   /equip room hook:wrap myns:wrapper  - Equip as wrap hook
--------------------------------------------------------------------------------

function M.equip(args)
    if not args or args:match("^%s*$") then
        page.show("Equip", [[
Usage: /equip <context> [slot] <pattern>

Context: me | room
Pattern: qualified name or glob (e.g., sshwarma:*, holler:sample)
Slot (optional): command:*, hook:wrap, hook:background

Examples:
  /equip me sshwarma:*              Equip all sshwarma tools to yourself
  /equip room holler:sample         Equip tool to room (for LLM)
  /equip me command:fish atobey:fish   Bind /fish to atobey:fish
  /equip room hook:wrap myns:wrap   Add wrap hook to room
]])
        return {}
    end

    local parts = str.split(args, "%s+")
    local context = parts[1]  -- 'me' or 'room'

    if context ~= "me" and context ~= "room" then
        return {
            text = "Error: context must be 'me' or 'room'",
            mode = "notification"
        }
    end

    -- Parse remaining args: could be [slot] <pattern> or just <pattern>
    local slot, pattern
    if #parts >= 3 then
        -- Check if second part looks like a slot
        local maybe_slot = parts[2]
        if maybe_slot:match("^command:") or
           maybe_slot:match("^hook:") or
           maybe_slot:match("^hotkey:") then
            slot = maybe_slot
            pattern = parts[3]
        else
            -- No slot, second part is the pattern
            pattern = parts[2]
        end
    elseif #parts == 2 then
        pattern = parts[2]
    else
        return {
            text = "Error: missing pattern argument",
            mode = "notification"
        }
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
        return {
            text = string.format("No things match pattern: %s", pattern),
            mode = "notification"
        }
    end

    -- Get target ID
    local target_id
    if context == "me" then
        target_id = get_agent_id()
        if not target_id then
            return {text = "Error: not logged in", mode = "notification"}
        end
    else
        target_id = get_room_id()
        if not target_id then
            return {text = "Error: not in a room", mode = "notification"}
        end
    end

    -- Equip each matching thing
    local equipped = {}
    for i, thing in ipairs(things) do
        local actual_slot = slot or thing.default_slot
        local success
        if context == "me" then
            success = tools.agent_equip(target_id, thing.id, actual_slot, config, i)
        else
            success = tools.room_equip(target_id, thing.id, actual_slot, i)
        end
        if success then
            table.insert(equipped, thing.qualified_name or thing.name)
        end
    end

    if #equipped == 0 then
        return {
            text = "Error: could not equip any items",
            mode = "notification"
        }
    end

    local lines = {string.format("Equipped %d item(s) to %s:", #equipped, context)}
    for _, name in ipairs(equipped) do
        table.insert(lines, "  " .. name)
    end

    page.show("Equip", table.concat(lines, "\n"))
    return {}
end

--------------------------------------------------------------------------------
-- /unequip <context> [slot] <pattern>
--
-- Examples:
--   /unequip me sshwarma:*         - Unequip all sshwarma tools from user
--   /unequip room holler:sample    - Unequip tool from room
--   /unequip me command:fish       - Unequip by slot only
--------------------------------------------------------------------------------

function M.unequip(args)
    if not args or args:match("^%s*$") then
        page.show("Unequip", [[
Usage: /unequip <context> [slot] <pattern>

Context: me | room
Pattern: qualified name or glob
Slot (optional): command:*, hook:wrap, hook:background

Examples:
  /unequip me sshwarma:*            Unequip all sshwarma tools
  /unequip room holler:sample       Unequip from room
]])
        return {}
    end

    local parts = str.split(args, "%s+")
    local context = parts[1]  -- 'me' or 'room'

    if context ~= "me" and context ~= "room" then
        return {
            text = "Error: context must be 'me' or 'room'",
            mode = "notification"
        }
    end

    -- Parse remaining args: could be [slot] <pattern> or just <pattern>
    local slot, pattern
    if #parts >= 3 then
        local maybe_slot = parts[2]
        if maybe_slot:match("^command:") or
           maybe_slot:match("^hook:") or
           maybe_slot:match("^hotkey:") then
            slot = maybe_slot
            pattern = parts[3]
        else
            pattern = parts[2]
        end
    elseif #parts == 2 then
        pattern = parts[2]
    else
        return {
            text = "Error: missing pattern argument",
            mode = "notification"
        }
    end

    -- Expand pattern to matching things
    local things = tools.things_match(pattern) or {}
    if #things == 0 then
        return {
            text = string.format("No things match pattern: %s", pattern),
            mode = "notification"
        }
    end

    -- Get target ID
    local target_id
    if context == "me" then
        target_id = get_agent_id()
        if not target_id then
            return {text = "Error: not logged in", mode = "notification"}
        end
    else
        target_id = get_room_id()
        if not target_id then
            return {text = "Error: not in a room", mode = "notification"}
        end
    end

    -- Unequip each matching thing
    local unequipped = {}
    for _, thing in ipairs(things) do
        local success
        if context == "me" then
            success = tools.agent_unequip(target_id, thing.id, slot)
        else
            success = tools.room_unequip(target_id, thing.id, slot)
        end
        if success then
            table.insert(unequipped, thing.qualified_name or thing.name)
        end
    end

    if #unequipped == 0 then
        return {
            text = "Error: could not unequip any items",
            mode = "notification"
        }
    end

    local lines = {string.format("Unequipped %d item(s) from %s:", #unequipped, context)}
    for _, name in ipairs(unequipped) do
        table.insert(lines, "  " .. name)
    end

    page.show("Unequip", table.concat(lines, "\n"))
    return {}
end

return M
