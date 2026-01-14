-- mcp/fork_room.lua - Fork a room, inheriting its context
-- Wave 4: Room mutation MCP tool

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "fork_room",
    description = "Fork a room, inheriting its context",
    schema = {
        type = "object",
        properties = {
            source = {
                type = "string",
                description = "Name of the source room to fork from"
            },
            new_name = {
                type = "string",
                description = "Name for the new forked room (alphanumeric, dashes, underscores)"
            }
        },
        required = { "source", "new_name" }
    },
    module_path = "mcp.fork_room"
}

--- Handler function called when the tool is invoked
--- @param params table The parameters passed to the tool
--- @return table Result with status and room info or error
function M.handler(params)
    if not params.source then
        return { error = "source parameter is required" }
    end

    if not params.new_name then
        return { error = "new_name parameter is required" }
    end

    -- Use the db_fork_room primitive which handles validation and context copying
    local result = tools.db_fork_room(params.source, params.new_name)

    if result.success then
        return {
            status = "forked",
            room = result.room,
            source = result.source,
            message = string.format(
                "Forked '%s' from '%s'. Inherited: vibe, tags, assets, equipment.",
                result.room, result.source
            )
        }
    else
        return { error = result.error }
    end
end

return M
