# 05: Command Dispatch and Handlers

**File:** `src/embedded/commands/*.lua`
**Focus:** Lua implementation of all slash commands
**Dependencies:** 01-require, 04-tools-api
**Unblocks:** 08-integration

---

## Task

Implement all slash commands in Lua. Commands call `tools.*` for operations and format output for display in regions.

**Why this task?** This is the core of "Lua owns UI" — all command logic moves from Rust to Lua.

**Deliverables:**
1. Command dispatch table
2. All commands from `src/commands.rs` reimplemented
3. Output formatting for regions (overlays, chat, status)
4. Help text generation

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- @mention handling — that's 07-streaming
- Chat message sending — that's 06-chat
- Input parsing — that's 02-input

Focus ONLY on slash command implementation.

---

## Command Dispatch

```lua
-- commands/init.lua

local M = {}

-- Import command modules
local nav = require 'commands.nav'
local inventory = require 'commands.inventory'
local journal = require 'commands.journal'
local context = require 'commands.context'
local mcp = require 'commands.mcp'
local prompt = require 'commands.prompt'
local rules = require 'commands.rules'

-- Dispatch table
local commands = {
    -- Navigation
    rooms = nav.rooms,
    join = nav.join,
    leave = nav.leave,
    create = nav.create,
    look = nav.look,
    who = nav.who,
    go = nav.go,
    exits = nav.exits,
    dig = nav.dig,
    fork = nav.fork,
    nav = nav.nav_toggle,

    -- Inventory
    inv = inventory.inv,
    inventory = inventory.inv,
    equip = inventory.equip,
    unequip = inventory.unequip,
    portal = inventory.portal,

    -- Journal
    journal = journal.list,
    note = journal.note,
    decide = journal.decide,
    idea = journal.idea,
    milestone = journal.milestone,

    -- Context
    vibe = context.vibe,
    bring = context.bring,
    drop = context.drop,
    examine = context.examine,
    inspire = context.inspire,
    history = context.history,

    -- MCP/Tools
    tools = mcp.list_tools,
    run = mcp.run,
    mcp = mcp.mcp_cmd,

    -- Prompts
    prompt = prompt.cmd,

    -- Rules
    rules = rules.cmd,

    -- Meta
    help = M.help,
    wrap = M.wrap,
    quit = M.quit,
}

--- Dispatch a command
---@param input string Full input line (e.g., "/join workshop")
---@return table {display, region} How to display result
function M.dispatch(input)
    -- Check if it's a command
    if not input:match("^/") then
        return nil  -- Not a command, handle as chat
    end

    -- Parse command and args
    local cmd, args = input:match("^/(%S+)%s*(.*)")
    if not cmd then
        return { text = "Invalid command", region = "overlay", title = "Error" }
    end

    -- Look up handler
    local handler = commands[cmd:lower()]
    if not handler then
        return {
            text = "Unknown command: /" .. cmd,
            region = "overlay",
            title = "Error"
        }
    end

    -- Call handler
    local ok, result = pcall(handler, args)
    if not ok then
        return {
            text = "Command error: " .. tostring(result),
            region = "overlay",
            title = "Error"
        }
    end

    return result
end

--- Help command
function M.help()
    local text = [[
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

Context:
  /vibe [text]        Get or set vibe
  /note <text>        Add note to journal
  /decide <text>      Record decision
  /idea <text>        Capture idea
  /milestone <text>   Mark milestone
  /journal [kind]     View journal
  /bring <id> as <role>   Bind artifact
  /drop <role>        Unbind artifact
  /inspire [text]     Get or add inspiration

Inventory:
  /inv [all]          Show equipped (all shows available)
  /equip <name>       Equip tool for room
  /unequip <name>     Remove tool from room
  /portal <server>    Create exit to MCP server

Tools:
  /tools              List MCP tools
  /run <tool> [json]  Run tool directly
  /mcp                MCP server status

Prompts:
  /prompt             List prompts
  /prompt set <n> <t> Set prompt
  /prompt push <t> <n> Push to stack
  /prompt pop <t>     Pop from stack

Rules:
  /rules              List room rules
  /rules add ...      Add rule
  /rules del <id>     Delete rule

System:
  /help               This help
  /quit               Disconnect
]]
    return { text = text, region = "overlay", title = "Help" }
end

--- Wrap preview command
function M.wrap()
    local result = tools.wrap_preview()
    return { text = result, region = "overlay", title = "Wrap Preview" }
end

--- Quit command
function M.quit()
    tools.disconnect()
    return { text = "Goodbye!", region = "status" }
end

return M
```

