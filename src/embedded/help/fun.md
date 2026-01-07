# Lua Fun — Quick Reference

High-performance functional programming for Lua. Lazy iterators, chainable operations.

**Source:** `require 'fun'` → [luafun/luafun](https://github.com/luafun/luafun)

## Core Concept: Iterators

Everything returns lazy iterators. Chain operations, then materialize:

```lua
local fun = require 'fun'

-- Chainable (method syntax)
fun.iter({1, 2, 3, 4, 5})
    :filter(function(x) return x > 2 end)
    :map(function(x) return x * 2 end)
    :totable()  -- {6, 8, 10}

-- Functional (traditional syntax)
fun.totable(fun.map(function(x) return x * 2 end,
    fun.filter(function(x) return x > 2 end, {1, 2, 3, 4, 5})))
```

## Generators

```lua
fun.range(5)              -- 1, 2, 3, 4, 5
fun.range(2, 5)           -- 2, 3, 4, 5
fun.range(1, 10, 2)       -- 1, 3, 5, 7, 9

fun.duplicate("x")        -- "x", "x", "x", ... (infinite)
fun.tabulate(math.sin)    -- sin(0), sin(1), sin(2), ...
fun.zeros()               -- 0, 0, 0, ... (infinite)
fun.ones()                -- 1, 1, 1, ... (infinite)
fun.rands(1, 100)         -- random integers 1-100 (infinite)
```

## Slicing

```lua
:take(n)                  -- first n elements
:take_while(pred)         -- while predicate true
:drop(n)                  -- skip first n
:drop_while(pred)         -- skip while predicate true
:nth(n)                   -- get nth element (1-indexed)
:head()                   -- first element (errors if empty)
:tail()                   -- all but first
```

## Filtering

```lua
:filter(pred)             -- keep elements where pred(x) is true
:grep(pattern)            -- keep strings matching pattern
:partition(pred)          -- returns (matches, non-matches)
```

## Transformations

```lua
:map(fn)                  -- apply fn to each element
:enumerate()              -- (1, a), (2, b), (3, c), ...
:intersperse(x)           -- a, x, b, x, c, ...
:zip(other)               -- pairs from two iterators
:chain(other)             -- concatenate iterators
:cycle()                  -- repeat iterator forever
```

## Reductions

```lua
:reduce(fn, init)         -- fold left: fn(fn(init, a), b), ...
:foldl(fn, init)          -- alias for reduce
:length()                 -- count elements
:sum()                    -- sum numbers
:product()                -- multiply numbers
:min()                    -- minimum value
:max()                    -- maximum value
:all(pred)                -- true if all match
:any(pred)                -- true if any match
:is_null()                -- true if empty
```

## Materialization

```lua
:totable()                -- collect into array
:tomap()                  -- collect k,v pairs into table
:each(fn)                 -- call fn on each (for side effects)
```

## Built-in Operators

```lua
fun.op.add(a, b)          -- a + b
fun.op.sub(a, b)          -- a - b
fun.op.mul(a, b)          -- a * b
fun.op.div(a, b)          -- a / b
fun.op.mod(a, b)          -- a % b
fun.op.pow(a, b)          -- a ^ b
fun.op.eq(a, b)           -- a == b
fun.op.ne(a, b)           -- a ~= b
fun.op.lt(a, b)           -- a < b
fun.op.le(a, b)           -- a <= b
fun.op.gt(a, b)           -- a > b
fun.op.ge(a, b)           -- a >= b
fun.op.concat(a, b)       -- a .. b
fun.op.len(a)             -- #a
fun.op.unm(a)             -- -a
fun.op.land(a, b)         -- a and b
fun.op.lor(a, b)          -- a or b
fun.op.lnot(a)            -- not a
```

## Common Patterns

### Sum of squares
```lua
fun.range(10):map(function(x) return x*x end):sum()
```

### Filter and count
```lua
fun.iter(items):filter(is_valid):length()
```

### Find first match
```lua
fun.iter(items):filter(pred):nth(1)
```

### Group by key (manual)
```lua
local groups = {}
fun.iter(items):each(function(item)
    local key = item.kind
    groups[key] = groups[key] or {}
    table.insert(groups[key], item)
end)
```

### Zip two arrays
```lua
fun.zip(names, scores):each(function(name, score)
    print(name, score)
end)
```

### Take while with index
```lua
fun.iter(items):enumerate():take_while(function(i, _) return i <= 10 end)
```

### Flatten (one level)
```lua
fun.iter(nested):chain():totable()  -- only works for iterator of iterators
-- For array of arrays, use reduce:
fun.iter(arrays):reduce(function(acc, arr)
    for _, v in ipairs(arr) do table.insert(acc, v) end
    return acc
end, {})
```

## Gotchas

1. **Iterators are consumed** — once iterated, you can't reuse them
2. **Infinite iterators** — always use `:take(n)` before materializing
3. **Side effects** — use `:each()` not `:map()` for side effects
4. **Multiple returns** — some iterators yield multiple values (k, v)

## Source Reference

For deep dives, extract sections from `sshwarma.get_embedded_module("fun")`:

| Section | Lines | Description |
|---------|-------|-------------|
| Tools | 15-72 | Internal utilities, iterator protocol |
| Basic Functions | 73-183 | iter, each, wrap/unwrap |
| Generators | 184-293 | range, duplicate, tabulate, zeros, ones, rands |
| Slicing | 294-473 | nth, head, tail, take, drop, take_while, drop_while |
| Indexing | 474-522 | index, indexes, index_of |
| Filtering | 523-591 | filter, grep, partition |
| Reducing | 592-819 | foldl, length, sum, min, max, totable, tomap |
| Transformations | 820-877 | map, enumerate, intersperse |
| Compositions | 878-998 | zip, cycle, chain |
| Operators | 999-1051 | comparison, arithmetic, logical (fun.op.*) |

Example:
```lua
local str = require 'str'
local src = sshwarma.get_embedded_module("fun")
local operators = str.extract_lines(src, 999, 1051)
```

## See Also

- Full source: `require 'fun'` (~1075 lines, well-commented)
- Original docs: https://luafun.github.io/
