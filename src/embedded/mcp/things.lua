-- mcp/things.lua - Thing operations (contents, take, drop, create, destroy)
-- Wave 5: Scripts/Things MCP tools

local M = {}

--- Tool definitions for MCP registration
M.tools = {
    {
        name = "thing_contents",
        description = "List contents of a container (things inside rooms, agents, or shared)",
        schema = {
            type = "object",
            properties = {
                target = {
                    type = "string",
                    description = "Target container: 'shared', room name, or @agent_name"
                }
            },
            required = { "target" }
        },
        module_path = "mcp.things",
        handler_name = "contents"
    },
    {
        name = "thing_take",
        description = "Copy a thing into your inventory (copy-on-write)",
        schema = {
            type = "object",
            properties = {
                thing = {
                    type = "string",
                    description = "Qualified name or pattern of thing to copy (e.g., 'holler:sample')"
                }
            },
            required = { "thing" }
        },
        module_path = "mcp.things",
        handler_name = "take"
    },
    {
        name = "thing_drop",
        description = "Move a thing from your inventory to a room",
        schema = {
            type = "object",
            properties = {
                thing = {
                    type = "string",
                    description = "Name of thing in your inventory to drop"
                },
                room = {
                    type = "string",
                    description = "Room to drop into (defaults to 'lobby')"
                }
            },
            required = { "thing" }
        },
        module_path = "mcp.things",
        handler_name = "drop"
    },
    {
        name = "thing_create",
        description = "Create a new thing in a container",
        schema = {
            type = "object",
            properties = {
                target = {
                    type = "string",
                    description = "Target container: 'me', room name, 'shared', or @agent_name"
                },
                name = {
                    type = "string",
                    description = "Name for the new thing"
                },
                kind = {
                    type = "string",
                    description = "Kind: 'data', 'container', or 'tool' (default: 'data')"
                },
                content = {
                    type = "string",
                    description = "Content for data things"
                },
                code = {
                    type = "string",
                    description = "Lua code for tool things"
                },
                description = {
                    type = "string",
                    description = "Description of the thing"
                }
            },
            required = { "target", "name" }
        },
        module_path = "mcp.things",
        handler_name = "create"
    },
    {
        name = "thing_destroy",
        description = "Delete a thing (must specify owner:name)",
        schema = {
            type = "object",
            properties = {
                target = {
                    type = "string",
                    description = "Owner and thing name: 'me:thing', 'room:thing', '@agent:thing'"
                }
            },
            required = { "target" }
        },
        module_path = "mcp.things",
        handler_name = "destroy"
    }
}

--- Resolve containment target to parent_id
--- @param target string Target identifier
--- @return string|nil parent_id
--- @return string|nil error message
local function resolve_target(target)
    if target == "me" then
        -- For MCP, "me" = agent_claude
        return "agent_claude", nil
    elseif target == "shared" or target == "world" then
        return "shared", nil
    elseif target:sub(1, 1) == "@" then
        -- @agent_name format
        local agent_name = target:sub(2)
        return "agent_" .. agent_name, nil
    else
        -- Assume it's a room name
        local room = tools.db_room(target)
        if room then
            return room.id, nil
        else
            return nil, string.format("Room '%s' not found", target)
        end
    end
end

--- List contents of a container
--- @param params table { target: string }
--- @return table Result with contents or error
function M.contents(params)
    if not params.target then
        return { error = "target parameter is required" }
    end

    local parent_id, err = resolve_target(params.target)
    if err then
        return { error = err }
    end

    local children = tools.things_children(parent_id)
    local contents = {}

    for i, thing in ipairs(children or {}) do
        contents[i] = {
            id = thing.id,
            name = thing.name,
            qualified_name = thing.qualified_name,
            kind = thing.kind,
            available = thing.available
        }
    end

    -- Generate title based on target
    local title
    if params.target == "me" then
        title = "Your Inventory"
    elseif params.target == "shared" or params.target == "world" then
        title = "Shared Resources"
    elseif params.target:sub(1, 1) == "@" then
        title = params.target .. "'s Inventory"
    else
        title = "Contents of room '" .. params.target .. "'"
    end

    return {
        target = params.target,
        title = title,
        contents = contents
    }
end

