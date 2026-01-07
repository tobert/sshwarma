# 08: Integration

**File:** `src/embedded/screen.lua`, `src/ssh/handler.rs`, `src/lua/mod.rs`
**Focus:** Wire all components together
**Dependencies:** All Group A and B tasks
**Unblocks:** 09-cleanup

---

## Task

Integrate all the new Lua components into a working system. This is the final assembly before cleanup.

**Why this task?** All pieces exist but need to be connected. This validates everything works together.

**Deliverables:**
1. screen.lua orchestrates all modules
2. SSH handler simplified to byte forwarding
3. Bootstrap sequence works (require, init, screen)
4. All slash commands work end-to-end
5. @mentions stream correctly
6. Regions/overlays display properly
7. Input handling complete

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
# Manual testing: connect via SSH, test all commands
```

## Out of Scope

- Deleting old code — that's 09-cleanup
- New features — just integration

Focus ONLY on making everything work together.

---

## screen.lua Structure

```lua
-- src/embedded/screen.lua
-- Main entry point for SSH UI

-- Bootstrap (require system already initialized)
local regions = require 'ui.regions'
local input = require 'ui.input'
local chat = require 'ui.chat'
local commands = require 'commands'
local overlay = require 'ui.overlay'
local status = require 'ui.status'

-- Initialize regions
regions.define('chat', { top = 0, bottom = -2, fill = true, z = 0 })
regions.define('status', { bottom = -2, height = 1, z = 0 })
regions.define('input', { bottom = 0, height = 1, z = 0 })
regions.define('overlay', { width = "80%", height = "80%", z = 10, visible = false })

-- Renderers for each region
local renderers = {
    chat = chat.render,
    status = status.render,
    input = input.render,
    overlay = overlay.render,
}

--- Main render entry point
function on_tick(dirty_tags, tick, ctx)
    ctx:clear()

    -- Resolve regions
    local resolved = regions.resolve(ctx.w, ctx.h)

    -- Get visible regions in z-order
    local ordered = regions.visible_ordered()

    -- Render each region
    for _, r in ipairs(ordered) do
        local render_fn = renderers[r.name]
        if render_fn and r.area then
            render_fn(ctx, r.area)
        end
    end
end

--- Input entry point
function on_input(bytes)
    local keys = input.parse(bytes)

    for _, key in ipairs(keys) do
        -- Check if overlay wants the key first
        if regions.is_visible('overlay') then
            if key.type == "escape" then
                regions.hide('overlay')
                tools.mark_dirty('chat', 'overlay')
                goto continue
            elseif key.type == "pageup" then
                overlay.scroll_up()
                goto continue
            elseif key.type == "pagedown" then
                overlay.scroll_down()
                goto continue
            end
        end

        -- Normal input handling
        if key.type == "char" then
            input.insert(key.char)
        elseif key.type == "backspace" then
            input.backspace()
        elseif key.type == "delete" then
            input.delete_char()
        elseif key.type == "arrow" then
            if key.dir == "left" then input.left()
            elseif key.dir == "right" then input.right()
            elseif key.dir == "up" then chat.scroll_up()
            elseif key.dir == "down" then chat.scroll_down()
            end
        elseif key.type == "home" then
            input.home()
        elseif key.type == "end" then
            input.end_of_line()
        elseif key.type == "enter" then
            local text = input.submit()
            if #text > 0 then
                handle_submit(text)
            end
        elseif key.type == "ctrl" then
            handle_ctrl(key.char)
        elseif key.type == "pageup" then
            chat.page_up()
        elseif key.type == "pagedown" then
            chat.page_down()
        end

        ::continue::
    end
end

--- Handle submitted input
function handle_submit(text)
    if text:match("^/") then
        -- Slash command
        local result = commands.dispatch(text)
        if result then
            show_result(result)
        end
    elseif text:match("^@") then
        -- @mention
        input.handle_mention(text)
    else
        -- Chat message
        tools.say(text)
        tools.mark_dirty('chat')
    end
end

--- Show command result
function show_result(result)
    if result.region == "overlay" then
        overlay.show(result.title or "Output", result.text)
        regions.show('overlay')
        tools.mark_dirty('overlay')
    elseif result.region == "status" then
        status.set_message(result.text)
        tools.mark_dirty('status')
    elseif result.region == "chat" then
        tools.mark_dirty('chat')
    end
