-- inspect.lua - Human-readable representation of Lua tables
-- Based on kikito/inspect.lua (https://github.com/kikito/inspect.lua)
-- This is a minimal implementation for sshwarma's needs.
--
-- MIT LICENSE
-- Copyright (c) 2022 Enrique Garcia Cota
--
-- Permission is hereby granted, free of charge, to any person obtaining a
-- copy of this software and associated documentation files (the
-- "Software"), to deal in the Software without restriction, including
-- without limitation the rights to use, copy, modify, merge, publish,
-- distribute, sublicense, and/or sell copies of the Software, and to
-- permit persons to whom the Software is furnished to do so, subject to
-- the following conditions:
--
-- The above copyright notice and this permission notice shall be included
-- in all copies or substantial portions of the Software.
--
-- THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
-- OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
-- MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
-- IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
-- CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
-- TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
-- SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

local inspect = { Options = {} }

inspect._VERSION = 'inspect.lua 3.1.0 (minimal)'
inspect._URL = 'http://github.com/kikito/inspect.lua'
inspect._DESCRIPTION = 'human-readable representations of tables'

-- Sentinel values for special cases
inspect.KEY = setmetatable({}, { __tostring = function() return 'inspect.KEY' end })
inspect.METATABLE = setmetatable({}, { __tostring = function() return 'inspect.METATABLE' end })

local tostring = tostring
local rep = string.rep
local match = string.match
local gsub = string.gsub
local fmt = string.format
local floor = math.floor

-- Quote a string appropriately
local function smartQuote(str)
    if match(str, '"') and not match(str, "'") then
        return "'" .. str .. "'"
    end
    return '"' .. gsub(str, '"', '\\"') .. '"'
end

-- Escape control characters
local function escape(str)
    return (gsub(gsub(str, "\\", "\\\\"), "%c", function(c)
        return fmt("\\%03d", c:byte())
    end))
end

-- Lua reserved words
local luaKeywords = {
    ['and'] = true, ['break'] = true, ['do'] = true, ['else'] = true,
    ['elseif'] = true, ['end'] = true, ['false'] = true, ['for'] = true,
    ['function'] = true, ['goto'] = true, ['if'] = true, ['in'] = true,
    ['local'] = true, ['nil'] = true, ['not'] = true, ['or'] = true,
    ['repeat'] = true, ['return'] = true, ['then'] = true, ['true'] = true,
    ['until'] = true, ['while'] = true,
}

-- Check if string is a valid identifier
local function isIdentifier(str)
    return type(str) == "string"
        and not not str:match("^[_%a][_%a%d]*$")
        and not luaKeywords[str]
end

-- Check if key is a sequence key
local function isSequenceKey(k, sequenceLength)
    return type(k) == "number"
        and floor(k) == k
        and 1 <= k
        and k <= sequenceLength
end

-- Type order for sorting keys
local defaultTypeOrders = {
    ['number'] = 1, ['boolean'] = 2, ['string'] = 3, ['table'] = 4,
    ['function'] = 5, ['userdata'] = 6, ['thread'] = 7,
}

-- Sort keys by type then value
local function sortKeys(a, b)
    local ta, tb = type(a), type(b)
    if ta == tb and (ta == 'string' or ta == 'number') then
        return a < b
    end
    local dta = defaultTypeOrders[ta] or 100
    local dtb = defaultTypeOrders[tb] or 100
    return dta == dtb and ta < tb or dta < dtb
end

-- Get all non-sequence keys from a table
local function getKeys(t)
    local seqLen = 1
    while rawget(t, seqLen) ~= nil do
        seqLen = seqLen + 1
    end
    seqLen = seqLen - 1

    local keys = {}
    for k in pairs(t) do
        if not isSequenceKey(k, seqLen) then
            keys[#keys + 1] = k
        end
    end
    table.sort(keys, sortKeys)

    return keys, #keys, seqLen
end

-- Count cycles in the table structure
local function countCycles(x, cycles)
    if type(x) == 'table' then
        if cycles[x] then
            cycles[x] = cycles[x] + 1
        else
            cycles[x] = 1
            for k, v in pairs(x) do
                countCycles(k, cycles)
                countCycles(v, cycles)
            end
            local mt = getmetatable(x)
            if type(mt) == 'table' then
                countCycles(mt, cycles)
            end
        end
    end
end

-- Inspector class
local Inspector = {}
local Inspector_mt = { __index = Inspector }

function Inspector:getId(v)
    local id = self.ids[v]
    if not id then
        id = self.maxId + 1
        self.maxId = id
        self.ids[v] = id
    end
    return id
end

function Inspector:puts(str)
    self.buf[#self.buf + 1] = str
end

function Inspector:tabify()
    self:puts(self.newline .. rep(self.indent, self.level))
end

function Inspector:putValue(v)
    local tv = type(v)

    if tv == 'string' then
        self:puts(smartQuote(escape(v)))
    elseif tv == 'number' or tv == 'boolean' or tv == 'nil' then
        self:puts(tostring(v))
    elseif tv == 'table' then
        local t = v
        if self.level >= self.depth then
            self:puts('{...}')
        elseif self.cycles[t] and self.cycles[t] > 1 then
            if self.seenTables[t] then
                self:puts(fmt('<table %d>', self:getId(t)))
                return
            end
            self.seenTables[t] = true
            self:puts(fmt('<%d>', self:getId(t)))
            self:putTable(t)
        else
            self:putTable(t)
        end
    else
        self:puts(fmt('<%s %d>', tv, self:getId(v)))
    end
end

function Inspector:putTable(t)
    local keys, keysLen, seqLen = getKeys(t)

    self:puts('{')
    self.level = self.level + 1

    for i = 1, seqLen + keysLen do
        if i > 1 then self:puts(',') end
        if i <= seqLen then
            self:puts(' ')
            self:putValue(t[i])
        else
            local k = keys[i - seqLen]
            self:tabify()
            if isIdentifier(k) then
                self:puts(k)
            else
                self:puts("[")
                self:putValue(k)
                self:puts("]")
            end
            self:puts(' = ')
            self:putValue(t[k])
        end
    end

    local mt = getmetatable(t)
    if type(mt) == 'table' then
        if seqLen + keysLen > 0 then self:puts(',') end
        self:tabify()
        self:puts('<metatable> = ')
        self:putValue(mt)
    end

    self.level = self.level - 1

    if keysLen > 0 or type(mt) == 'table' then
        self:tabify()
    elseif seqLen > 0 then
        self:puts(' ')
    end

    self:puts('}')
end

-- Main inspect function
function inspect.inspect(root, options)
    options = options or {}

    local depth = options.depth or math.huge
    local newline = options.newline or '\n'
    local indent = options.indent or '  '

    local cycles = {}
    countCycles(root, cycles)

    local inspector = setmetatable({
        buf = {},
        ids = {},
        maxId = 0,
        cycles = cycles,
        seenTables = {},
        depth = depth,
        level = 0,
        newline = newline,
        indent = indent,
    }, Inspector_mt)

    inspector:putValue(root)

    return table.concat(inspector.buf)
end

-- Allow calling inspect directly
setmetatable(inspect, {
    __call = function(_, root, options)
        return inspect.inspect(root, options)
    end,
})

return inspect
