-- commands/init.lua - Main command dispatcher for sshwarma
--
-- Provides a Lua-based command handler that maps slash commands to
-- appropriate handlers. Commands are grouped into submodules:
--   - commands.nav:       Navigation (rooms, join, leave, go, exits, look, who)
--   - commands.room:      Room management (create, fork, vibe, dig, nav)
--   - commands.inventory: Inventory system (inv, equip, unequip, bring, drop, examine)
--   - commands.journal:   Journal entries (journal, note, decide, idea, milestone, inspire)
--   - commands.mcp:       MCP tools (mcp, tools, run)
--
-- Each handler returns an action table:
--   {text = "...", mode = "notification"|"overlay", title = "..."}

local M = {}

-- ============================================================================
-- Load submodules
-- ============================================================================

-- Navigation commands (rooms, join, leave, go, exits, look, who)
local nav = require("commands.nav")

-- Room management commands (create, fork, vibe, dig, nav)
local room = require("commands.room")

-- Inventory commands (inv, equip, unequip, bring, drop, examine)
local inventory = require("commands.inventory")

-- Journal commands (journal, note, decide, idea, milestone, inspire)
local journal = require("commands.journal")

-- MCP commands (mcp, tools, run)
local mcp = require("commands.mcp")

-- History commands (history, history --tools, history --stats)
local history = require("commands.history")

-- Debug commands (wrap)
local debug = require("commands.debug")

-- Prompt commands (prompt list/show/push/pop/rm/insert/delete)
local prompt = require("commands.prompt")

-- Rules commands (rules list/add/del/enable/disable/scripts)
local rules = require("commands.rules")

-- ============================================================================
-- System commands (inline implementations)
-- ============================================================================

local function cmd_help(args)
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
  /examine <role>     Inspect bound asset
  /who                Who's online
  /history [n]        Recent messages
  /history --tools    Tool call history
  /history --stats    Tool usage statistics

Room Context:
  /vibe [text]        Set/view room vibe
  /note <text>        Add journal note
  /decide <text>      Record decision
  /idea <text>        Capture idea
  /milestone <text>   Mark milestone
  /journal [kind]     View journal entries
  /bring <id> as <role>  Bind artifact
  /drop <role>        Unbind asset
  /inspire <text>     Add to mood board
  /nav [on|off]       Toggle model navigation

Inventory:
  /inv                Show equipped tools in room
  /inv all            Include available (not equipped)
  /equip <thing>      Equip tool/data by qualified name
  /unequip <thing>    Unequip from room
  /portal <dir> <room>  Create exit to another room

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

/quit to disconnect
]]

    return {
        text = help_text,
        mode = "overlay",
        title = "Help"
    }
end

local function cmd_quit(args)
    return {
        text = "Goodbye!",
        mode = "overlay",
        title = "Quit"
    }
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
    ["equip"]     = inventory.equip,
    ["unequip"]   = inventory.unequip,
    ["bring"]     = inventory.bring,
    ["drop"]      = inventory.drop,
    ["examine"]   = inventory.examine,

    -- Journal (from commands.journal)
    ["journal"]   = journal.journal,
    ["note"]      = journal.note,
    ["decide"]    = journal.decide,
    ["idea"]      = journal.idea,
    ["milestone"] = journal.milestone,
    ["inspire"]   = journal.inspire,

    -- MCP (from commands.mcp)
    ["mcp"]   = mcp.mcp,
    ["tools"] = mcp.tools,
    ["run"]   = mcp.run,

    -- History (from commands.history)
    ["history"] = history.history,

    -- Debug (from commands.debug)
    ["wrap"] = debug.wrap,

    -- Prompt (from commands.prompt)
    ["prompt"] = prompt.prompt,

    -- Rules (from commands.rules)
    ["rules"] = rules.rules,

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
    else
        return {
            text = "Unknown command: /" .. name,
            mode = "notification"
        }
    end
end

--- Check if a command exists
--- @param name string The command name
--- @return boolean
function M.exists(name)
    return handlers[name] ~= nil
end

--- Get list of all command names
--- @return table Array of command names
function M.list()
    local names = {}
    for name, _ in pairs(handlers) do
        table.insert(names, name)
    end
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
