-- Rules command handlers for sshwarma
--
-- Commands for managing room rules (triggers and scripts).
-- Each handler receives args (string) and returns {text, mode, title?}

local M = {}

-- Format trigger kind for display
local function format_trigger(trigger)
    if not trigger then return "unknown" end
    if trigger.tick then
        return string.format("tick:%d", trigger.tick)
    elseif trigger.interval then
        return string.format("interval:%dms", trigger.interval)
    elseif trigger.row then
        return string.format("row:%s", trigger.row)
    elseif trigger.row_tag then
        return string.format("tag:%s", trigger.row_tag)
    end
    return "custom"
end

-- Parse trigger specification: tick:N, interval:Nms, row:pattern
local function parse_trigger(spec)
    local kind, value = spec:match("^(%w+):(.+)$")
    if not kind then
        return nil, "Invalid trigger format. Use tick:N, interval:Nms, or row:pattern"
    end

    if kind == "tick" then
        local n = tonumber(value)
        if not n then
            return nil, "tick requires a number (e.g., tick:1)"
        end
        return "tick", n
    elseif kind == "interval" then
        local ms = value:match("^(%d+)ms$")
        if ms then
            return "interval", tonumber(ms)
        end
        local n = tonumber(value)
        if n then
            return "interval", n
        end
        return nil, "interval requires milliseconds (e.g., interval:500ms)"
    elseif kind == "row" then
        return "row", value
    elseif kind == "tag" then
        return "row_tag", value
    else
        return nil, string.format("Unknown trigger kind '%s'. Use: tick, interval, row, tag", kind)
    end
end

-- /rules - Main rules command dispatcher
function M.rules(args)
    local trimmed = args:match("^%s*(.-)%s*$")

    if trimmed == "" then
        return M.rules_list("")
    end

    local subcmd, rest = trimmed:match("^(%S+)%s*(.*)")

    if subcmd == "add" then
        return M.rules_add_cmd(rest)
    elseif subcmd == "del" or subcmd == "delete" then
        return M.rules_del_cmd(rest)
    elseif subcmd == "enable" then
        return M.rules_enable_cmd(rest, true)
    elseif subcmd == "disable" then
        return M.rules_enable_cmd(rest, false)
    elseif subcmd == "scripts" then
        return M.rules_scripts("")
    else
        return {
            text = "Unknown subcommand. Use: add, del, enable, disable, scripts",
            mode = "notification"
        }
    end
end

-- /rules (list) - List room rules
function M.rules_list(args)
    local data = tools.rules()

    local lines = {}
    table.insert(lines, "=== Room Rules ===\r\n\r\n")

    local rules = data.rules or {}
    if #rules == 0 then
        table.insert(lines, "(no rules defined)\r\n\r\n")
    else
        for _, rule in ipairs(rules) do
            local status = rule.enabled and "" or ""
            local trigger = format_trigger(rule.trigger)
            table.insert(lines, string.format("%s [%s] %s -> %s\r\n",
                status, rule.id or "?", trigger, rule.script or "?"))
            if rule.name and rule.name ~= "" then
                table.insert(lines, string.format("    name: %s\r\n", rule.name))
            end
        end
        table.insert(lines, "\r\n")
    end

    table.insert(lines, "Commands:\r\n")
    table.insert(lines, "  /rules add <trigger> <script>  Add rule\r\n")
    table.insert(lines, "  /rules del <id>                Delete rule\r\n")
    table.insert(lines, "  /rules enable <id>             Enable rule\r\n")
    table.insert(lines, "  /rules disable <id>            Disable rule\r\n")
    table.insert(lines, "  /rules scripts                 List scripts\r\n")
    table.insert(lines, "\r\nTriggers: tick:N, interval:Nms, row:pattern, tag:name\r\n")

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Rules"
    }
end

-- /rules add <trigger> <script> - Add a new rule
function M.rules_add_cmd(args)
    local trigger_spec, script = args:match("^%s*(%S+)%s+(.-)%s*$")

    if not trigger_spec or not script or script == "" then
        return {
            text = "Usage: /rules add <trigger> <script>\r\n\r\n" ..
                   "Triggers:\r\n" ..
                   "  tick:N        Run on tick N (0-4 in 500ms cycle)\r\n" ..
                   "  interval:Nms  Run every N milliseconds\r\n" ..
                   "  row:pattern   Run when row matches pattern\r\n" ..
                   "  tag:name      Run when row has tag\r\n",
            mode = "notification"
        }
    end

    local trigger_kind, trigger_value = parse_trigger(trigger_spec)
    if not trigger_kind then
        return {
            text = string.format("Error: %s", trigger_value),
            mode = "notification"
        }
    end

    -- Build opts table for rules_add
    local opts = {}
    if trigger_kind == "tick" then
        opts.tick = trigger_value
    elseif trigger_kind == "interval" then
        opts.interval_ms = trigger_value
    elseif trigger_kind == "row" then
        opts.row_pattern = trigger_value
    elseif trigger_kind == "row_tag" then
        opts.row_tag = trigger_value
    end

    local result = tools.rules_add(trigger_kind, script, opts)

    if result.success then
        local id = result.rule_id or "(new)"
        return {
            text = string.format("Added rule [%s]: %s -> %s", id, trigger_spec, script),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "unknown error"),
            mode = "notification"
        }
    end
end

-- /rules del <id> - Delete a rule
function M.rules_del_cmd(args)
    local id = args:match("^%s*(.-)%s*$")

    if id == "" then
        return {
            text = "Usage: /rules del <rule_id>",
            mode = "notification"
        }
    end

    local result = tools.rules_del(id)

    if result.success then
        return {
            text = string.format("Deleted rule [%s]", id),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "rule not found"),
            mode = "notification"
        }
    end
end

-- /rules enable|disable <id> - Enable or disable a rule
function M.rules_enable_cmd(args, enabled)
    local id = args:match("^%s*(.-)%s*$")

    if id == "" then
        return {
            text = string.format("Usage: /rules %s <rule_id>", enabled and "enable" or "disable"),
            mode = "notification"
        }
    end

    local result = tools.rules_enable(id, enabled)

    if result.success then
        local action = enabled and "Enabled" or "Disabled"
        return {
            text = string.format("%s rule [%s]", action, id),
            mode = "notification"
        }
    else
        return {
            text = string.format("Error: %s", result.error or "rule not found"),
            mode = "notification"
        }
    end
end

-- /rules scripts - List available scripts
function M.rules_scripts(args)
    local data = tools.scripts()

    local lines = {}
    table.insert(lines, "=== Available Scripts ===\r\n\r\n")

    local scripts = data.scripts or {}
    if #scripts == 0 then
        table.insert(lines, "(no scripts available)\r\n\r\n")
        table.insert(lines, "Scripts are Lua functions that can be called by rules.\r\n")
        table.insert(lines, "Define them in your Lua configuration.\r\n")
    else
        for _, script in ipairs(scripts) do
            table.insert(lines, string.format("  %s", script.name))
            if script.kind then
                table.insert(lines, string.format(" (%s)", script.kind))
            end
            table.insert(lines, "\r\n")
            if script.description then
                table.insert(lines, string.format("    %s\r\n", script.description))
            end
        end
    end

    return {
        text = table.concat(lines),
        mode = "overlay",
        title = "Scripts"
    }
end

return M