---

## Navigation Commands

```lua
-- commands/nav.lua

local M = {}
local regions = require 'ui.regions'

function M.rooms()
    local result = tools.rooms()
    local lines = {"Rooms:\n"}
    for _, room in ipairs(result.rooms) do
        local desc = room.description and (" - " .. room.description) or ""
        table.insert(lines, string.format("  %s (%d users, %d models)%s",
            room.name, room.user_count, room.model_count, desc))
    end
    return { text = table.concat(lines, "\n"), region = "overlay", title = "Rooms" }
end

function M.join(args)
    local room_name = args:match("^%s*(%S+)")
    if not room_name then
        return { text = "Usage: /join <room>", region = "overlay", title = "Error" }
    end

    local result = tools.join(room_name)
    if result.success then
        return M.look()  -- Show room info after joining
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.leave()
    local result = tools.leave()
    if result.success then
        return { text = "Left room, now in lobby", region = "overlay", title = "Leave" }
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.create(args)
    local name, desc = args:match("^%s*(%S+)%s*(.*)")
    if not name then
        return { text = "Usage: /create <name> [description]", region = "overlay", title = "Error" }
    end

    local result = tools.create(name, desc ~= "" and desc or nil)
    if result.success then
        tools.join(name)  -- Auto-join created room
        return M.look()
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.look()
    local info = tools.look()
    local lines = {}

    table.insert(lines, "Room: " .. (info.room and info.room.name or "???"))
    if info.vibe and info.vibe ~= "" then
        table.insert(lines, "Vibe: " .. info.vibe)
    end

    if info.users and #info.users > 0 then
        table.insert(lines, "\nUsers:")
        for _, user in ipairs(info.users) do
            table.insert(lines, "  " .. user.name)
        end
    end

    if info.models and #info.models > 0 then
        table.insert(lines, "\nModels:")
        for _, model in ipairs(info.models) do
            table.insert(lines, "  " .. model.name)
        end
    end

    if info.exits and #info.exits > 0 then
        table.insert(lines, "\nExits:")
        for _, exit in ipairs(info.exits) do
            table.insert(lines, string.format("  %s -> %s", exit.direction, exit.target_room))
        end
    end

    return { text = table.concat(lines, "\n"), region = "overlay", title = "Look" }
end

function M.who()
    local result = tools.who()
    local lines = {"Online:\n"}
    for _, user in ipairs(result.users) do
        table.insert(lines, "  " .. user.name .. " (" .. user.status .. ")")
    end
    return { text = table.concat(lines, "\n"), region = "overlay", title = "Who" }
end

function M.go(args)
    local direction = args:match("^%s*(%S+)")
    if not direction then
        return { text = "Usage: /go <direction>", region = "overlay", title = "Error" }
    end

    local result = tools.go(direction)
    if result.success then
        return M.look()
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.exits()
    local result = tools.exits()
    if #result.exits == 0 then
        return { text = "No exits from this room", region = "overlay", title = "Exits" }
    end

    local lines = {"Exits:\n"}
    for _, exit in ipairs(result.exits) do
        table.insert(lines, string.format("  %s -> %s", exit.direction, exit.target_room))
    end
    return { text = table.concat(lines, "\n"), region = "overlay", title = "Exits" }
end

function M.dig(args)
    local dir, target = args:match("^%s*(%S+)%s+(%S+)")
    if not dir or not target then
        return { text = "Usage: /dig <direction> <target-room>", region = "overlay", title = "Error" }
    end

    local result = tools.dig(dir, target, true)
    if result.success then
        return { text = string.format("Dug exit: %s -> %s", dir, target), region = "overlay", title = "Dig" }
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.fork(args)
    local name = args:match("^%s*(%S+)")
    if not name then
        return { text = "Usage: /fork <new-name>", region = "overlay", title = "Error" }
    end

    local result = tools.fork(name)
    if result.success then
        tools.join(name)
        return M.look()
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.nav_toggle(args)
    local setting = args:match("^%s*(%S+)")
    if setting then
        local result = tools.set_nav(setting == "on")
        return { text = "Navigation " .. (result.enabled and "enabled" or "disabled"), region = "overlay", title = "Navigation" }
    else
        local result = tools.get_nav()
        return { text = "Navigation is " .. (result.enabled and "enabled" or "disabled"), region = "overlay", title = "Navigation" }
    end
end

return M
```

