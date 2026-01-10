-- sshwarma conjure command
--
-- Create new things (tools, hooks, commands) in a specific container.
--
-- Examples:
--   /conjure me fish                    Create thing in my inventory
--   /conjure room welcome               Create thing in current room
--   /conjure me memories/               Create container (trailing /)
--   /conjure me fish --code 'return {}' Create with inline code
--   /conjure @agent fish                Create in another agent's inventory

local page = require('page')
local str = require('str')
local util = require('util')
local M = {}

--------------------------------------------------------------------------------
-- Helper: get current username for namespace
--------------------------------------------------------------------------------

local function get_username()
    local user = tools.current_user()
    return user and user.name or "user"
end

--------------------------------------------------------------------------------
-- Helper: parse --code argument
--------------------------------------------------------------------------------

local function parse_code_arg(args)
    -- Look for --code 'some code' or --code "some code"
    local code = args:match("%-%-code%s+'([^']*)'")
    if not code then
        code = args:match('%-%-code%s+"([^"]*)"')
    end
    if not code then
        -- Unquoted: take rest of line after --code
        code = args:match("%-%-code%s+(.+)$")
    end
    return code
end

--------------------------------------------------------------------------------
-- Helper: resolve target to parent_id
--------------------------------------------------------------------------------

local function resolve_target(target)
    if target == "me" then
        local id = tools.get_agent_thing_id()
        if id then return id, nil end
        return nil, "Not logged in"
    elseif target == "room" then
        local id = util.get_room_id()
        if id then return id, nil end
        return nil, "Not in a room"
    elseif target == "shared" or target == "world" then
        return "shared", nil
    elseif target:match("^@") then
        local agent_name = target:sub(2)
        return "agent_" .. agent_name, nil
    else
        return nil, "Invalid target: use me, room, shared, or @agent"
    end
end

--------------------------------------------------------------------------------
-- /conjure <target> <name> [--code 'code']
--
-- target: me | room | shared | @agent
-- name:   Thing name (use trailing / for container)
--
-- Examples:
--   /conjure me fish                    -> thing under agents/me
--   /conjure room welcome-prompt        -> thing under current room
--   /conjure me memories/               -> create container under me
--   /conjure me fish --code 'return {}' -> create with inline code
--------------------------------------------------------------------------------

function M.conjure(args)
    if not args or args:match("^%s*$") then
        page.show("Conjure", [[
Usage: /conjure <target> <name> [--code 'code']

Create a new thing in a container.

Target: me | room | shared | @agent
Name:   Thing name (use trailing / for container)

Examples:
  /conjure me fish                    Create thing in my inventory
  /conjure room welcome               Create thing in room
  /conjure me memories/               Create container (note trailing /)
  /conjure me fish --code 'return {}' Create with inline code
  /conjure shared utils               Create in shared resources
  /conjure @qwenl note                Create in another agent's inventory

The created thing can then be equipped:
  /equip me command:fish username:fish
  /equip room hook:wrap username:myhook
]])
        return {}
    end

    -- Parse arguments (target and name, plus optional --code)
    local parts = str.split(args, "%s+")
    local target = parts[1]
    local name_arg = parts[2]

    if not name_arg then
        return { text = "Usage: /conjure <target> <name>", mode = "notification" }
    end

    -- Resolve target to parent_id
    local parent_id, err = resolve_target(target)
    if not parent_id then
        return { text = "Error: " .. err, mode = "notification" }
    end

    -- Detect container creation (trailing /)
    local is_container = name_arg:match("/$")
    local short_name = is_container and name_arg:sub(1, -2) or name_arg

    -- Check for --code
    local code = parse_code_arg(args)

    -- Determine kind
    local kind = "data"
    if is_container then
        kind = "container"
    elseif code then
        kind = "tool"
    end

    -- Parse name: could be "fish" or "myns:fish"
    local qualified_name
    if short_name:match(":") then
        qualified_name = short_name
        short_name = short_name:match(":(.+)$")
    else
        qualified_name = get_username() .. ":" .. short_name
    end

    -- Default code if not provided and it's a tool
    if kind == "tool" and not code then
        code = string.format([[
-- %s
-- TODO: implement this thing
return function(args)
    return {success = true, message = "hello from %s"}
end
]], qualified_name, short_name)
    end

    -- Create the thing
    local result = tools.thing_create({
        qualified_name = qualified_name,
        name = short_name,
        kind = kind,
        parent_id = parent_id,
        description = "Created by /conjure",
        code = code,
        created_by = get_username(),
    })

    if not result or not result.success then
        return {
            text = string.format("Error: %s", result and result.error or "unknown error"),
            mode = "notification"
        }
    end

    -- Build success message
    local target_desc
    if target == "me" then
        target_desc = "your inventory"
    elseif target == "room" then
        target_desc = "the room"
    elseif target == "shared" then
        target_desc = "shared resources"
    else
        target_desc = target .. "'s inventory"
    end

    local lines = {
        string.format("Created: %s", qualified_name),
        string.format("   Kind: %s", kind),
        string.format("     In: %s", target_desc),
        "",
    }

    if kind == "tool" then
        table.insert(lines, "To use as a command:")
        table.insert(lines, string.format("  /equip me command:%s %s", short_name, qualified_name))
        table.insert(lines, string.format("  /%s", short_name))
        table.insert(lines, "")
        table.insert(lines, "Or equip to room for LLM access:")
        table.insert(lines, string.format("  /equip room %s", qualified_name))
    elseif kind == "container" then
        table.insert(lines, string.format("View contents with: /inv %s", target))
    else
        table.insert(lines, string.format("View with: /inv %s", target))
    end

    page.show("Conjure", table.concat(lines, "\n"))
    return {}
end

--------------------------------------------------------------------------------
-- /unconjure <qualified_name> - Delete a thing
--------------------------------------------------------------------------------

function M.unconjure(args)
    if not args or args:match("^%s*$") then
        page.show("Unconjure", [[
Usage: /unconjure <qualified_name>

Delete (soft-delete) a thing.

Example:
  /unconjure username:fish
]])
        return {}
    end

    local qualified_name = args:match("^%s*(%S+)")

    local result = tools.thing_delete(qualified_name)

    if not result or not result.success then
        return {
            text = string.format("Error: %s", result and result.error or "unknown error"),
            mode = "notification"
        }
    end

    return {
        text = string.format("Deleted: %s", qualified_name),
        mode = "notification"
    }
end

return M
