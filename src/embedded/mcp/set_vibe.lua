-- mcp/set_vibe.lua - Set the vibe/atmosphere for a room
-- Wave 4: Room mutation MCP tool

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "set_vibe",
    description = "Set the vibe/atmosphere for a room",
    schema = {
        type = "object",
        properties = {
            room = {
                type = "string",
                description = "Name of the room"
            },
            vibe = {
                type = "string",
                description = "Vibe/atmosphere description for the room"
            }
        },
        required = { "room", "vibe" }
    },
    module_path = "mcp.set_vibe"
}

--- Handler function called when the tool is invoked
--- @param params table The parameters passed to the tool
--- @return table Result with status or error
function M.handler(params)
    if not params.room then
        return { error = "room parameter is required" }
    end

    if not params.vibe then
        return { error = "vibe parameter is required" }
    end

    -- Use the db_set_vibe primitive
    local result = tools.db_set_vibe(params.room, params.vibe)

    if result.success then
        return {
            status = "updated",
            room = params.room,
            vibe = params.vibe,
            message = string.format("Set vibe for '%s': %s", params.room, params.vibe)
        }
    else
        return { error = result.error }
    end
end

return M