---

## Inventory Commands

```lua
-- commands/inventory.lua

local M = {}

function M.inv(args)
    local show_all = args:match("all")
    local inv = tools.inventory()

    local lines = {}

    if #inv.equipped > 0 then
        table.insert(lines, "Equipped:")
        for _, item in ipairs(inv.equipped) do
            local desc = item.description and (" - " .. item.description) or ""
            table.insert(lines, "  ● " .. item.qualified_name .. desc)
        end
    else
        table.insert(lines, "No tools equipped")
    end

    if show_all and #inv.available > 0 then
        table.insert(lines, "\nAvailable to equip:")
        for _, item in ipairs(inv.available) do
            local desc = item.description and (" - " .. item.description) or ""
            table.insert(lines, "  ○ " .. item.qualified_name .. desc)
        end
    end

    return { text = table.concat(lines, "\n"), region = "overlay", title = "Inventory" }
end

function M.equip(args)
    local name = args:match("^%s*(%S+)")
    if not name then
        return { text = "Usage: /equip <qualified-name>", region = "overlay", title = "Error" }
    end

    local result = tools.equip(name)
    if result.success then
        local lines = {}
        if #result.added > 0 then
            table.insert(lines, "Added:")
            for _, n in ipairs(result.added) do
                table.insert(lines, "  + " .. n)
            end
        end
        table.insert(lines, "\nEquipped:")
        for i, item in ipairs(result.equipped) do
            local marker = result.added_set and result.added_set[item.qualified_name] and "● " or "  "
            table.insert(lines, marker .. item.qualified_name)
        end
        return { text = table.concat(lines, "\n"), region = "overlay", title = "Equip" }
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.unequip(args)
    local name = args:match("^%s*(%S+)")
    if not name then
        return { text = "Usage: /unequip <qualified-name>", region = "overlay", title = "Error" }
    end

    local result = tools.unequip(name)
    if result.success then
        local lines = {"Removed: " .. (result.removed or name)}
        if #result.equipped > 0 then
            table.insert(lines, "\nStill equipped:")
            for _, item in ipairs(result.equipped) do
                table.insert(lines, "  " .. item.qualified_name)
            end
        end
        return { text = table.concat(lines, "\n"), region = "overlay", title = "Unequip" }
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

function M.portal(args)
    local server, direction = args:match("^%s*(%S+)%s*(%S*)")
    if not server then
        return { text = "Usage: /portal <server> [direction]", region = "overlay", title = "Error" }
    end

    direction = direction ~= "" and direction or server
    local result = tools.portal(server, direction)
    if result.success then
        return { text = "Created portal: " .. direction .. " -> " .. server, region = "overlay", title = "Portal" }
    else
        return { text = result.error, region = "overlay", title = "Error" }
    end
end

return M
```

---

## Acceptance Criteria

- [ ] `/help` shows help text in overlay
- [ ] `/rooms` lists rooms
- [ ] `/join <room>` joins and shows room info
- [ ] `/leave` returns to lobby
- [ ] `/look` shows room details
- [ ] `/inv` shows equipped tools
- [ ] `/inv all` shows available tools too
- [ ] `/equip <name>` equips and shows delta
- [ ] `/unequip <name>` removes and shows remaining
- [ ] All journal commands work
- [ ] All navigation commands work
- [ ] `/tools` lists MCP tools
- [ ] `/run <tool>` executes tool
- [ ] Unknown commands show error
- [ ] Argument validation shows usage hints
