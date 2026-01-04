-- sshwarma module bootstrap
--
-- Sets up the custom module loader before user scripts run.
-- Supports three tiers of module resolution:
--   1. Embedded modules (compiled into binary)
--   2. User modules (~/.config/sshwarma/lua/)
--   3. Standard package.path (system libs like penlight)
--
-- Module path resolution:
--   require("foo")        -> embedded/foo.lua OR ~/.config/sshwarma/lua/foo.lua
--   require("foo.bar")    -> embedded/foo/bar.lua OR ~/.config/sshwarma/lua/foo/bar.lua
--   require("commands")   -> embedded/commands/init.lua
--   require("pl.tablex")  -> standard package.path

-- Custom searcher for sshwarma modules
-- Inserted at position 2 (after preload, before standard path searchers)
local function sshwarma_searcher(modname)
    -- Try embedded modules first (always available, no io needed)
    if sshwarma and sshwarma.get_embedded_module then
        local embedded = sshwarma.get_embedded_module(modname)
        if embedded then
            local loader, err = load(embedded, "@embedded/" .. modname:gsub("%.", "/") .. ".lua")
            if loader then
                return loader, "embedded:" .. modname
            end
            return "\n\tcannot load embedded module '" .. modname .. "': " .. (err or "unknown error")
        end
    end

    -- Try user config directory (requires io library, not available in Luau)
    if io and io.open and sshwarma and sshwarma.config_path then
        local path = modname:gsub("%.", "/")

        -- Try direct file
        local user_path = sshwarma.config_path .. "/lua/" .. path .. ".lua"
        local f = io.open(user_path, "r")
        if f then
            local content = f:read("*a")
            f:close()
            local loader, err = load(content, "@" .. user_path)
            if loader then
                return loader, user_path
            end
            return "\n\tcannot load user module '" .. modname .. "': " .. (err or "unknown error")
        end

        -- Try init.lua for packages
        local init_path = sshwarma.config_path .. "/lua/" .. path .. "/init.lua"
        f = io.open(init_path, "r")
        if f then
            local content = f:read("*a")
            f:close()
            local loader, err = load(content, "@" .. init_path)
            if loader then
                return loader, init_path
            end
            return "\n\tcannot load user module '" .. modname .. "': " .. (err or "unknown error")
        end
    end

    -- Fall through to standard searchers
    return nil
end

-- Insert our custom searcher at position 2
-- Position 1 = package.preload
-- Position 2 = our custom searcher (embedded + user)
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
                return load(code, "@embedded/lib/inspect.lua")()
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
