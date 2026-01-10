-- sshwarma conjure command
--
-- Create new things (tools, hooks, commands) from scratch or wrap MCP tools.
--
-- Examples:
--   /conjure fish                       Create new thing with stub code
--   /conjure fish --code 'return ...'   Create with inline code
--   /conjure myns:fish                  Create with explicit namespace
--   /conjure fish holler:sample         Wrap MCP tool with custom name

local page = require('page')
local str = require('str')
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
-- /conjure <name> [source] [--code 'code']
--
-- Examples:
--   /conjure fish                  -> username:fish with stub code
--   /conjure myns:fish             -> myns:fish with stub code
--   /conjure fish --code 'return "hello"'  -> username:fish with inline code
--   /conjure fish holler:sample    -> username:fish wrapping MCP tool (future)
--------------------------------------------------------------------------------

function M.conjure(args)
    if not args or args:match("^%s*$") then
        page.show("Conjure", [[
Usage: /conjure <name> [--code 'code']

Create a new thing (tool, hook, or command).

Arguments:
  name        Thing name (e.g., fish or myns:fish)
  --code      Inline Lua code for the thing

Examples:
  /conjure fish                     Create thing with stub code
  /conjure fish --code 'return "hello"'   Create with inline code
  /conjure myns:fish                Create in specific namespace

The created thing can then be equipped:
  /equip me command:fish username:fish
  /equip room hook:wrap username:myhook
]])
        return {}
    end

    -- Parse arguments
    local parts = str.split(args, "%s+")
    local name_arg = parts[1]

    -- Check for --code
    local code = parse_code_arg(args)

    -- Parse name: could be "fish" or "myns:fish"
    local qualified_name, short_name
    if name_arg:match(":") then
        qualified_name = name_arg
        short_name = name_arg:match(":(.+)$")
    else
        short_name = name_arg
        qualified_name = get_username() .. ":" .. name_arg
    end

    -- Default code if not provided
    if not code then
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

    local lines = {
        string.format("Created: %s", qualified_name),
        "",
        "To use this thing:",
        string.format("  /equip me command:%s %s", short_name, qualified_name),
        string.format("  /%s", short_name),
        "",
        "Or equip to room for LLM access:",
        string.format("  /equip room %s", qualified_name),
    }

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