end

--- Handle Ctrl+key
function handle_ctrl(char)
    if char == "c" or char == "u" then
        input.clear()
    elseif char == "a" then
        input.home()
    elseif char == "e" then
        input.end_of_line()
    elseif char == "l" then
        tools.mark_dirty('chat', 'status', 'input')
    elseif char == "d" then
        -- EOF - disconnect?
    end
end

--- Row added callback
function on_row_added(buffer_id, row)
    tools.mark_dirty('chat')
end

--- Background tick
function background(tick)
    -- Refresh status every 2 seconds
    if tick % 4 == 0 then
        tools.mark_dirty('status')
    end
end

-- Register callbacks
tools.on_row_added(on_row_added)
```

---

## SSH Handler Simplification

```rust
// src/ssh/handler.rs - simplified

impl Handler for SshHandler {
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Just forward bytes to Lua
        if let Some(ref lua_runtime) = self.lua_runtime {
            let lua = lua_runtime.lock().await;
            if let Err(e) = lua.call_on_input(data) {
                tracing::error!("Lua input error: {}", e);
                // Could show error to user via overlay
            }
        }
        Ok(())
    }

    // ... other handlers unchanged
}
```

---

## Bootstrap Sequence

```rust
// In src/lua/mod.rs

impl LuaRuntime {
    pub fn new(state: Arc<SharedState>, username: &str, room: Option<&str>) -> Result<Self> {
        let lua = Lua::new();

        // 1. Register core tools (tools.*, sshwarma.*)
        register_tools(&lua, state.clone())?;
        register_layout_functions(&lua)?;
        register_render_functions(&lua)?;
        register_scroll_functions(&lua)?;

        // 2. Set up custom require (01-require)
        setup_custom_require(&lua)?;

        // 3. Set session context
        let session = SessionContext {
            username: username.to_string(),
            room_name: room.map(|s| s.to_string()),
            // ...
        };
        set_session_context(&lua, session)?;

        // 4. Run bootstrap (init.lua)
        lua.load(include_str!("../embedded/init.lua"))
            .set_name("init.lua")
            .exec()?;

        // 5. Load screen.lua
        lua.load(include_str!("../embedded/screen.lua"))
            .set_name("screen.lua")
            .exec()?;

        // 6. Register callbacks
        register_callbacks(&lua)?;

        Ok(Self { lua, state })
    }
}
```

---

## Testing Checklist

### Commands
- [ ] /help → overlay with help text
- [ ] /rooms → overlay with room list
- [ ] /join <room> → joins, shows room info
- [ ] /leave → returns to lobby
- [ ] /look → shows room details
- [ ] /inv → shows inventory overlay
- [ ] /inv all → includes available tools
- [ ] /equip <name> → equips, shows delta
- [ ] /unequip <name> → removes tool
- [ ] /tools → lists MCP tools
- [ ] /run <tool> → executes, shows result
- [ ] Unknown command → error overlay

### Input
- [ ] Typing shows in input line
- [ ] Backspace deletes
- [ ] Arrows move cursor
- [ ] Enter submits and clears
- [ ] Ctrl+C clears line
- [ ] Ctrl+L redraws

### Chat
- [ ] Messages display correctly
- [ ] Word wrapping works
- [ ] Scroll works (page up/down)
- [ ] New messages appear at bottom

### @Mentions
- [ ] @model message starts stream
- [ ] Chunks appear incrementally
- [ ] Streaming indicator visible
- [ ] Tool calls display
- [ ] Complete response shows

### Overlays
- [ ] Overlay covers chat area
- [ ] ESC closes overlay
- [ ] PgUp/PgDn scroll overlay content
- [ ] Multiple overlays? (if supported)

---

## Acceptance Criteria

- [ ] SSH connection works
- [ ] All commands functional
- [ ] Chat displays correctly
- [ ] @mentions stream responses
- [ ] Overlays show/hide properly
- [ ] Input handling complete
- [ ] No Rust panics
- [ ] No Lua errors (or gracefully handled)
- [ ] Screen refresh smooth
- [ ] Terminal resize works
