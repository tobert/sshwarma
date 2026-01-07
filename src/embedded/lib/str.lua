--- str.lua - Minimal string utilities for luau
---
--- Provides Python-style string operations without dependencies.
--- Designed to complement luafun for data transformation pipelines.
---
--- Copyright (c) 2025 Andrew Tobey
--- MIT License (see LICENSE)

local str = {}

--- Split a string by separator.
--- @param s string The string to split
--- @param sep string|nil Separator pattern (default: whitespace)
--- @param plain boolean|nil If true, sep is plain text not pattern
--- @return table Array of substrings
function str.split(s, sep, plain)
    if sep == nil or sep == "" then
        -- Split on whitespace, skip empty
        local result = {}
        for word in s:gmatch("%S+") do
            result[#result + 1] = word
        end
        return result
    end

    local result = {}
    local start = 1
    local sep_start, sep_end = s:find(sep, start, plain)

    while sep_start do
        result[#result + 1] = s:sub(start, sep_start - 1)
        start = sep_end + 1
        sep_start, sep_end = s:find(sep, start, plain)
    end

    result[#result + 1] = s:sub(start)
    return result
end

--- Split string into lines.
--- Handles \n, \r\n, and \r line endings.
--- @param s string The string to split
--- @return table Array of lines
function str.lines(s)
    local result = {}
    local pos = 1
    local len = #s

    while pos <= len do
        local nl = s:find("[\r\n]", pos)
        if nl then
            result[#result + 1] = s:sub(pos, nl - 1)
            -- Handle \r\n as single newline
            if s:sub(nl, nl) == "\r" and s:sub(nl + 1, nl + 1) == "\n" then
                pos = nl + 2
            else
                pos = nl + 1
            end
        else
            result[#result + 1] = s:sub(pos)
            break
        end
    end

    return result
end

--- Strip whitespace from both ends.
--- @param s string The string to strip
--- @return string Stripped string
function str.strip(s)
    return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

--- Strip whitespace from left end.
--- @param s string The string to strip
--- @return string Stripped string
function str.lstrip(s)
    return (s:gsub("^%s+", ""))
end

--- Strip whitespace from right end.
--- @param s string The string to strip
--- @return string Stripped string
function str.rstrip(s)
    return (s:gsub("%s+$", ""))
end

--- Check if string starts with prefix.
--- @param s string The string to check
--- @param prefix string The prefix to look for
--- @return boolean
function str.startswith(s, prefix)
    return s:sub(1, #prefix) == prefix
end

--- Check if string ends with suffix.
--- @param s string The string to check
--- @param suffix string The suffix to look for
--- @return boolean
function str.endswith(s, suffix)
    return suffix == "" or s:sub(-#suffix) == suffix
end

--- Pad string on the left to reach width.
--- @param s string The string to pad
--- @param width number Target width
--- @param char string|nil Padding character (default: space)
--- @return string Padded string
function str.lpad(s, width, char)
    char = char or " "
    local pad = width - #s
    if pad <= 0 then return s end
    return char:rep(pad) .. s
end

--- Pad string on the right to reach width.
--- @param s string The string to pad
--- @param width number Target width
--- @param char string|nil Padding character (default: space)
--- @return string Padded string
function str.rpad(s, width, char)
    char = char or " "
    local pad = width - #s
    if pad <= 0 then return s end
    return s .. char:rep(pad)
end

--- Center string to reach width.
--- @param s string The string to center
--- @param width number Target width
--- @param char string|nil Padding character (default: space)
--- @return string Centered string
function str.center(s, width, char)
    char = char or " "
    local pad = width - #s
    if pad <= 0 then return s end
    local left = math.floor(pad / 2)
    local right = pad - left
    return char:rep(left) .. s .. char:rep(right)
end

--- Join array of strings with separator.
--- @param tbl table Array of strings
--- @param sep string|nil Separator (default: empty string)
--- @return string Joined string
function str.join(tbl, sep)
    return table.concat(tbl, sep or "")
end

--- Check if string contains substring.
--- @param s string The string to search in
--- @param sub string The substring to find
--- @return boolean
function str.contains(s, sub)
    return s:find(sub, 1, true) ~= nil
end

--- Count occurrences of substring.
--- @param s string The string to search in
--- @param sub string The substring to count
--- @return number Count of occurrences
function str.count(s, sub)
    local count = 0
    local start = 1
    while true do
        local pos = s:find(sub, start, true)
        if not pos then break end
        count = count + 1
        start = pos + 1
    end
    return count
end

--- Replace occurrences of old with new.
--- @param s string The string to modify
--- @param old string Substring to replace
--- @param new string Replacement string
--- @param limit number|nil Max replacements (default: all)
--- @return string Modified string
--- @return number Number of replacements made
function str.replace(s, old, new, limit)
    -- Escape pattern special characters for plain replacement
    local escaped = old:gsub("([%(%)%.%%%+%-%*%?%[%]%^%$])", "%%%1")
    if limit then
        return s:gsub(escaped, new, limit)
    end
    return s:gsub(escaped, new)
end

--- Truncate string to max length, adding suffix if truncated.
--- @param s string The string to truncate
--- @param max number Maximum length
--- @param suffix string|nil Suffix to add if truncated (default: "...")
--- @return string Truncated string
function str.truncate(s, max, suffix)
    if #s <= max then return s end
    suffix = suffix or "..."
    return s:sub(1, max - #suffix) .. suffix
end

--- Wrap text to specified width.
--- @param s string The text to wrap
--- @param width number Maximum line width
--- @return string Wrapped text with newlines
function str.wrap(s, width)
    local lines = {}
    local line = ""

    for word in s:gmatch("%S+") do
        if #line == 0 then
            line = word
        elseif #line + 1 + #word <= width then
            line = line .. " " .. word
        else
            lines[#lines + 1] = line
            line = word
        end
    end

    if #line > 0 then
        lines[#lines + 1] = line
    end

    return table.concat(lines, "\n")
end

--- Check if string is empty or only whitespace.
--- @param s string The string to check
--- @return boolean
function str.isblank(s)
    return s:match("^%s*$") ~= nil
end

--- Convert to lowercase.
--- @param s string
--- @return string
function str.lower(s)
    return s:lower()
end

--- Convert to uppercase.
--- @param s string
--- @return string
function str.upper(s)
    return s:upper()
end

--- Capitalize first character.
--- @param s string
--- @return string
function str.capitalize(s)
    if #s == 0 then return s end
    return s:sub(1, 1):upper() .. s:sub(2)
end

--- Title case (capitalize each word).
--- @param s string
--- @return string
function str.title(s)
    return (s:gsub("(%w)(%w*)", function(first, rest)
        return first:upper() .. rest:lower()
    end))
end

--- Extract lines from text by range.
--- Useful for pulling sections from source code.
--- @param text string Source text
--- @param start_line number First line (1-indexed)
--- @param end_line number Last line (inclusive)
--- @return string Extracted lines joined with newlines
function str.extract_lines(text, start_line, end_line)
    local all_lines = str.lines(text)
    local result = {}
    local last = math.min(end_line, #all_lines)
    for i = start_line, last do
        result[#result + 1] = all_lines[i]
    end
    return str.join(result, "\n")
end

return str
