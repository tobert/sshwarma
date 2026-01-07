# inspect.lua — Quick Reference

Human-readable Lua table serialization. Great for debugging.

**Source:** `require 'inspect'` → [kikito/inspect.lua](https://github.com/kikito/inspect.lua)

## Basic Usage

```lua
local inspect = require 'inspect'

-- Simple call
print(inspect({1, 2, 3}))
-- { 1, 2, 3 }

print(inspect({name = "alice", level = 5}))
-- {
--   level = 5,
--   name = "alice"
-- }
```

## Nested Tables

```lua
local data = {
    users = {
        {name = "alice", active = true},
        {name = "bob", active = false},
    },
    count = 2
}
print(inspect(data))
-- {
--   count = 2,
--   users = {
--     {
--       active = true,
--       name = "alice"
--     }, {
--       active = false,
--       name = "bob"
--     }
--   }
-- }
```

## Options

```lua
-- Limit depth
inspect(deep_table, {depth = 2})
-- Nested tables beyond depth show as {...}

-- Custom indentation
inspect(data, {indent = "    "})  -- 4 spaces instead of 2

-- Custom newline (for single-line output)
inspect(data, {newline = " "})
```

## Cycle Detection

```lua
local t = {a = 1}
t.self = t  -- circular reference

print(inspect(t))
-- <1>{
--   a = 1,
--   self = <table 1>
-- }
```

## Special Values

```lua
inspect(nil)              -- nil
inspect(true)             -- true
inspect(42)               -- 42
inspect("hello")          -- "hello"
inspect(function() end)   -- <function 1>
inspect(coroutine.create(function() end))  -- <thread 1>
```

## Metatables

```lua
local mt = {__index = {default = 0}}
local t = setmetatable({value = 1}, mt)

print(inspect(t))
-- {
--   value = 1,
--   <metatable> = {
--     __index = {
--       default = 0
--     }
--   }
-- }
```

## Common Patterns

### Debug logging
```lua
local function debug(label, value)
    print(label .. ": " .. inspect(value))
end

debug("state", {room = "lobby", users = {"alice", "bob"}})
```

### Compact single-line
```lua
local function compact(t)
    return inspect(t, {newline = " ", indent = ""})
end
```

### Safe tostring
```lua
local function safe_tostring(v)
    if type(v) == "table" then
        return inspect(v, {depth = 1})
    end
    return tostring(v)
end
```

## Gotchas

1. **Large tables** — no built-in truncation; use `depth` option
2. **Functions** — shows as `<function N>`, not source code
3. **Userdata** — shows as `<userdata N>`
4. **Not reversible** — output is for humans, not `loadstring`

## See Also

- Full source: `require 'inspect'` (~270 lines)
- Original repo: https://github.com/kikito/inspect.lua
