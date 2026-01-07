# Lua Functional UI — Design Session Brief

**Purpose:** Design a GUI rewrite leveraging luafun for cleaner data transformations.

**Session Type:** Claude & Amy Show — collaborative design and implementation

---

## Context

We've added functional programming primitives to our Lua toolkit:

| Library | Size | Purpose |
|---------|------|---------|
| `fun` (luafun) | 1075 lines, ~7.5k tokens | Lazy iterators, map/filter/reduce |
| `str` | 250 lines | Python-style string utilities |
| `inspect` | 270 lines | Debug pretty-printing |

These compose nicely for data transformation pipelines.

## Current Pain Points

The existing UI code in `screen.lua` and command modules uses imperative patterns:

```lua
-- Typical current pattern
local result = {}
for _, row in ipairs(rows) do
    if row.kind == "message" then
        local text = format_message(row)
        if #text > 0 then
            table.insert(result, text)
        end
    end
end
return table.concat(result, "\n")
```

Issues:
- Nested loops obscure intent
- Intermediate tables allocate memory
- Filter/transform logic interleaved
- Hard to test individual transformations

## Functional Alternative

```lua
local fun = require 'fun'
local str = require 'str'

return fun.iter(rows)
    :filter(function(r) return r.kind == "message" end)
    :map(format_message)
    :filter(function(t) return #t > 0 end)
    :totable()
    |> str.join("\n")
```

Wins:
- **Declarative** — reads top-to-bottom
- **Lazy** — no intermediate tables until `totable()`
- **Composable** — each step is testable in isolation
- **Chainable** — method syntax for fluent APIs

## Design Questions

### 1. Where to Apply Functional Patterns?

**High-value targets:**
- `render_chat()` — row filtering, formatting, scrolling
- `render_status()` — participant list building
- `render_prompt()` — completion filtering
- Command output formatting — `/history`, `/journal`, `/inventory`
- Context composition (`wrap.lua`) — history truncation, token budgeting

**Lower value:**
- Simple loops with 2-3 lines
- Single-pass mutations
- Performance-critical inner loops (measure first)

### 2. Abstraction Level

Should we create higher-level helpers that use luafun internally?

```lua
-- Option A: Raw luafun everywhere
fun.iter(rows):filter(is_message):map(format):totable()

-- Option B: Domain-specific helpers
rows:messages():formatted():as_lines()

-- Option C: Hybrid — luafun for data, helpers for rendering
local messages = fun.iter(rows):filter(is_message):totable()
render_block(messages, format_message)
```

### 3. Iterator vs Table Boundaries

Where should we materialize iterators?

```lua
-- Keep lazy as long as possible
local pipeline = fun.iter(rows)
    :filter(pred)
    :map(transform)
    :take(limit)  -- still lazy

-- Materialize at API boundaries
return pipeline:totable()  -- materialize for external use
```

### 4. Error Handling in Pipelines

Luafun iterators don't handle errors mid-stream. Options:

```lua
-- Option A: Filter out errors
:map(function(r)
    local ok, result = pcall(transform, r)
    return ok and result or nil
end)
:filter(function(x) return x ~= nil end)

-- Option B: Collect errors separately
local results, errors = {}, {}
fun.iter(rows):each(function(r)
    local ok, result = pcall(transform, r)
    if ok then results[#results+1] = result
    else errors[#errors+1] = {row=r, err=result} end
end)

-- Option C: Let it crash (simpler, surfaces bugs faster)
:map(transform)  -- throws on bad data
```

## Implementation Sketch

### Phase 1: Low-Hanging Fruit
- Replace simple `for` loops in command output formatting
- Add `fun` and `str` requires to existing modules
- No structural changes, just cleaner transforms

### Phase 2: Chat Rendering
- Refactor `render_chat()` to use lazy pipelines
- Extract row filtering predicates as named functions
- Add scroll position as pipeline parameter

### Phase 3: Context Composition
- Rewrite `wrap.lua` history building with luafun
- Token budget as `:take_while()` over cumulative sum
- Role-based filtering with named predicates

### Phase 4: Component Architecture
- Define render components as pure functions: `(state) -> segments`
- Compose screen from pipelines of components
- Hot-reload individual components

## Example Refactors

### Before: render_participants
```lua
local function render_participants(participants)
    local parts = {}
    for _, p in ipairs(participants) do
        local icon = p.kind == "model" and "◈" or "●"
        local status = p.status == "idle" and "" or (" " .. p.status)
        table.insert(parts, icon .. " " .. p.name .. status)
    end
    return table.concat(parts, "  ")
end
```

### After: render_participants
```lua
local function render_participants(participants)
    return fun.iter(participants)
        :map(function(p)
            local icon = p.kind == "model" and "◈" or "●"
            local status = p.status == "idle" and "" or (" " .. p.status)
            return icon .. " " .. p.name .. status
        end)
        :totable()
        |> function(t) return str.join(t, "  ") end
end
```

### Before: filter_history
```lua
local function filter_history(rows, limit, kinds)
    local result = {}
    local kinds_set = {}
    for _, k in ipairs(kinds or {}) do kinds_set[k] = true end

    for i = #rows, 1, -1 do
        local row = rows[i]
        if #kinds == 0 or kinds_set[row.kind] then
            table.insert(result, 1, row)
            if #result >= limit then break end
        end
    end
    return result
end
```

### After: filter_history
```lua
local function filter_history(rows, limit, kinds)
    local kinds_set = fun.iter(kinds or {}):tomap(function(k) return k, true end)
    local accept = #kinds == 0
        and function() return true end
        or function(r) return kinds_set[r.kind] end

    -- Reverse, filter, take, reverse back
    return fun.iter(rows)
        :enumerate()
        :map(function(i, r) return #rows - i + 1, r end)  -- reverse index
        :filter(function(_, r) return accept(r) end)
        :take(limit)
        :map(function(_, r) return r end)
        :totable()
end
```

(Note: This shows the complexity of reverse iteration with luafun — might keep imperative for this case)

## Testing Strategy

Functional pipelines are easier to test:

```lua
-- Test individual predicates
assert(is_message({kind = "message"}) == true)
assert(is_message({kind = "system"}) == false)

-- Test transformations
local input = {kind = "message", content = "hello"}
assert(format_message(input) == "hello")

-- Test composition
local pipeline = fun.iter(test_rows)
    :filter(is_message)
    :map(format_message)
    :totable()
assert(#pipeline == expected_count)
```

## Open Questions for Session

1. **Luau pipe operator** — Does luau have `|>`? If not, helper function?
2. **Performance** — Any hot paths where luafun overhead matters?
3. **Debugging** — How to inspect mid-pipeline values?
4. **Generators** — Use `fun.range()` for scroll calculations?
5. **Partial application** — Create curried helpers for common patterns?

## Resources

- Help docs: `src/embedded/help/fun.md`, `str.md`, `inspect.md`
- Luafun source: `src/embedded/lib/fun.lua` (~7.5k tokens, readable)
- Current screen: `src/embedded/screen.lua`
- Current wrap: `src/embedded/wrap.lua`

---

*Prepared for Claude & Amy design session. Ready when you are!* 🌯
