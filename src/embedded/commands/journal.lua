-- sshwarma journal command handlers
--
-- Lua command handlers for journal/notes:
--   journal  - View journal entries
--   note     - Add a note
--   decide   - Record a decision
--   idea     - Capture an idea
--   milestone - Mark a milestone
--   inspire  - Add or view inspirations
--
-- Each handler receives args string and returns:
--   {text = "output", mode = "overlay"|"notification", title = "Title"}

local M = {}

--------------------------------------------------------------------------------
-- Helper: format timestamp
--------------------------------------------------------------------------------

local function format_timestamp(ts_ms)
    -- Convert milliseconds to approximate time string
    if not ts_ms then return "???" end

    -- For now just show relative or short format
    local now_ms = os.time() * 1000
    local diff_ms = now_ms - ts_ms

    if diff_ms < 60000 then
        return "just now"
    elseif diff_ms < 3600000 then
        local mins = math.floor(diff_ms / 60000)
        return string.format("%dm ago", mins)
    elseif diff_ms < 86400000 then
        local hours = math.floor(diff_ms / 3600000)
        return string.format("%dh ago", hours)
    else
        local days = math.floor(diff_ms / 86400000)
        return string.format("%dd ago", days)
    end
end

--------------------------------------------------------------------------------
-- Helper: kind to icon
--------------------------------------------------------------------------------

local function kind_icon(kind)
    local icons = {
        note = ".",
        decision = "!",
        idea = "*",
        milestone = "#",
    }
    return icons[kind] or "?"
end

--------------------------------------------------------------------------------
-- /journal [kind] - View journal entries
--------------------------------------------------------------------------------

function M.journal(args)
    local kind_filter = nil
    local limit = 20

    if args and not args:match("^%s*$") then
        -- Parse args: [kind] or [number] or [kind number]
        local word, num = args:match("^%s*(%a+)%s*(%d*)%s*$")
        if word then
            kind_filter = word
            if num and num ~= "" then
                limit = tonumber(num) or 20
            end
        else
            -- Try just a number
            local n = args:match("^%s*(%d+)%s*$")
            if n then
                limit = tonumber(n) or 20
            end
        end
    end

    local entries = tools.journal(kind_filter, limit)

    if not entries or #entries == 0 then
        local msg = "No journal entries"
        if kind_filter then
            msg = msg .. string.format(" of type '%s'", kind_filter)
        end
        return {
            text = msg .. ".",
            mode = "notification"
        }
    end

    local lines = {}
    table.insert(lines, "--- Journal ---")

    for _, entry in ipairs(entries) do
        local icon = kind_icon(entry.kind)
        local time = format_timestamp(entry.timestamp)
        local line = string.format("[%s] %s (%s): %s",
            icon, entry.kind, entry.author, entry.content)
        table.insert(lines, line)
    end

    return {
        text = table.concat(lines, "\n"),
        mode = "overlay",
        title = "Journal"
    }
end

--------------------------------------------------------------------------------
-- /note <text> - Add a note
--------------------------------------------------------------------------------

function M.note(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /note <text>",
            mode = "notification"
        }
    end

    local content = args:match("^%s*(.-)%s*$")  -- trim
    local result = tools.journal_add("note", content)

    if not result then
        return {
            text = "Error: Could not add note",
            mode = "notification"
        }
    end

    if result.success then
        return {
            text = string.format("[note] %s", content),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /decide <text> - Record a decision
--------------------------------------------------------------------------------

function M.decide(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /decide <text>",
            mode = "notification"
        }
    end

    local content = args:match("^%s*(.-)%s*$")  -- trim
    local result = tools.journal_add("decision", content)

    if not result then
        return {
            text = "Error: Could not record decision",
            mode = "notification"
        }
    end

    if result.success then
        return {
            text = string.format("[decision] %s", content),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /idea <text> - Capture an idea
--------------------------------------------------------------------------------

function M.idea(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /idea <text>",
            mode = "notification"
        }
    end

    local content = args:match("^%s*(.-)%s*$")  -- trim
    local result = tools.journal_add("idea", content)

    if not result then
        return {
            text = "Error: Could not capture idea",
            mode = "notification"
        }
    end

    if result.success then
        return {
            text = string.format("[idea] %s", content),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /milestone <text> - Mark a milestone
--------------------------------------------------------------------------------

function M.milestone(args)
    if not args or args:match("^%s*$") then
        return {
            text = "Usage: /milestone <text>",
            mode = "notification"
        }
    end

    local content = args:match("^%s*(.-)%s*$")  -- trim
    local result = tools.journal_add("milestone", content)

    if not result then
        return {
            text = "Error: Could not mark milestone",
            mode = "notification"
        }
    end

    if result.success then
        return {
            text = string.format("[milestone] %s", content),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

--------------------------------------------------------------------------------
-- /inspire [text] - Add or view inspirations
--------------------------------------------------------------------------------

function M.inspire(args)
    if not args or args:match("^%s*$") then
        -- View inspirations
        local result = tools.inspire()  -- no args = get list

        if not result then
            return {
                text = "Error: Could not get inspirations",
                mode = "notification"
            }
        end

        if not result.inspirations or #result.inspirations == 0 then
            return {
                text = "No inspirations yet. Use /inspire <text> to add one.",
                mode = "notification"
            }
        end

        local lines = {}
        table.insert(lines, "--- Inspirations ---")

        for _, content in ipairs(result.inspirations) do
            table.insert(lines, string.format(". %s", content))
        end

        return {
            text = table.concat(lines, "\n"),
            mode = "overlay",
            title = "Inspirations"
        }
    else
        -- Add inspiration
        local content = args:match("^%s*(.-)%s*$")  -- trim
        local result = tools.inspire(content)

        if not result then
            return {
                text = "Error: Could not add inspiration",
                mode = "notification"
            }
        end

        if result.success then
            return {
                text = string.format("Added inspiration: %s", content),
                mode = "notification"
            }
        else
            return {
                text = string.format("Error: %s", result.error or "unknown error"),
                mode = "notification"
            }
        end
    end
end

return M