--- Copy a thing into agent's inventory (copy-on-write)
--- @param params table { thing: string }
--- @return table Result with status or error
function M.take(params)
    if not params.thing then
        return { error = "thing parameter is required" }
    end

    local agent_thing_id = "agent_claude"

    -- Find the thing to take
    local things = tools.things_find(params.thing)
    if not things or #things == 0 then
        return { error = string.format("Thing '%s' not found", params.thing) }
    end

    if #things > 1 then
        return { error = string.format("Ambiguous: %d matches for '%s'", #things, params.thing) }
    end

    local thing = things[1]

    -- Copy the thing using thing_copy
    local copy = tools.thing_copy(thing.id, agent_thing_id)
    if copy then
        local name = thing.qualified_name or thing.name
        return {
            status = "taken",
            name = name,
            copy_id = copy.id,
            message = string.format("Took %s (copy id: %s)", name, copy.id:sub(1, 8))
        }
    else
        return { error = string.format("Failed to take '%s'", params.thing) }
    end
end

--- Move a thing from agent's inventory to a room
--- @param params table { thing: string, room?: string }
--- @return table Result with status or error
function M.drop(params)
    if not params.thing then
        return { error = "thing parameter is required" }
    end

    local agent_thing_id = "agent_claude"
    local room_name = params.room or "lobby"

    -- Get room ID
    local room = tools.db_room(room_name)
    if not room then
        return { error = string.format("Room '%s' not found", room_name) }
    end

    -- Find thing in agent's inventory
    local children = tools.things_children(agent_thing_id)
    local thing = nil
    for _, t in ipairs(children or {}) do
        if t.name == params.thing or t.qualified_name == params.thing then
            thing = t
            break
        end
    end

    if not thing then
        return { error = string.format("'%s' not in your inventory", params.thing) }
    end

    -- Move it using thing_move
    local success = tools.thing_move(thing.id, room.id)
    if success then
        local name = thing.qualified_name or thing.name
        return {
            status = "dropped",
            name = name,
            room = room_name,
            message = string.format("Dropped %s into %s", name, room_name)
        }
    else
        return { error = string.format("Failed to drop '%s'", params.thing) }
    end
end

--- Create a new thing in a container
--- @param params table { target, name, kind?, content?, code?, description? }
--- @return table Result with status or error
function M.create(params)
    if not params.target then
        return { error = "target parameter is required" }
    end
    if not params.name then
        return { error = "name parameter is required" }
    end

    -- Resolve target to parent_id
    local parent_id, err = resolve_target(params.target)
    if err then
        return { error = err }
    end

    -- Validate kind
    local kind = params.kind or "data"
    if kind ~= "data" and kind ~= "container" and kind ~= "tool" then
        return { error = string.format("Invalid kind '%s'. Use: data, container, tool", kind) }
    end

    -- Generate qualified name
    local qualified_name
    if params.name:find(":") then
        qualified_name = params.name
    else
        qualified_name = "claude:" .. params.name
    end

    -- Use thing_create primitive
    local result = tools.thing_create({
        qualified_name = qualified_name,
        name = params.name,
        kind = kind,
        parent_id = parent_id,
        content = params.content,
        code = params.code,
        description = params.description
    })

    if result and result.success then
        local target_desc
        if params.target == "me" then
            target_desc = "your inventory"
        elseif params.target:sub(1, 1) == "@" then
            target_desc = params.target .. "'s inventory"
        else
            target_desc = params.target
        end

        return {
            status = "created",
            qualified_name = qualified_name,
            id = result.id,
            target = target_desc,
            message = string.format("Created %s in %s (id: %s)", qualified_name, target_desc, result.id:sub(1, 8))
        }
    else
        return { error = result and result.error or "Failed to create thing" }
    end
end

--- Delete a thing
--- @param params table { target: string } -- format: "owner:thing"
--- @return table Result with status or error
function M.destroy(params)
    if not params.target then
        return { error = "target parameter is required" }
    end

    -- Parse owner:thing format
    local colon_pos = params.target:find(":")
    if not colon_pos then
        return { error = "Must specify owner:thing (e.g., 'me:old-note', '@claude:test')" }
    end

    local owner = params.target:sub(1, colon_pos - 1)
    local thing_name = params.target:sub(colon_pos + 1)

    -- Resolve owner to parent_id
    local parent_id, err = resolve_target(owner)
    if err then
        return { error = err }
    end

    -- Find thing under owner
    local children = tools.things_children(parent_id)
    local thing = nil
    for _, t in ipairs(children or {}) do
        if t.name == thing_name or t.qualified_name == thing_name then
            thing = t
            break
        end
    end

    if not thing then
        return { error = string.format("'%s' not found under '%s'", thing_name, owner) }
    end

    -- Use thing_delete primitive
    local result = tools.thing_delete(thing.qualified_name or (owner .. ":" .. thing_name))
    if result and result.success then
        return {
            status = "destroyed",
            name = thing_name,
            message = string.format("Destroyed %s", thing_name)
        }
    else
        return { error = result and result.error or "Failed to destroy thing" }
    end
end

return M
