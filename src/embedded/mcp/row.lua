-- mcp/row.lua - Get a single row by ID
local M = {}

M.tool = {
    name = "row",
    description = "Get a single row by ID with full details",
    schema = {
        type = "object",
        properties = {
            id = { type = "string", description = "Row ID to retrieve" }
        },
        required = { "id" }
    },
    module_path = "mcp.row"
}

function M.handler(params)
    if not params.id or params.id == "" then
        return { error = "id parameter is required" }
    end

    local row = tools.db_row(params.id)

    if not row then
        return { error = "Row not found", id = params.id }
    end

    return row
end

return M
