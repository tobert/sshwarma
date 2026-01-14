-- mcp/rows.lua - Query rows from a room as nested tree
local tree = require('lib.tree')

local M = {}

M.tool = {
    name = "rows",
    description = "Query rows from a room as nested JSON tree. Better than get_history for understanding conversation structure.",
    schema = {
        type = "object",
        properties = {
            room = { type = "string", description = "Room name to query" },
            limit = { type = "integer", description = "Max rows to return (default 50)" }
        },
        required = { "room" }
    },
    module_path = "mcp.rows"
}

function M.handler(params)
    if not params.room or params.room == "" then
        return { error = "room parameter is required" }
    end

    local limit = params.limit or 50
    local flat_rows = tools.db_rows(params.room, { limit = limit })

    if not flat_rows or #flat_rows == 0 then
        return { rows = {}, count = 0 }
    end

    -- Build tree structure
    local nested = tree.build(flat_rows)

    return {
        rows = nested,
        count = #flat_rows,
        room = params.room
    }
end

return M
