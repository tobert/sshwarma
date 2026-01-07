# str.lua — Quick Reference

Minimal Python-style string utilities for luau.

**Source:** `require 'str'` — sshwarma built-in

## Splitting

```lua
local str = require 'str'

str.split("a,b,c", ",")           -- {"a", "b", "c"}
str.split("a,,c", ",")            -- {"a", "", "c"}
str.split("hello world")          -- {"hello", "world"} (whitespace)
str.split("a.b.c", ".", true)     -- {"a", "b", "c"} (plain, not pattern)

str.lines("a\nb\nc")              -- {"a", "b", "c"}
str.lines("a\r\nb\nc")            -- {"a", "b", "c"} (handles \r\n)
```

## Trimming

```lua
str.strip("  hello  ")            -- "hello"
str.lstrip("  hello  ")           -- "hello  "
str.rstrip("  hello  ")           -- "  hello"
```

## Padding & Alignment

```lua
str.lpad("42", 5)                 -- "   42"
str.lpad("42", 5, "0")            -- "00042"
str.rpad("hi", 5)                 -- "hi   "
str.center("hi", 6)               -- "  hi  "
```

## Testing

```lua
str.startswith("hello", "he")     -- true
str.endswith("hello", "lo")       -- true
str.contains("hello", "ell")      -- true
str.isblank("   ")                -- true
str.isblank("")                   -- true
```

## Searching & Counting

```lua
str.contains("hello world", "wor")  -- true
str.count("ababa", "ab")            -- 2
```

## Transforming

```lua
str.replace("hello", "l", "L")    -- "heLLo", 2
str.replace("hello", "l", "L", 1) -- "heLlo", 1

str.lower("HELLO")                -- "hello"
str.upper("hello")                -- "HELLO"
str.capitalize("hello")           -- "Hello"
str.title("hello world")          -- "Hello World"

str.truncate("hello world", 8)    -- "hello..."
str.truncate("hello world", 8, "…")  -- "hello w…"
```

## Joining

```lua
str.join({"a", "b", "c"}, ",")    -- "a,b,c"
str.join({"a", "b", "c"})         -- "abc"
```

## Text Wrapping

```lua
str.wrap("the quick brown fox jumps over the lazy dog", 20)
-- "the quick brown fox\njumps over the lazy\ndog"
```

## Integration with Luafun

```lua
local fun = require 'fun'
local str = require 'str'

-- Split lines, strip whitespace, filter blanks
fun.iter(str.lines(text))
    :map(str.strip)
    :filter(function(s) return not str.isblank(s) end)
    :totable()

-- Parse CSV-ish data
fun.iter(str.lines(csv))
    :map(function(line) return str.split(line, ",") end)
    :filter(function(row) return #row >= 3 end)
    :totable()

-- Word frequency
local words = str.split(str.lower(text))
fun.iter(words):reduce(function(counts, word)
    counts[word] = (counts[word] or 0) + 1
    return counts
end, {})
```

## Differences from Python

| Python | str.lua | Notes |
|--------|---------|-------|
| `s.split()` | `str.split(s)` | Function, not method |
| `s.strip()` | `str.strip(s)` | Function, not method |
| `"sep".join(list)` | `str.join(list, "sep")` | Args reversed |
| `s.find(sub)` | `s:find(sub, 1, true)` | Use Lua native |
| `s[1:3]` | `s:sub(1, 3)` | Use Lua native |

## See Also

- Lua string library: `string.find`, `string.match`, `string.gsub`
- Luafun for iteration: `require 'fun'`
