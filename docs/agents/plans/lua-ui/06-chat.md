# 06: Chat Rendering

**File:** `src/embedded/ui/chat.lua`
**Focus:** Render chat buffer from rows
**Dependencies:** 03-regions, 04-tools-api
**Unblocks:** 08-integration

---

## Task

Implement chat rendering in Lua using the row buffer system. Chat displays messages, tool calls, and streaming model responses.

**Why this task?** Chat is the main UI element. Must render from rows for consistency with model tool calls.

**Deliverables:**
1. Render messages from `tools.history()` rows
2. Handle different content methods (message.user, message.model, tool.call, etc.)
3. Word wrapping
4. Scroll support (page up/down, tail mode)
5. Streaming indicator for in-progress responses

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- Input line rendering — that's 02-input
- Status bar — simple, inline in screen.lua
- Message sending — Rust handles, we just render

Focus ONLY on rendering the chat region from row data.

---

## Row Content Methods

Messages come as rows with different `content_method` values:

| content_method | Meaning |
|----------------|---------|
| `message.user` | User chat message |
| `message.model` | Complete model response |
| `message.model.chunk` | Streaming chunk (partial) |
| `message.system` | System notification |
| `tool.call` | Tool invocation |
| `tool.result` | Tool result |

---

## Chat Module

