-- mcp/say.lua - Send a message to a room, with @mention support
local M = {}

M.tool = {
    name = "say",  -- Replace existing Rust say tool
    description = "Send a message to a room. Supports @mention syntax to invoke models.",
    schema = {
        type = "object",
        properties = {
            room = { type = "string", description = "Room name" },
            message = { type = "string", description = "Message to send" },
            sender = { type = "string", description = "Sender name (optional, defaults to 'claude')" }
        },
        required = { "room", "message" }
    },
    module_path = "mcp.say"
}

function M.handler(params)
    if not params.room or params.room == "" then
        return { error = "room parameter is required" }
    end
    if not params.message or params.message == "" then
        return { error = "message parameter is required" }
    end

    -- Check for @mention pattern at start of message
    -- Pattern: @model-name followed by space and message content
    local model_name, rest = params.message:match("^@(%w[%w%-_]*)%s+(.+)")

    if model_name and rest then
        -- This is an @mention - trigger model response
        local result = tools.trigger_mention(model_name, rest, params.room)
        if result.error then
            return { error = result.error }
        end
        return {
            status = "mention_triggered",
            model = model_name,
            room = params.room,
            message_row_id = result.message_row_id,
            response_row_id = result.response_row_id,
            note = "Model response is being generated. Poll with 'rows' or 'row' tool."
        }
    else
        -- Regular message - just add to buffer
        local sender = params.sender or "claude"

        -- Get the room's buffer
        local buffer = tools.db_buffer(params.room)
        if not buffer then
            return { error = "Room not found: " .. params.room }
        end

        -- Get or create agent for sender
        local agents = tools.db_agents()
        local agent_id = nil
        if agents then
            for _, agent in ipairs(agents) do
                if agent.name == sender then
                    agent_id = agent.id
                    break
                end
            end
        end

        -- Append the row to the buffer
        local row_id = tools.db_append_row(buffer.id, agent_id, params.message, false)
        if not row_id then
            return { error = "Failed to send message" }
        end

        return {
            status = "sent",
            room = params.room,
            sender = sender,
            row_id = row_id
        }
    end
end

return M
