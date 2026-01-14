-- mcp/room_context.lua - Get full room context for agent onboarding
local M = {}

M.tool = {
    name = "room_context",
    description = "Get full room context for agent onboarding - vibe, description, exits",
    schema = {
        type = "object",
        properties = {
            room = { type = "string", description = "Room name to get context for" }
        },
        required = { "room" }
    },
    module_path = "mcp.room_context"
}

function M.handler(params)
    if not params.room or params.room == "" then
        return { error = "room parameter is required" }
    end

    local room = tools.db_room(params.room)
    if not room then
        return { error = "Room not found: " .. params.room }
    end

    local exits = tools.db_exits(params.room)

    return {
        room = params.room,
        vibe = room.vibe,
        description = room.description,
        exits = exits or {},
        created_at = room.created_at
    }
end

return M
