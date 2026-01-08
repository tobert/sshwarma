--- page.lua - Helper for commands to open pages
---
--- Commands that want to show content (previously "overlay" mode) should use
--- this module to directly open pages instead of returning mode = "overlay".
---
--- Copyright (c) 2025 Andrew Tobey
--- MIT License (see LICENSE)

local pages = require("ui.pages")

local M = {}

--- Open a page with content
--- @param title string Page title/name
--- @param content string Text content to display
function M.show(title, content)
    pages.open(title, content)
    -- tools is a global registered by Rust after module load
    local t = rawget(_G, "tools")
    if t and t.mark_dirty then
        t.mark_dirty("chat")
    end
end

--- Close the current page (return to chat)
function M.close()
    pages.close()
    local t = rawget(_G, "tools")
    if t and t.mark_dirty then
        t.mark_dirty("chat")
    end
end

return M
