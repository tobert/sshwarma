-- mcp/inventory.lua - Room inventory management
-- Wave 5: Scripts/Things MCP tools

local M = {}

--- Tool definitions for MCP registration
M.tools = {
    {
        name = "inventory_list",
        description = "List equipped tools in a room's inventory",
        schema = {
            type = "object",
            properties = {
                room = {
                    type = "string",
                    description = "Name of the room to list inventory for"
                },
                include_available = {
                    type = "boolean",
                    description = "Include available (unequipped) tools in list"
                }
            },
            required = { "room" }
        },
        module_path = "mcp.inventory",
        handler_name = "list"
    },
    {
        name = "inventory_equip",
        description = "Equip a tool in a room",
        schema = {
            type = "object",
            properties = {
                room = {
                    type = "string",
                    description = "Name of the room"
                },
                qualified_name = {
                    type = "string",
                    description = "Qualified name of the thing to equip (e.g. 'holler:sample')"
                },
                priority = {
                    type = "number",
                    description = "Priority for ordering (lower = first)"
                }
            },
            required = { "room", "qualified_name" }
        },
        module_path = "mcp.inventory",
        handler_name = "equip"
    },
    {
        name = "inventory_unequip",
        description = "Unequip a tool from a room",
        schema = {
            type = "object",
            properties = {
                room = {
                    type = "string",
                    description = "Name of the room"
                },
                qualified_name = {
                    type = "string",
                    description = "Qualified name of the thing to unequip"
                }
            },
            required = { "room", "qualified_name" }
        },
        module_path = "mcp.inventory",
        handler_name = "unequip"
    }
}

--- List equipped tools in a room's inventory
--- @param params table { room: string, include_available?: boolean }
--- @return table Result with equipped list or error
function M.list(params)
    if not params.room then
        return { error = "room parameter is required" }
    end

    -- Look up room to get its ID
    local room = tools.db_room(params.room)
    if not room then
        return { error = string.format("Room '%s' not found", params.room) }
    end

    -- Get equipped tools for the room
    local equipped = tools.equipped_list(room.id)
    local equipped_result = {}

    for i, eq in ipairs(equipped) do
        equipped_result[i] = {
            qualified_name = eq.qualified_name,
            name = eq.name,
            kind = eq.kind,
            priority = eq.priority,
            available = eq.available,
            slot = eq.slot
        }
    end

    local result = {
        room = params.room,
        equipped = equipped_result
    }

    -- Include available (unequipped) tools if requested
    if params.include_available then
        local all_tools = tools.things_by_kind and tools.things_by_kind("tool") or {}
        local equipped_ids = {}
        for _, eq in ipairs(equipped) do
            equipped_ids[eq.id] = true
        end

        local available = {}
        for _, t in ipairs(all_tools) do
            if t.available and not equipped_ids[t.id] then
                table.insert(available, {
                    qualified_name = t.qualified_name,
                    name = t.name,
                    kind = t.kind
                })
            end
        end
        result.available = available
    end

    return result
end

--- Equip a tool in a room
--- @param params table { room: string, qualified_name: string, priority?: number }
--- @return table Result with status or error
function M.equip(params)
    if not params.room then
        return { error = "room parameter is required" }
    end
    if not params.qualified_name then
        return { error = "qualified_name parameter is required" }
    end

    -- Look up room to get its ID
    local room = tools.db_room(params.room)
    if not room then
        return { error = string.format("Room '%s' not found", params.room) }
    end

    -- Find the thing by qualified name
    local things = tools.things_find(params.qualified_name)
    if not things or #things == 0 then
        return { error = string.format("Tool '%s' not found", params.qualified_name) }
    end
    if #things > 1 and not params.qualified_name:find("*") then
        return { error = string.format("Multiple matches for '%s', be more specific", params.qualified_name) }
    end

    local equipped_names = {}
    for _, thing in ipairs(things) do
        -- Get max priority for this room
        local current_equipped = tools.equipped_list(room.id)
        local max_priority = 0
        for _, eq in ipairs(current_equipped) do
            if eq.priority > max_priority then
                max_priority = eq.priority
            end
        end
        local priority = params.priority or (max_priority + 1)

        -- Use room_equip primitive: room_equip(room_id, thing_id, slot, priority)
        local success = tools.room_equip(room.id, thing.id, nil, priority)
        if success then
            table.insert(equipped_names, thing.qualified_name or thing.name)
        end
    end

    if #equipped_names > 0 then
        return {
            status = "equipped",
            room = params.room,
            equipped = equipped_names,
            message = string.format("Equipped %d tool(s) in '%s'", #equipped_names, params.room)
        }
    else
        return { error = "Failed to equip tool(s)" }
    end
end

--- Unequip a tool from a room
--- @param params table { room: string, qualified_name: string }
--- @return table Result with status or error
function M.unequip(params)
    if not params.room then
        return { error = "room parameter is required" }
    end
    if not params.qualified_name then
        return { error = "qualified_name parameter is required" }
    end

    -- Look up room to get its ID
    local room = tools.db_room(params.room)
    if not room then
        return { error = string.format("Room '%s' not found", params.room) }
    end

    -- Find the thing by qualified name
    local things = tools.things_find(params.qualified_name)
    if not things or #things == 0 then
        return { error = string.format("Tool '%s' not found", params.qualified_name) }
    end

    local unequipped_names = {}
    for _, thing in ipairs(things) do
        -- Use room_unequip primitive: room_unequip(room_id, thing_id, slot)
        local success = tools.room_unequip(room.id, thing.id, nil)
        if success then
            table.insert(unequipped_names, thing.qualified_name or thing.name)
        end
    end

    if #unequipped_names > 0 then
        return {
            status = "unequipped",
            room = params.room,
            removed = unequipped_names,
            message = string.format("Unequipped %d tool(s) from '%s'", #unequipped_names, params.room)
        }
    else
        return { error = "Failed to unequip tool(s)" }
    end
end

return M
