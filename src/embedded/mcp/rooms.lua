-- mcp/rooms.lua - List all available rooms
-- Read-only MCP tool implementation

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "list_rooms",
    description = "List all available rooms",
    schema = {
        type = "object",
        properties = {},
    },
    module_path = "mcp.rooms"
}

--- Handler function called when the tool is invoked
--- @param params table The parameters passed to the tool (unused)
--- @return table Array of room objects with name, description, and vibe
function M.handler(params)
    -- Use the db_rooms primitive which returns:
    -- { id, name, created_at, vibe?, description? }
    local rooms = tools.db_rooms()
    local result = {}

    for i, room in ipairs(rooms) do
        result[i] = {
            name = room.name,
            description = room.description,
            vibe = room.vibe
        }
    end

    return result
end

return M
