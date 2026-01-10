-- commands/init.lua - Main command dispatcher for sshwarma
--
-- Provides a Lua-based command handler that maps slash commands to
-- appropriate handlers. Commands are grouped into submodules:
--   - commands.nav:       Navigation (rooms, join, leave, go, exits, look, who)
--   - commands.room:      Room management (create, fork, vibe, nav)
--   - commands.inventory: Inventory system (inv, equip, unequip)
--   - commands.mcp:       MCP tools (mcp, tools, run)
--
-- Commands that display content use page.show() directly. Commands returning
-- quick feedback use: {text = "...", mode = "notification"}

local fun = require('fun')

local M = {}

-- ============================================================================
-- Load submodules
-- ============================================================================

-- Navigation commands (rooms, join, leave, go, exits, look, who)
local nav = require("commands.nav")

-- Room management commands (create, fork, vibe, dig, nav)
local room = require("commands.room")

-- Inventory commands (inv, equip, unequip)
local inventory = require("commands.inventory")

-- MCP commands (mcp, tools, run)
local mcp = require("commands.mcp")

-- History commands (history, history --tools, history --stats)
local history = require("commands.history")

-- Debug commands (wrap)
local debug = require("commands.debug")

-- Reload commands (reload, reload default)
local reload = require("commands.reload")

-- Conjure commands (conjure, unconjure)
local conjure = require("commands.conjure")

-- ============================================================================
-- System commands (inline implementations)
-- ============================================================================

local function cmd_help(args)
    local page = require('page')

    -- Check if a topic was provided
    local topic = args and args:match("%S+")

    if topic then
        -- Topic help: delegate to help module
        local help_lib = require('help')
        local content, err = help_lib.help(topic)
        if err or not content then
            return {
                text = err or ("No help found for: " .. topic),
                mode = "notification"
            }
        end
        page.show("Help: " .. topic, content)
        return {}
    end

    -- General command help
    local help_text = [[
Navigation:
  /rooms              List rooms
  /join <room>        Enter a room
  /leave              Return to lobby
  /create <name>      New room
  /go <direction>     Navigate via exit
  /exits              List exits from room
  /fork <name>        Fork room (inherit context)

Looking:
  /look               Room summary
  /who                Who's online
  /history [n]        Recent messages
  /history --tools    Tool call history
  /history --stats    Tool usage statistics

Room Context:
  /vibe [text]        Set/view room vibe
  /nav [on|off]       Toggle model navigation
  /portal <dir> <room>  Create exit to another room

Inventory:
  /inv [target]       Show contents (me, room, shared, @agent)
  /take <thing>       Copy thing into your inventory
  /drop <thing>       Move thing to current room
  /destroy owner:name Delete a thing

Equipment:
  /equip <ctx> <thing>    Equip tool (me, room, @agent)
  /unequip <ctx> <thing>  Unequip tool

Communication:
  <text>              Say to room
  @model <msg>        Message a model

Tools:
  /tools              List available tools
  /run <tool> [args]  Invoke tool with JSON args

MCP:
  /mcp                List connected MCP servers
  /mcp connect <name> <url>  Connect to MCP server
  /mcp disconnect <name>     Disconnect from server
  /mcp refresh <name>        Refresh tool list

UI:
  /reload             Reload UI from database
  /reload default     Reset to embedded default UI
  /reload <module>    Reload specific module

Help Topics:
  /help fun           Luafun functional programming
  /help str           String utilities
  /help inspect       Table pretty-printing
  /help tools         MCP tool reference
  /help room          Room navigation, vibes

/quit to disconnect
]]

    page.show("Help", help_text)
    return {}
end

local function cmd_quit(args)
    local page = require('page')
    page.show("Quit", "Goodbye!")
    return {}
end

local function cmd_clear(args)
    -- Clear is typically handled by the terminal layer
    -- Return empty response to signal success without visible output
    return {
        text = "",
        mode = "notification"
    }
end

-- ============================================================================
-- Handler dispatch table
-- ============================================================================

