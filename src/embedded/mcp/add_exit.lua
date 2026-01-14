-- mcp/add_exit.lua - Create an exit from one room to another
-- Wave 4: Room mutation MCP tool

local M = {}

--- Tool definition for MCP registration
M.tool = {
    name = "add_exit",
    description = "Create an exit from one room to another",
    schema = {
        type = "object",
        properties = {
            room = {
                type = "string",
                description = "Name of the source room"
            },
            direction = {
                type = "string",
                description = "Direction (e.g. 'north', 'south', 'east', 'west', 'up', 'down', 'studio', 'archive')"
            },
            target = {
                type = "string",
                description = "Target room name"
            },
            bidirectional = {
                type = "boolean",
                description = "Create bidirectional exit (default true)"
            }
        },
        required = { "room", "direction", "target" }
    },
    module_path = "mcp.add_exit"
}

--- Handler function called when the tool is invoked
--- @param params table The parameters passed to the tool
--- @return table Result with status or error
function M.handler(params)
    if not params.room then
        return { error = "room parameter is required" }
    end

    if not params.direction then
        return { error = "direction parameter is required" }
    end

    if not params.target then
        return { error = "target parameter is required" }
    end

    -- Default bidirectional to true if not specified
    local bidirectional = params.bidirectional
    if bidirectional == nil then
        bidirectional = true
    end

    -- Use the db_add_exit primitive
    local result = tools.db_add_exit(params.room, params.direction, params.target, bidirectional)

    if result.success then
        local response = {
            status = "created",
            room = params.room,
            direction = params.direction,
            target = params.target
        }

        if result.reverse then
            response.reverse = result.reverse
            response.message = string.format(
                "Created exit %s -> %s (via '%s') and reverse exit (via '%s')",
                params.room, params.target, params.direction, result.reverse
            )
        else
            response.message = string.format(
                "Created one-way exit %s -> %s (via '%s')",
                params.room, params.target, params.direction
            )
        end

        -- Include warning if reverse creation failed
        if result.warning then
            response.warning = result.warning
        end

        return response
    else
        return { error = result.error }
    end
end

return M
