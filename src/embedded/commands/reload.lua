-- commands/reload.lua
--
-- UI reload commands
--
-- Uses the virtual require system to reload UI scripts from database.
-- The searcher in init.lua handles the DB → embedded fallback.

local M = {}

-- Clear user/room modules from package.loaded (keep system modules)
local function clear_user_modules()
    if not package or not package.loaded then return end

    for k in pairs(package.loaded) do
        -- Keep sshwarma.*, commands.*, ui.*, and inspect
        if not k:match("^sshwarma%.") and
           not k:match("^commands%.") and
           not k:match("^ui%.") and
           k ~= "inspect" then
            package.loaded[k] = nil
        end
    end
end

--- /reload [default]
--- Reloads the UI from database entrypoint or reverts to default
---@param args string Optional "default" to reset to embedded UI
---@return table Command result
function M.reload(args)
    if args == "default" then
        -- Reset to embedded default
        clear_user_modules()

        -- Load embedded screen directly via sshwarma namespace
        local ok, err = pcall(function()
            local screen = require("sshwarma.screen")
            if screen and screen.on_tick then
                on_tick = screen.on_tick
            end
        end)

        if ok then
            return { text = "Reverted to default UI", mode = "notification" }
        else
            return { text = "Failed to reset UI: " .. tostring(err), mode = "notification" }
        end
    elseif args and #args > 0 then
        -- Reload a specific module
        if package and package.loaded then
            package.loaded[args] = nil
        end

        local ok, result = pcall(require, args)
        if ok then
            return { text = "Reloaded: " .. args, mode = "notification" }
        else
            return { text = "Failed to reload '" .. args .. "': " .. tostring(result), mode = "notification" }
        end
    else
        -- Reload current entrypoint from DB
        clear_user_modules()

        -- Get entrypoint from Lua callback (which queries DB)
        -- The entrypoint is whatever module the user has set up
        -- If no custom entrypoint, just reload the default screen module
        local entrypoint = "screen"  -- Default

        -- Try to load the entrypoint module
        -- The searcher will check user DB first, then embedded
        local ok, m = pcall(require, entrypoint)
        if ok and type(m) == "table" and type(m.on_tick) == "function" then
            on_tick = m.on_tick
            return { text = "UI reloaded", mode = "notification" }
        elseif ok then
            return { text = "Module loaded but no on_tick function", mode = "notification" }
        else
            return { text = "Failed to reload UI: " .. tostring(m), mode = "notification" }
        end
    end
end

--- /reload help
function M.help()
    return {
        text = [[
/reload         - Reload UI (clears cache, reloads screen module)
/reload default - Reset to embedded default UI
/reload <mod>   - Reload specific module
]],
        mode = "display"
    }
end

return M
