-- mcp/create_room.lua - Create a new room
-- Wave 4: Room mutation MCP tool

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "create_room",
    description = "Create a new room",
    schema = {
        type = "object",
        properties = {
            name = {
                type = "string",
                description = "Name for the new room (alphanumeric, dashes, underscores)"
            },
            description = {
                type = "string",
                description = "Optional description for the room"
            }
        },
        required = { "name" }
    },
    module_path = "mcp.create_room"
}

--- Handler function called when the tool is invoked
--- @param params table The parameters passed to the tool
--- @return table Result with status and room info or error
function M.handler(params)
    if not params.name then
        return { error = "name parameter is required" }
    end

    -- Use the db_create_room primitive which handles validation
    local result = tools.db_create_room(params.name, params.description)

    if result.success then
        return {
            status = "created",
            room = result.room,
            message = string.format("Created room '%s'", result.room)
        }
    else
        return { error = result.error }
    end
end

return M
