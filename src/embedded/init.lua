-- sshwarma module bootstrap
--
-- Sets up the custom module loader before user scripts run.
-- Supports a virtual require system with namespace routing:
--
--   require("sshwarma.X")  -> embedded/X.lua (stable toolkit, always embedded)
--   require("room.X")      -> DB: room-scoped script X (graceful nil if missing)
--   require("X")           -> DB: user script first, then embedded fallback
--
-- Module path resolution:
--   require("sshwarma.ui.regions") -> embedded/ui/regions.lua
--   require("screen")              -> user DB "screen" OR embedded/screen.lua
--   require("room.tools")          -> room DB "tools" (nil if not found)

-- Custom searcher for sshwarma modules
-- Inserted at position 2 (after preload, before standard path searchers)
--
-- Note: Luau doesn't have `load()` - must use loadstring or have Rust pre-load modules
local load_fn = load or loadstring

local function sshwarma_searcher(modname)
    -- 1. "sshwarma.X" -> embedded only (stable toolkit)
    if modname:match("^sshwarma%.") then
        local embedded_name = modname:sub(10)  -- Remove "sshwarma." prefix
        if sshwarma and sshwarma.get_embedded_module then
            local code = sshwarma.get_embedded_module(embedded_name)
            if code then
                if load_fn then
                    local loader, err = load_fn(code, "@embedded/" .. embedded_name:gsub("%.", "/") .. ".lua")
                    if loader then
                        return loader, "embedded:" .. embedded_name
                    end
                    return "\n\tcannot load embedded '" .. embedded_name .. "': " .. (err or "?")
                elseif sshwarma.load_module then
                    return function() return sshwarma.load_module(embedded_name) end, "embedded:" .. embedded_name
                end
            end
        end
        return "\n\tno embedded module '" .. embedded_name .. "'"
    end

    -- 2. "room.X" -> current room's DB script (graceful nil on missing)
    if modname:match("^room%.") then
        local room_mod = modname:sub(6)  -- Remove "room." prefix
        if sshwarma and sshwarma.load_room_script then
            local code, err = sshwarma.load_room_script(room_mod)
            if code then
                if load_fn then
                    local loader, lerr = load_fn(code, "@room:" .. room_mod)
                    if loader then
                        return loader, "room:" .. room_mod
                    end
                    return "\n\tcannot load room module '" .. room_mod .. "': " .. (lerr or "?")
                end
            end
            -- Return nil for graceful handling - room module not found is not an error
            -- This allows users to check: local mod = require("room.tools") and use or ignore
            return nil
        end
        return nil  -- No loader available, graceful nil
    end

    -- 3. Plain "X" -> user DB first, then embedded fallback

    -- Try user DB script first
    if sshwarma and sshwarma.load_user_script then
        local code, err = sshwarma.load_user_script(modname)
        if code then
            if load_fn then
                local loader, lerr = load_fn(code, "@user:" .. modname)
                if loader then
                    return loader, "user:" .. modname
                end
                return "\n\tcannot load user script '" .. modname .. "': " .. (lerr or "?")
            end
        end
        -- If there was an error (not just missing), log it but continue to fallback
        -- err is nil if script just doesn't exist
    end

    -- Fall back to embedded
    if sshwarma and sshwarma.get_embedded_module then
        local embedded = sshwarma.get_embedded_module(modname)
        if embedded then
            if load_fn then
                local loader, lerr = load_fn(embedded, "@embedded/" .. modname:gsub("%.", "/") .. ".lua")
                if loader then
                    return loader, "embedded:" .. modname
                end
                return "\n\tcannot load embedded '" .. modname .. "': " .. (lerr or "?")
            elseif sshwarma.load_module then
                return function() return sshwarma.load_module(modname) end, "embedded:" .. modname
            end
        end
    end

    -- Try user config directory (filesystem fallback for development)
    if io and io.open and sshwarma and sshwarma.config_path then
        local path = modname:gsub("%.", "/")

        -- Try direct file
        local user_path = sshwarma.config_path .. "/lua/" .. path .. ".lua"
        local f = io.open(user_path, "r")
        if f then
            local content = f:read("*a")
            f:close()
            if load_fn then
                local loader, err = load_fn(content, "@" .. user_path)
                if loader then
                    return loader, user_path
                end
                return "\n\tcannot load user module '" .. modname .. "': " .. (err or "?")
            end
        end

        -- Try init.lua for packages
        local init_path = sshwarma.config_path .. "/lua/" .. path .. "/init.lua"
        f = io.open(init_path, "r")
        if f then
            local content = f:read("*a")
            f:close()
            if load_fn then
                local loader, err = load_fn(content, "@" .. init_path)
                if loader then
                    return loader, init_path
                end
                return "\n\tcannot load user module '" .. modname .. "': " .. (err or "?")
            end
        end
    end

    -- Fall through to standard searchers
    return nil
end

-- Insert our custom searcher at position 2
-- Position 1 = package.preload
-- Position 2 = our custom searcher (sshwarma namespace + DB + embedded)
-- Position 3+ = standard path searchers
-- Note: package.searchers may be nil in some Lua variants (Luau)
-- In that case, we skip installing the searcher - embedded modules still
-- work via sshwarma.get_embedded_module() direct calls
if package.searchers then
    table.insert(package.searchers, 2, sshwarma_searcher)
elseif package.loaders then
    -- Lua 5.1 compatibility
    table.insert(package.loaders, 2, sshwarma_searcher)
end

-- Preload inspect for convenience (very commonly used)
-- Note: package.preload may be nil in some Lua variants (Luau)
if package.preload then
    package.preload['inspect'] = function()
        if sshwarma and sshwarma.get_embedded_module then
            local code = sshwarma.get_embedded_module('inspect')
            if code then
                return load_fn(code, "@embedded/lib/inspect.lua")()
            end
        end
        error("inspect module not found in embedded modules")
    end
end

-- Add user lua directory to package.path for standard libs
if package.path and sshwarma and sshwarma.config_path then
    local user_lua = sshwarma.config_path .. "/lua/"
    package.path = user_lua .. "?.lua;" .. user_lua .. "?/init.lua;" .. package.path
end