```lua
-- ui/chat.lua

local M = {}

local colors = {
    fg       = "#c0caf5",
    dim      = "#565f89",
    nick     = "#7aa2f7",
    model    = "#bb9af7",
    self     = "#9ece6a",
    system   = "#7dcfff",
    error    = "#f7768e",
    tool     = "#ff9e64",
}

--- Get display width of a string
---@param str string
---@return number
function M.display_width(str)
    if not str or str == "" then return 0 end
    if tools and tools.display_width then
        return tools.display_width(str)
    end
    return utf8.len(str) or #str
end

--- Wrap text to fit within width
---@param text string
---@param width number
---@return string[]
function M.wrap_text(text, width)
    if width <= 0 then return {""} end
    if not text or text == "" then return {""} end

    local lines = {}

    for segment in (text .. "\n"):gmatch("([^\n]*)\n") do
        if #segment == 0 then
            table.insert(lines, "")
        elseif M.display_width(segment) <= width then
            table.insert(lines, segment)
        else
            -- Word wrap
            local pos = 1
            while pos <= #segment do
                local end_pos = pos
                local current_width = 0

                while end_pos <= #segment do
                    local next_pos = utf8.offset(segment, 2, end_pos) or (#segment + 1)
                    local char = segment:sub(end_pos, next_pos - 1)
                    local char_width = M.display_width(char)

                    if current_width + char_width > width then break end

                    current_width = current_width + char_width
                    end_pos = next_pos
                end

                if end_pos == pos then
                    end_pos = utf8.offset(segment, 2, pos) or (#segment + 1)
                end

                local chunk = segment:sub(pos, end_pos - 1)

                -- Try word boundary
                if end_pos <= #segment then
                    local last_space = chunk:match(".*()%s")
                    if last_space and last_space > #chunk / 2 then
                        chunk = chunk:sub(1, last_space - 1)
                        end_pos = pos + last_space
                        while end_pos <= #segment and segment:sub(end_pos, end_pos):match("%s") do
                            end_pos = end_pos + 1
                        end
                    end
                end

                table.insert(lines, (chunk:gsub("%s+$", "")))
                pos = end_pos
            end
        end
    end

    return #lines > 0 and lines or {""}
end

--- Build display lines from rows
---@param rows table[] Array of row data
---@param width number Terminal width
---@param my_name string Current user's name
---@return table[] Display lines with formatting
function M.build_display_lines(rows, width, my_name)
    local display_lines = {}
    local prefix_width = 0

    -- Calculate max nick width
    for _, row in ipairs(rows) do
        local author = row.author or "???"
        prefix_width = math.max(prefix_width, M.display_width(author) + 3)
    end

    local content_width = width - prefix_width
    if content_width < 10 then
        prefix_width = 0
        content_width = width
    end

    -- Track streaming state
    local streaming_id = nil

    for _, row in ipairs(rows) do
        local method = row.content_method or "message.user"
        local author = row.author or "???"
        local content = row.content or ""

        -- Determine formatting based on content method
        local nick_color = colors.nick
        local is_model = false
        local is_streaming = false
        local is_tool = false

        if method:match("^message%.model") then
            nick_color = colors.model
            is_model = true
            if method == "message.model.chunk" then
                is_streaming = true
                streaming_id = row.id
            end
        elseif method == "message.user" and author == my_name then
            nick_color = colors.self
        elseif method == "message.system" then
            nick_color = colors.system
        elseif method:match("^tool%.") then
            nick_color = colors.tool
            is_tool = true
            -- Format tool calls specially
            if method == "tool.call" then
                author = "⚙ " .. (row.tool_name or "tool")
            elseif method == "tool.result" then
                author = "← " .. (row.tool_name or "result")
            end
        end

        -- Wrap content
        local wrapped = M.wrap_text(content, content_width)

        for i, line_text in ipairs(wrapped) do
            local entry = {
                text = line_text,
                nick_color = nick_color,
                is_first_line = (i == 1),
                is_last_line = (i == #wrapped),
                is_streaming = is_streaming,
                is_tool = is_tool,
                prefix_width = prefix_width,
                author = (i == 1) and author or nil,
                row_id = row.id,
            }
            table.insert(display_lines, entry)
        end
    end

    return display_lines
end

--- Render chat buffer
---@param ctx table Draw context
---@param area table Region bounds
---@param scroll table Scroll state
function M.render(ctx, area, scroll)
    local rows = tools.history(200)  -- Get recent history
    local session = tools.session()
    local my_name = session and session.username or ""

    local display_lines = M.build_display_lines(rows, area.w, my_name)
    local total_lines = #display_lines

    -- Update scroll state
    scroll:set_content_height(total_lines)
    scroll:set_viewport_height(area.h)

    -- Get visible range
    local start_line, end_line = scroll:visible_range()
    if end_line > total_lines then end_line = total_lines end
    if start_line < 0 then start_line = 0 end

    -- Draw visible lines
    for i = start_line, end_line - 1 do
        local line_idx = i + 1
        local line = display_lines[line_idx]

        if line then
            local y = i - start_line
            local x = 0

            if line.is_first_line and line.author then
                -- Draw prefix
                ctx:print(x, y, "<", {fg = colors.dim})
                x = x + 1
                ctx:print(x, y, line.author, {fg = line.nick_color})
                x = x + M.display_width(line.author)
                ctx:print(x, y, "> ", {fg = colors.dim})
                x = x + 2
            else
                x = line.prefix_width or 0
            end

            -- Draw content
            local text = line.text
            if line.is_streaming and line.is_last_line then
                text = text .. " ◌"
            end
            ctx:print(x, y, text)
        end
    end

    -- Scroll indicator
    if not scroll.at_bottom and total_lines > area.h then
        local pct = math.floor(scroll.percent * 100)
        local indicator = string.format("── %d%% ──", pct)
        local ix = area.w - M.display_width(indicator)
        ctx:print(ix, area.h - 1, indicator, {fg = colors.dim})
    end
end

return M
```

---

## Tool Call Rendering

Tool calls and results should be visually distinct:

```lua
-- For tool.call:
-- ⚙ sample> {prompt: "jazz"}

-- For tool.result:
-- ← sample> Generated: abc123.wav
```

The content can be collapsed/expanded (future enhancement).

---

## Scroll Integration

Use existing scroll state from `tools.scroll()`:

```lua
local scroll = tools.scroll()

-- In on_tick
local chat = require 'ui.chat'
local regions = require 'ui.regions'

local chat_area = regions.get('chat')
if chat_area then
    chat.render(ctx, chat_area, scroll)
end
```

---

## Acceptance Criteria

- [ ] User messages display with colored nick
- [ ] Model messages display with model color
- [ ] System messages display with system color
- [ ] Tool calls display with tool indicator
- [ ] Tool results display with result indicator
- [ ] Word wrapping works correctly
- [ ] UTF-8/emoji render correctly
- [ ] Scroll state tracks position
- [ ] Page up/down work
- [ ] Tail mode follows new messages
- [ ] Scroll indicator shows when not at bottom
- [ ] Streaming indicator (◌) shows during response
- [ ] Long messages wrap at word boundaries
