# 01: Custom Module Loader

**File:** `src/lua/mod.rs`, `src/embedded/init.lua`
**Focus:** Lua require system with embedded, user, and system modules
**Dependencies:** None
**Unblocks:** 05-commands (needs `require 'commands'`)

---

## Task

Implement a custom Lua module loader that supports:
1. Embedded modules (compiled into binary)
2. User modules (`~/.config/sshwarma/lua/`)
3. Standard package.path (penlight, etc.)

**Why this first?** Commands and UI modules need `require()` to work. This unblocks all Lua module organization.

**Deliverables:**
1. Custom searcher added to `package.searchers`
2. Embedded module registry in Rust
3. `require 'inspect'` works (vendor inspect.lua)
4. `require 'commands.nav'` loads `src/embedded/commands/nav.lua`
5. User can override with `~/.config/sshwarma/lua/commands/nav.lua`

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- Database-stored scripts — that's future work
- Hot-reloading individual modules — that's integration work
- Actual command implementations — that's 05-commands

Focus ONLY on making `require()` work with our module hierarchy.

---

## mlua Patterns

```rust
use mlua::{Lua, Function, Table, Value, Result as LuaResult};

// Add custom searcher to package.searchers
fn setup_custom_require(lua: &Lua) -> LuaResult<()> {
    let package: Table = lua.globals().get("package")?;
    let searchers: Table = package.get("searchers")?;

    // Insert at position 2 (after preload, before path searchers)
    let custom_searcher = lua.create_function(|lua, modname: String| {
        // Return loader function or nil
        Ok(Value::Nil)
    })?;

    // Lua tables are 1-indexed, insert at 2 shifts others down
    searchers.raw_insert(2, custom_searcher)?;
    Ok(())
}

// Load embedded module content
fn get_embedded_module(name: &str) -> Option<&'static str> {
    match name {
        "inspect" => Some(include_str!("../embedded/lib/inspect.lua")),
        "commands" => Some(include_str!("../embedded/commands/init.lua")),
        "commands.nav" => Some(include_str!("../embedded/commands/nav.lua")),
        // ...
        _ => None,
    }
}
```

---

## Types

```rust
/// Registry of embedded Lua modules
pub struct EmbeddedModules {
    modules: HashMap<String, &'static str>,
}

impl EmbeddedModules {
    pub fn new() -> Self {
        let mut modules = HashMap::new();
        // Populate from include_str! macros
        modules.insert("inspect".to_string(), include_str!("../embedded/lib/inspect.lua"));
        // ...
        Self { modules }
    }

    pub fn get(&self, name: &str) -> Option<&'static str> {
        self.modules.get(name).copied()
    }

    /// List all embedded module names (for debugging)
    pub fn list(&self) -> Vec<&str> {
        self.modules.keys().map(|s| s.as_str()).collect()
    }
}
```

---

## Lua Bootstrap (init.lua)

```lua
-- src/embedded/init.lua
-- Bootstrap script run before user code

-- Custom searcher for embedded and user modules
local function sshwarma_searcher(modname)
    -- Try embedded first
    local embedded = sshwarma.get_embedded_module(modname)
    if embedded then
        local loader, err = load(embedded, "@embedded/" .. modname .. ".lua")
        if loader then return loader end
        return "\n\tcannot load embedded module '" .. modname .. "': " .. (err or "unknown error")
    end

    -- Try user config directory
    local path = modname:gsub("%.", "/")
    local user_path = sshwarma.config_path .. "/lua/" .. path .. ".lua"
    local f = io.open(user_path, "r")
    if f then
        local content = f:read("*a")
        f:close()
        local loader, err = load(content, "@" .. user_path)
        if loader then return loader end
        return "\n\tcannot load user module '" .. modname .. "': " .. (err or "unknown error")
    end

    -- Fall through to standard searchers
    return nil
end

-- Insert after preload (position 2)
table.insert(package.searchers, 2, sshwarma_searcher)

-- Preload commonly used modules
package.preload['inspect'] = function()
    return load(sshwarma.get_embedded_module('inspect'), "@embedded/inspect.lua")()
end
```

---

## Methods to Implement

**In `src/lua/mod.rs`:**
- `register_embedded_modules(lua: &Lua) -> Result<()>` — Set up `sshwarma.get_embedded_module`
- `setup_require_path(lua: &Lua) -> Result<()>` — Add user config to package.path
- `run_bootstrap(lua: &Lua) -> Result<()>` — Execute init.lua

**In `LuaRuntime`:**
- Modify `new()` to call bootstrap before loading screen.lua

---

## Module Path Resolution

```
require("foo")        → embedded/foo.lua OR ~/.config/sshwarma/lua/foo.lua
require("foo.bar")    → embedded/foo/bar.lua OR ~/.config/sshwarma/lua/foo/bar.lua
require("commands")   → embedded/commands/init.lua
require("pl.tablex")  → standard package.path (e.g., /usr/share/lua/5.4/pl/tablex.lua)
```

---

## Acceptance Criteria

- [ ] `require 'inspect'` returns inspect table
- [ ] `require 'commands'` loads commands/init.lua
- [ ] `require 'commands.nav'` loads commands/nav.lua
- [ ] User file `~/.config/sshwarma/lua/test.lua` loadable via `require 'test'`
- [ ] User file shadows embedded (user's commands/nav.lua overrides embedded)
- [ ] Standard libs work: `require 'pl.path'` if penlight installed
- [ ] Missing module gives clear error message
- [ ] `sshwarma.list_embedded_modules()` returns module list (for debugging)