local handlers = {
    -- Navigation (from commands.nav)
    ["rooms"]   = nav.rooms,
    ["join"]    = nav.join,
    ["leave"]   = nav.leave,
    ["go"]      = nav.go,
    ["exits"]   = nav.exits,
    ["look"]    = nav.look,
    ["who"]     = nav.who,

    -- Room management (from commands.room)
    ["create"]  = room.create,
    ["fork"]    = room.fork,
    ["vibe"]    = room.vibe,
    ["portal"]  = room.portal,
    ["nav"]     = room.nav,

    -- Inventory (from commands.inventory)
    ["inv"]       = inventory.inv,
    ["inventory"] = inventory.inv,  -- alias
    ["take"]      = inventory.take,
    ["drop"]      = inventory.drop,
    ["destroy"]   = inventory.destroy,
    ["equip"]     = inventory.equip,
    ["unequip"]   = inventory.unequip,

    -- MCP (from commands.mcp)
    ["mcp"]   = mcp.mcp,
    ["tools"] = mcp.tools,
    ["run"]   = mcp.run,

    -- History (from commands.history)
    ["history"] = history.history,

    -- Debug (from commands.debug)
    ["wrap"] = debug.wrap,

    -- Reload (from commands.reload)
    ["reload"] = reload.reload,

    -- Conjure (from commands.conjure)
    ["conjure"]   = conjure.conjure,
    ["unconjure"] = conjure.unconjure,

    -- System (inline)
    ["help"]  = cmd_help,
    ["quit"]  = cmd_quit,
    ["clear"] = cmd_clear,
}

-- ============================================================================
-- Public API
-- ============================================================================

--- Dispatch a command by name
--- @param name string The command name (without leading slash)
--- @param args string The arguments string (may be empty)
--- @return table Action table with {text, mode, title?}
function M.dispatch(name, args)
    -- Check built-in handlers first
    local handler = handlers[name]
    if handler then
        local ok, result = pcall(handler, args or "")
        if ok then
            -- Ensure result has required fields
            result.text = result.text or ""
            result.mode = result.mode or "notification"
            return result
        else
            -- Handler threw an error
            return {
                text = "Error in /" .. name .. ": " .. tostring(result),
                mode = "notification"
            }
        end
    end

    -- Check for equipped command slot (e.g., command:fish)
    local slot_name = "command:" .. name
    local session = tools.session()

    -- Check agent equipment first, then room equipment
    local equipped = {}
    if session and session.agent_id then
        equipped = tools.get_agent_equipment(session.agent_id, slot_name) or {}
    end

    if #equipped == 0 and session and session.room_id then
        equipped = tools.get_room_equipment(session.room_id, slot_name) or {}
    end

    if #equipped > 0 then
        -- Found an equipped thing for this command slot
        local item = equipped[1]
        if item.code then
            -- Execute the thing's Lua code
            local result = tools.execute_code(item.code, args or "")
            if type(result) == "table" then
                result.text = result.text or ""
                result.mode = result.mode or "notification"
                return result
            else
                return {
                    text = tostring(result or ""),
                    mode = "notification"
                }
            end
        else
            return {
                text = string.format("Command /%s has no code", name),
                mode = "notification"
            }
        end
    end

    return {
        text = "Unknown command: /" .. name,
        mode = "notification"
    }
end

--- Check if a command exists
--- @param name string The command name
--- @return boolean
function M.exists(name)
    return handlers[name] ~= nil
end

--- Get list of all command names
--- @return table Array of command names (sorted)
function M.list()
    local names = fun.iter(handlers):map(function(k, _) return k end):totable()
    table.sort(names)
    return names
end

--- Register a new command handler
--- Used by submodules to extend the command set
--- @param name string The command name
--- @param handler function The handler function
function M.register(name, handler)
    handlers[name] = handler
end

--- Unregister a command handler
--- @param name string The command name
function M.unregister(name)
    handlers[name] = nil
end

return M
