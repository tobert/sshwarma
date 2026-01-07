-- ui/pages.lua - Page stack management
--
-- Pages are ephemeral views within the current room.
-- Chat is always the base layer (index 1), other pages stack on top.
--
-- Navigation:
--   pages.nav_left()  / h / left arrow  -> previous page
--   pages.nav_right() / l / right arrow -> next page
--   pages.close()     / q               -> close current page (returns to chat)
--
-- Pages vs Rooms:
--   - Rooms are places (MUD metaphor) - shared state, you /join them
--   - Pages are local views - help, journal, search results, etc.

local M = {}

-- Page stack: array of pages
-- Index 1 is always "chat" (the base layer)
-- Other pages are tables: {name, content, scroll}
local pages = { "chat" }

-- Current page index (1-based)
local current_idx = 1

-- ==========================================================================
-- Page Access
-- ==========================================================================

--- Get the current page
---@return string|table page name or page table
function M.current()
    return pages[current_idx]
end

--- Get the current page name
---@return string
function M.current_name()
    local p = pages[current_idx]
    if type(p) == "table" then
        return p.name
    else
        return p
    end
end

--- Check if currently on a specific page
---@param name string
---@return boolean
function M.is_current(name)
    return M.current_name() == name
end

--- Check if on the chat page
---@return boolean
function M.is_chat()
    return current_idx == 1
end

--- Get all pages as array
---@return table
function M.list()
    return pages
end

--- Get page count
---@return number
function M.count()
    return #pages
end

--- Get current index
---@return number
function M.index()
    return current_idx
end

-- ==========================================================================
-- Page Navigation
-- ==========================================================================

--- Navigate to previous page (left)
---@return boolean true if moved
function M.nav_left()
    if current_idx > 1 then
        current_idx = current_idx - 1
        return true
    end
    return false
end

--- Navigate to next page (right)
---@return boolean true if moved
function M.nav_right()
    if current_idx < #pages then
        current_idx = current_idx + 1
        return true
    end
    return false
end

--- Jump to a specific page by name
---@param name string
---@return boolean true if found and switched
function M.goto(name)
    for i, p in ipairs(pages) do
        local pname = type(p) == "table" and p.name or p
        if pname == name then
            current_idx = i
            return true
        end
    end
    return false
end

--- Jump to chat
function M.goto_chat()
    current_idx = 1
end

-- ==========================================================================
-- Page Management
-- ==========================================================================

--- Open a page (creates if not exists, switches to it)
---@param name string page name
---@param content any optional content for the page
---@return table page
function M.open(name, content)
    -- Check if page already exists
    for i, p in ipairs(pages) do
        if type(p) == "table" and p.name == name then
            current_idx = i
            -- Update content if provided
            if content then
                p.content = content
            end
            return p
        end
    end

    -- Create new page
    local page = {
        name = name,
        content = content,
        scroll = 0,
    }
    table.insert(pages, page)
    current_idx = #pages
    return page
end

--- Close current page (returns to chat if on non-chat page)
---@return boolean true if a page was closed
function M.close()
    if current_idx > 1 then
        table.remove(pages, current_idx)
        current_idx = math.min(current_idx, #pages)
        return true
    end
    return false
end

--- Close a specific page by name
---@param name string
---@return boolean true if found and closed
function M.close_by_name(name)
    for i, p in ipairs(pages) do
        if type(p) == "table" and p.name == name then
            table.remove(pages, i)
            -- Adjust current_idx if needed
            if current_idx > i then
                current_idx = current_idx - 1
            elseif current_idx == i then
                current_idx = math.max(1, math.min(current_idx, #pages))
            end
            return true
        end
    end
    return false
end

--- Close all pages except chat
function M.close_all()
    for i = #pages, 2, -1 do
        table.remove(pages, i)
    end
    current_idx = 1
end

-- ==========================================================================
-- Page Content Access
-- ==========================================================================

--- Get content of current page
---@return any
function M.content()
    local p = pages[current_idx]
    if type(p) == "table" then
        return p.content
    end
    return nil
end

--- Set content of current page
---@param content any
function M.set_content(content)
    local p = pages[current_idx]
    if type(p) == "table" then
        p.content = content
    end
end

--- Get scroll position of current page
---@return number
function M.scroll_pos()
    local p = pages[current_idx]
    if type(p) == "table" then
        return p.scroll or 0
    end
    return 0
end

--- Set scroll position of current page
---@param pos number
function M.set_scroll_pos(pos)
    local p = pages[current_idx]
    if type(p) == "table" then
        p.scroll = pos
    end
end

-- ==========================================================================
-- Reset (for room changes)
-- ==========================================================================

--- Reset page stack (on room change)
function M.reset()
    pages = { "chat" }
    current_idx = 1
end

return M
