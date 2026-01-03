//! Scroll and view stack management for Lua UI
//!
//! Provides scroll state tracking and view layer management
//! for the terminal UI.

use mlua::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Scroll mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollMode {
    /// Follow new content automatically
    #[default]
    Tail,
    /// Stay at fixed position
    Pinned,
}

impl ScrollMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScrollMode::Tail => "tail",
            ScrollMode::Pinned => "pinned",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tail" => Some(ScrollMode::Tail),
            "pinned" => Some(ScrollMode::Pinned),
            _ => None,
        }
    }
}

/// Scroll state for a single scrollable region
#[derive(Debug, Clone)]
pub struct ScrollState {
    /// Scroll offset (lines from top of content)
    pub offset: i32,
    /// Total content height in lines
    pub content_height: i32,
    /// Viewport height in lines
    pub viewport_height: i32,
    /// Scroll mode
    pub mode: ScrollMode,
    /// Row ID at top of viewport (for buffer scrolling)
    pub top_row_id: Option<String>,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollState {
    pub fn new() -> Self {
        Self {
            offset: 0,
            content_height: 0,
            viewport_height: 0,
            mode: ScrollMode::Tail,
            top_row_id: None,
        }
    }

    /// Maximum valid scroll offset
    pub fn max_offset(&self) -> i32 {
        (self.content_height - self.viewport_height).max(0)
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: i32) {
        self.offset = (self.offset - n).max(0);
        if self.offset < self.max_offset() {
            self.mode = ScrollMode::Pinned;
        }
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: i32) {
        self.offset = (self.offset + n).min(self.max_offset());
        if self.offset >= self.max_offset() {
            self.mode = ScrollMode::Tail;
        }
    }

    /// Scroll to top
    pub fn scroll_to_top(&mut self) {
        self.offset = 0;
        self.mode = ScrollMode::Pinned;
    }

    /// Scroll to bottom (tail mode)
    pub fn scroll_to_bottom(&mut self) {
        self.offset = self.max_offset();
        self.mode = ScrollMode::Tail;
    }

    /// Page up
    pub fn page_up(&mut self) {
        self.scroll_up(self.viewport_height.saturating_sub(2).max(1));
    }

    /// Page down
    pub fn page_down(&mut self) {
        self.scroll_down(self.viewport_height.saturating_sub(2).max(1));
    }

    /// Half page up
    pub fn half_page_up(&mut self) {
        self.scroll_up((self.viewport_height / 2).max(1));
    }

    /// Half page down
    pub fn half_page_down(&mut self) {
        self.scroll_down((self.viewport_height / 2).max(1));
    }

    /// Update content height (for tail mode, adjust offset)
    pub fn set_content_height(&mut self, height: i32) {
        self.content_height = height;
        if self.mode == ScrollMode::Tail {
            self.offset = self.max_offset();
        } else {
            // Clamp offset to valid range
            self.offset = self.offset.min(self.max_offset());
        }
    }

    /// Update viewport height
    pub fn set_viewport_height(&mut self, height: i32) {
        self.viewport_height = height;
        if self.mode == ScrollMode::Tail {
            self.offset = self.max_offset();
        }
    }

    /// Check if at bottom
    pub fn at_bottom(&self) -> bool {
        self.offset >= self.max_offset()
    }

    /// Check if at top
    pub fn at_top(&self) -> bool {
        self.offset == 0
    }

    /// Get visible range (start_line, end_line)
    pub fn visible_range(&self) -> (i32, i32) {
        let start = self.offset;
        let end = (start + self.viewport_height).min(self.content_height);
        (start, end)
    }

    /// Scroll percentage (0.0 to 1.0)
    pub fn scroll_percent(&self) -> f64 {
        if self.max_offset() == 0 {
            1.0
        } else {
            self.offset as f64 / self.max_offset() as f64
        }
    }
}

/// A layer in the view stack
#[derive(Debug, Clone)]
pub struct ViewLayer {
    /// Layer type (e.g., "buffer", "modal", "popup")
    pub kind: String,
    /// Layer data (e.g., buffer_id, modal content)
    pub data: HashMap<String, String>,
    /// Whether this layer is focusable
    pub focusable: bool,
}

impl ViewLayer {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            data: HashMap::new(),
            focusable: true,
        }
    }

    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }
}

/// View stack for managing UI layers
#[derive(Debug, Clone, Default)]
pub struct ViewStack {
    layers: Vec<ViewLayer>,
}

impl ViewStack {
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Push a layer onto the stack
    pub fn push(&mut self, layer: ViewLayer) {
        self.layers.push(layer);
    }

    /// Pop the top layer
    pub fn pop(&mut self) -> Option<ViewLayer> {
        self.layers.pop()
    }

    /// Get the top layer
    pub fn top(&self) -> Option<&ViewLayer> {
        self.layers.last()
    }

    /// Get the top layer mutably
    pub fn top_mut(&mut self) -> Option<&mut ViewLayer> {
        self.layers.last_mut()
    }

    /// Get all layers
    pub fn layers(&self) -> &[ViewLayer] {
        &self.layers
    }

    /// Number of layers
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// Check if stack is empty
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Clear all layers
    pub fn clear(&mut self) {
        self.layers.clear();
    }

    /// Find the topmost focusable layer index
    pub fn focused_index(&self) -> Option<usize> {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, layer)| layer.focusable)
            .map(|(i, _)| i)
    }
}

/// Lua userdata for scroll state
#[derive(Clone)]
pub struct LuaScrollState {
    state: Arc<Mutex<ScrollState>>,
}

impl LuaScrollState {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ScrollState::new())),
        }
    }

    pub fn inner(&self) -> Arc<Mutex<ScrollState>> {
        self.state.clone()
    }
}

impl Default for LuaScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaUserData for LuaScrollState {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Properties
        methods.add_meta_method(mlua::MetaMethod::Index, |lua, this, key: String| {
            let state = this.state.lock().unwrap();
            match key.as_str() {
                "offset" => Ok(LuaValue::Integer(state.offset)),
                "content_height" => Ok(LuaValue::Integer(state.content_height)),
                "viewport_height" => Ok(LuaValue::Integer(state.viewport_height)),
                "mode" => Ok(LuaValue::String(lua.create_string(state.mode.as_str())?)),
                "at_top" => Ok(LuaValue::Boolean(state.at_top())),
                "at_bottom" => Ok(LuaValue::Boolean(state.at_bottom())),
                "max_offset" => Ok(LuaValue::Integer(state.max_offset())),
                "percent" => Ok(LuaValue::Number(state.scroll_percent())),
                _ => Ok(LuaValue::Nil),
            }
        });

        // scroll:up(n?)
        methods.add_method("up", |_lua, this, n: Option<i32>| {
            let mut state = this.state.lock().unwrap();
            state.scroll_up(n.unwrap_or(1));
            Ok(())
        });

        // scroll:down(n?)
        methods.add_method("down", |_lua, this, n: Option<i32>| {
            let mut state = this.state.lock().unwrap();
            state.scroll_down(n.unwrap_or(1));
            Ok(())
        });

        // scroll:page_up()
        methods.add_method("page_up", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.page_up();
            Ok(())
        });

        // scroll:page_down()
        methods.add_method("page_down", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.page_down();
            Ok(())
        });

        // scroll:half_page_up()
        methods.add_method("half_page_up", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.half_page_up();
            Ok(())
        });

        // scroll:half_page_down()
        methods.add_method("half_page_down", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.half_page_down();
            Ok(())
        });

        // scroll:to_top()
        methods.add_method("to_top", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.scroll_to_top();
            Ok(())
        });

        // scroll:to_bottom()
        methods.add_method("to_bottom", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.scroll_to_bottom();
            Ok(())
        });

        // scroll:set_content_height(h)
        methods.add_method("set_content_height", |_lua, this, h: i32| {
            let mut state = this.state.lock().unwrap();
            state.set_content_height(h);
            Ok(())
        });

        // scroll:set_viewport_height(h)
        methods.add_method("set_viewport_height", |_lua, this, h: i32| {
            let mut state = this.state.lock().unwrap();
            state.set_viewport_height(h);
            Ok(())
        });

        // scroll:visible_range() -> start, end
        methods.add_method("visible_range", |_lua, this, ()| {
            let state = this.state.lock().unwrap();
            let (start, end) = state.visible_range();
            Ok((start, end))
        });

        // scroll:tail()
        methods.add_method("tail", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.scroll_to_bottom();
            Ok(())
        });

        // scroll:pin()
        methods.add_method("pin", |_lua, this, ()| {
            let mut state = this.state.lock().unwrap();
            state.mode = ScrollMode::Pinned;
            Ok(())
        });
    }
}

/// Lua userdata for view stack
#[derive(Clone)]
pub struct LuaViewStack {
    stack: Arc<Mutex<ViewStack>>,
}

impl LuaViewStack {
    pub fn new() -> Self {
        Self {
            stack: Arc::new(Mutex::new(ViewStack::new())),
        }
    }

    pub fn inner(&self) -> Arc<Mutex<ViewStack>> {
        self.stack.clone()
    }
}

impl Default for LuaViewStack {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaUserData for LuaViewStack {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Properties
        methods.add_meta_method(mlua::MetaMethod::Index, |_lua, this, key: String| {
            let stack = this.stack.lock().unwrap();
            match key.as_str() {
                "len" => Ok(LuaValue::Integer(stack.len() as i32)),
                "empty" => Ok(LuaValue::Boolean(stack.is_empty())),
                _ => Ok(LuaValue::Nil),
            }
        });

        // stack:push(kind, data?)
        methods.add_method(
            "push",
            |_lua, this, (kind, data): (String, Option<LuaTable>)| {
                let mut layer = ViewLayer::new(kind);

                if let Some(tbl) = data {
                    for pair in tbl.pairs::<String, String>().flatten() {
                        layer.data.insert(pair.0, pair.1);
                    }
                    if let Ok(focusable) = tbl.get::<bool>("focusable") {
                        layer.focusable = focusable;
                    }
                }

                let mut stack = this.stack.lock().unwrap();
                stack.push(layer);
                Ok(())
            },
        );

        // stack:pop() -> kind, data or nil
        methods.add_method("pop", |lua, this, ()| {
            let mut stack = this.stack.lock().unwrap();
            if let Some(layer) = stack.pop() {
                let data = lua.create_table()?;
                for (k, v) in &layer.data {
                    data.set(k.as_str(), v.as_str())?;
                }
                Ok((Some(layer.kind), Some(data)))
            } else {
                Ok((None, None))
            }
        });

        // stack:top() -> kind, data or nil
        methods.add_method("top", |lua, this, ()| {
            let stack = this.stack.lock().unwrap();
            if let Some(layer) = stack.top() {
                let data = lua.create_table()?;
                for (k, v) in &layer.data {
                    data.set(k.as_str(), v.as_str())?;
                }
                Ok((Some(layer.kind.clone()), Some(data)))
            } else {
                Ok((None, None))
            }
        });

        // stack:clear()
        methods.add_method("clear", |_lua, this, ()| {
            let mut stack = this.stack.lock().unwrap();
            stack.clear();
            Ok(())
        });

        // stack:layers() -> array of {kind, data}
        methods.add_method("layers", |lua, this, ()| {
            let stack = this.stack.lock().unwrap();
            let result = lua.create_table()?;

            for (i, layer) in stack.layers().iter().enumerate() {
                let layer_tbl = lua.create_table()?;
                layer_tbl.set("kind", layer.kind.as_str())?;

                let data = lua.create_table()?;
                for (k, v) in &layer.data {
                    data.set(k.as_str(), v.as_str())?;
                }
                layer_tbl.set("data", data)?;
                layer_tbl.set("focusable", layer.focusable)?;

                result.set(i + 1, layer_tbl)?;
            }

            Ok(result)
        });
    }
}

/// Register scroll functions in Lua
pub fn register_scroll_functions(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    let sshwarma: LuaTable = globals.get("sshwarma")?;

    // sshwarma.scroll_state() -> LuaScrollState
    sshwarma.set(
        "scroll_state",
        lua.create_function(|_lua, ()| Ok(LuaScrollState::new()))?,
    )?;

    // sshwarma.view_stack() -> LuaViewStack
    sshwarma.set(
        "view_stack",
        lua.create_function(|_lua, ()| Ok(LuaViewStack::new()))?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_state_basic() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);

        assert_eq!(state.max_offset(), 40);
        assert_eq!(state.offset, 40); // Tail mode
        assert!(state.at_bottom());

        state.scroll_up(5);
        assert_eq!(state.offset, 35);
        assert_eq!(state.mode, ScrollMode::Pinned);

        state.scroll_to_bottom();
        assert_eq!(state.offset, 40);
        assert_eq!(state.mode, ScrollMode::Tail);
    }

    #[test]
    fn test_scroll_state_page_navigation() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.scroll_to_top();

        assert_eq!(state.offset, 0);

        state.page_down();
        assert_eq!(state.offset, 8); // viewport - 2

        state.page_up();
        assert_eq!(state.offset, 0);
    }

    #[test]
    fn test_scroll_state_visible_range() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.offset = 20;

        let (start, end) = state.visible_range();
        assert_eq!(start, 20);
        assert_eq!(end, 30);
    }

    #[test]
    fn test_scroll_percent() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);

        state.scroll_to_top();
        assert!((state.scroll_percent() - 0.0).abs() < 0.01);

        state.scroll_to_bottom();
        assert!((state.scroll_percent() - 1.0).abs() < 0.01);

        state.offset = 20;
        assert!((state.scroll_percent() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_view_layer() {
        let layer = ViewLayer::new("buffer")
            .with_data("buffer_id", "buf123")
            .with_data("title", "Chat");

        assert_eq!(layer.kind, "buffer");
        assert_eq!(layer.data.get("buffer_id"), Some(&"buf123".to_string()));
        assert!(layer.focusable);
    }

    #[test]
    fn test_view_stack() {
        let mut stack = ViewStack::new();
        assert!(stack.is_empty());

        stack.push(ViewLayer::new("buffer").with_data("id", "1"));
        stack.push(ViewLayer::new("modal").with_data("id", "2"));

        assert_eq!(stack.len(), 2);
        assert_eq!(stack.top().unwrap().kind, "modal");

        let popped = stack.pop().unwrap();
        assert_eq!(popped.kind, "modal");
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn test_lua_scroll_integration() -> anyhow::Result<()> {
        let lua = Lua::new();

        let sshwarma = lua.create_table()?;
        lua.globals().set("sshwarma", sshwarma)?;

        register_scroll_functions(&lua)?;

        lua.load(
            r#"
            local scroll = sshwarma.scroll_state()

            scroll:set_viewport_height(10)
            scroll:set_content_height(50)

            assert(scroll.max_offset == 40, "max_offset should be 40")
            assert(scroll.at_bottom, "should be at bottom (tail mode)")

            scroll:up(10)
            assert(scroll.offset == 30, "offset should be 30")
            assert(scroll.mode == "pinned", "should be pinned")

            scroll:to_bottom()
            assert(scroll.at_bottom, "should be at bottom")
            assert(scroll.mode == "tail", "should be tail mode")

            local start, stop = scroll:visible_range()
            assert(start == 40, "start should be 40")
            assert(stop == 50, "end should be 50")
        "#,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_view_stack_integration() -> anyhow::Result<()> {
        let lua = Lua::new();

        let sshwarma = lua.create_table()?;
        lua.globals().set("sshwarma", sshwarma)?;

        register_scroll_functions(&lua)?;

        lua.load(
            r#"
            local stack = sshwarma.view_stack()
            assert(stack.empty, "should be empty")

            stack:push("buffer", { buffer_id = "buf1" })
            stack:push("modal", { title = "Confirm" })

            assert(stack.len == 2, "should have 2 layers")

            local kind, data = stack:top()
            assert(kind == "modal", "top should be modal")
            assert(data.title == "Confirm", "title should match")

            kind, data = stack:pop()
            assert(kind == "modal", "popped should be modal")
            assert(stack.len == 1, "should have 1 layer")

            stack:clear()
            assert(stack.empty, "should be empty after clear")
        "#,
        )
        .exec()?;

        Ok(())
    }

    // ==========================================================================
    // Scroll state edge case tests
    // ==========================================================================

    #[test]
    fn test_content_shorter_than_viewport() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(10); // Less than viewport

        // max_offset should be 0 (can't scroll)
        assert_eq!(state.max_offset(), 0);
        assert_eq!(state.offset, 0);
        assert!(state.at_top());
        assert!(state.at_bottom()); // Both true when no scrolling possible

        // Scroll operations should be no-ops
        state.scroll_down(5);
        assert_eq!(state.offset, 0);

        state.scroll_up(5);
        assert_eq!(state.offset, 0);
    }

    #[test]
    fn test_content_equals_viewport() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(20);

        assert_eq!(state.max_offset(), 0);
        assert!(state.at_top());
        assert!(state.at_bottom());
    }

    #[test]
    fn test_content_grows_while_pinned() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);

        // Pin at position 20
        state.offset = 20;
        state.mode = ScrollMode::Pinned;

        // Content grows to 100 lines
        state.set_content_height(100);

        // Offset should stay at 20 (pinned doesn't follow)
        assert_eq!(state.offset, 20);
        assert_eq!(state.mode, ScrollMode::Pinned);
        assert_eq!(state.max_offset(), 90);
    }

    #[test]
    fn test_content_grows_while_tail() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);

        // Should be at bottom in tail mode
        assert_eq!(state.offset, 40);
        assert_eq!(state.mode, ScrollMode::Tail);

        // Content grows
        state.set_content_height(100);

        // Offset should follow (tail mode)
        assert_eq!(state.offset, 90);
        assert_eq!(state.mode, ScrollMode::Tail);
    }

    #[test]
    fn test_content_shrinks_clamps_offset() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(100);

        // Pin at high offset
        state.offset = 80;
        state.mode = ScrollMode::Pinned;

        // Content shrinks - offset must be clamped
        state.set_content_height(50);

        // max_offset is now 40, so offset should clamp to 40
        assert_eq!(state.max_offset(), 40);
        assert_eq!(state.offset, 40);
    }

    #[test]
    fn test_half_page_navigation() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(100);
        state.scroll_to_top();

        state.half_page_down();
        assert_eq!(state.offset, 5); // 10 / 2

        state.half_page_down();
        assert_eq!(state.offset, 10);

        state.half_page_up();
        assert_eq!(state.offset, 5);
    }

    #[test]
    fn test_scroll_up_from_top() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.scroll_to_top();

        // Scrolling up from top should stay at 0
        state.scroll_up(10);
        assert_eq!(state.offset, 0);
    }

    #[test]
    fn test_scroll_down_from_bottom() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.scroll_to_bottom();

        let max = state.max_offset();
        state.scroll_down(10);
        assert_eq!(state.offset, max); // Should stay at max
    }

    #[test]
    fn test_mode_transitions() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);

        // Start in tail mode at bottom
        assert_eq!(state.mode, ScrollMode::Tail);
        assert!(state.at_bottom());

        // Any scroll up -> pinned
        state.scroll_up(1);
        assert_eq!(state.mode, ScrollMode::Pinned);

        // Scroll back to bottom -> tail
        state.scroll_to_bottom();
        assert_eq!(state.mode, ScrollMode::Tail);

        // Manual pin
        state.mode = ScrollMode::Pinned;
        state.offset = 20;

        // Scrolling down to exactly bottom -> tail
        state.scroll_down(20); // 20 + 20 = 40 = max_offset
        assert_eq!(state.mode, ScrollMode::Tail);
    }

    #[test]
    fn test_visible_range_at_boundaries() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);

        // At top
        state.scroll_to_top();
        let (start, end) = state.visible_range();
        assert_eq!(start, 0);
        assert_eq!(end, 10);

        // At bottom
        state.scroll_to_bottom();
        let (start, end) = state.visible_range();
        assert_eq!(start, 40);
        assert_eq!(end, 50);
    }

    #[test]
    fn test_scroll_percent_edge_cases() {
        let mut state = ScrollState::new();

        // No content - should be 100%
        state.set_viewport_height(10);
        state.set_content_height(0);
        assert!((state.scroll_percent() - 1.0).abs() < 0.01);

        // Content smaller than viewport - should be 100%
        state.set_content_height(5);
        assert!((state.scroll_percent() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_empty_view_stack_operations() {
        let mut stack = ViewStack::new();

        // Pop from empty
        assert!(stack.pop().is_none());

        // Top of empty
        assert!(stack.top().is_none());
        assert!(stack.top_mut().is_none());

        // Focused index of empty
        assert!(stack.focused_index().is_none());
    }

    #[test]
    fn test_view_stack_focused_index() {
        let mut stack = ViewStack::new();

        // Add focusable layer
        let mut layer1 = ViewLayer::new("buffer");
        layer1.focusable = true;
        stack.push(layer1);

        assert_eq!(stack.focused_index(), Some(0));

        // Add non-focusable layer
        let mut layer2 = ViewLayer::new("overlay");
        layer2.focusable = false;
        stack.push(layer2);

        // Focused should still be index 0 (first focusable from top)
        assert_eq!(stack.focused_index(), Some(0));

        // Add another focusable layer
        let layer3 = ViewLayer::new("modal");
        stack.push(layer3);

        // Now focused should be index 2 (topmost focusable)
        assert_eq!(stack.focused_index(), Some(2));
    }

    #[test]
    fn test_viewport_resize() {
        let mut state = ScrollState::new();
        state.set_content_height(100);
        state.set_viewport_height(20);

        // At bottom in tail mode
        assert_eq!(state.offset, 80);

        // Viewport grows
        state.set_viewport_height(30);
        assert_eq!(state.max_offset(), 70);
        assert_eq!(state.offset, 70); // Tail mode follows

        // Viewport shrinks
        state.set_viewport_height(10);
        assert_eq!(state.max_offset(), 90);
        assert_eq!(state.offset, 90); // Tail mode follows
    }

    // ==========================================================================
    // OFF-BY-ONE AND BOUNDARY TESTS
    // These tests verify scroll edge cases that could cause fencepost errors
    // ==========================================================================

    #[test]
    fn test_visible_range_is_half_open_interval() {
        // visible_range returns [start, end) - end is exclusive
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.offset = 20;

        let (start, end) = state.visible_range();

        // Visible lines are 20, 21, 22, ..., 29 (10 lines total)
        // So start=20, end=30 (exclusive)
        assert_eq!(start, 20, "start should be offset");
        assert_eq!(end, 30, "end should be offset + viewport_height");
        assert_eq!(end - start, 10, "range should span exactly viewport_height lines");
    }

    #[test]
    fn test_visible_range_clips_to_content() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(25);
        state.offset = 20; // Near the end

        let (start, end) = state.visible_range();

        // Only 5 lines remain (25 - 20 = 5)
        // But visible_range should clip to content_height
        assert_eq!(start, 20);
        assert_eq!(end, 25, "end should be clipped to content_height");
        assert_eq!(end - start, 5, "only 5 lines visible when near end");
    }

    #[test]
    fn test_visible_range_exactly_at_bottom() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.scroll_to_bottom();

        let (start, end) = state.visible_range();

        // At bottom: offset = max_offset = 40
        // Visible lines: 40, 41, 42, ..., 49 (exactly 10 lines)
        assert_eq!(start, 40);
        assert_eq!(end, 50);
        assert_eq!(end - start, 10, "full viewport should be visible at bottom");
    }

    #[test]
    fn test_scroll_up_by_one_from_bottom() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.scroll_to_bottom();

        assert_eq!(state.offset, 40);

        state.scroll_up(1);

        assert_eq!(state.offset, 39, "offset should decrease by exactly 1");
        let (start, end) = state.visible_range();
        assert_eq!(start, 39);
        assert_eq!(end, 49);
    }

    #[test]
    fn test_scroll_down_by_one_from_top() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.scroll_to_top();

        assert_eq!(state.offset, 0);

        state.scroll_down(1);

        assert_eq!(state.offset, 1, "offset should increase by exactly 1");
        let (start, end) = state.visible_range();
        assert_eq!(start, 1);
        assert_eq!(end, 11);
    }

    #[test]
    fn test_max_offset_boundary() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);

        // max_offset = content_height - viewport_height = 40
        // This means lines 40-49 are visible at max scroll
        assert_eq!(state.max_offset(), 40);

        // Verify boundary: at max_offset, last visible line is content_height - 1
        state.offset = state.max_offset();
        let (_, end) = state.visible_range();
        assert_eq!(end, 50, "at max_offset, end should equal content_height");
    }

    #[test]
    fn test_page_navigation_boundaries() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(100);
        state.scroll_to_top();

        // page_down moves by (viewport - 2) to maintain context
        state.page_down();
        assert_eq!(state.offset, 8, "page_down should move by viewport - 2");

        // Another page down
        state.page_down();
        assert_eq!(state.offset, 16);

        // page_up from here
        state.page_up();
        assert_eq!(state.offset, 8);

        // page_up to top (should clamp to 0)
        state.page_up();
        assert_eq!(state.offset, 0, "page_up should not go below 0");
    }

    #[test]
    fn test_half_page_on_small_viewport() {
        let mut state = ScrollState::new();
        state.set_viewport_height(3); // Very small viewport
        state.set_content_height(20);
        state.scroll_to_top();

        // half_page_down = max(viewport/2, 1) = max(1, 1) = 1
        state.half_page_down();
        assert_eq!(state.offset, 1, "half page on small viewport should move by at least 1");

        state.half_page_down();
        assert_eq!(state.offset, 2);
    }

    #[test]
    fn test_single_line_content() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(1); // Only 1 line of content

        // max_offset = max(1 - 10, 0) = 0
        assert_eq!(state.max_offset(), 0);
        assert!(state.at_top());
        assert!(state.at_bottom());

        // visible_range should show just line 0
        let (start, end) = state.visible_range();
        assert_eq!(start, 0);
        assert_eq!(end, 1, "only 1 line of content");
    }

    #[test]
    fn test_content_exactly_fills_viewport() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(10);

        // max_offset = 10 - 10 = 0 (no scrolling possible)
        assert_eq!(state.max_offset(), 0);
        assert!(state.at_top());
        assert!(state.at_bottom());

        let (start, end) = state.visible_range();
        assert_eq!(start, 0);
        assert_eq!(end, 10);
        assert_eq!(end - start, 10, "exactly fills viewport");
    }

    #[test]
    fn test_content_one_more_than_viewport() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(11);

        // max_offset = 11 - 10 = 1 (can scroll by 1 line)
        assert_eq!(state.max_offset(), 1);

        // At top
        state.scroll_to_top();
        let (start, end) = state.visible_range();
        assert_eq!(start, 0);
        assert_eq!(end, 10);

        // At bottom
        state.scroll_to_bottom();
        let (start, end) = state.visible_range();
        assert_eq!(start, 1);
        assert_eq!(end, 11);
    }

    #[test]
    fn test_zero_viewport_height() {
        // Edge case: viewport with 0 height
        let mut state = ScrollState::new();
        state.set_viewport_height(0);
        state.set_content_height(50);

        // max_offset should be content_height (50 - 0 = 50)
        assert_eq!(state.max_offset(), 50);

        // visible_range should return empty range
        state.offset = 0;
        let (start, end) = state.visible_range();
        assert_eq!(start, 0);
        assert_eq!(end, 0, "0-height viewport means 0 visible lines");
    }

    #[test]
    fn test_scroll_preserves_mode_at_exact_boundary() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.offset = 39; // One line above bottom
        state.mode = ScrollMode::Pinned;

        // Scroll down by exactly 1 - should reach bottom and switch to tail
        state.scroll_down(1);

        assert_eq!(state.offset, 40, "should be at max_offset");
        assert_eq!(state.mode, ScrollMode::Tail, "reaching bottom should enable tail mode");
    }

    #[test]
    fn test_scroll_up_one_from_tail_pins() {
        let mut state = ScrollState::new();
        state.set_viewport_height(10);
        state.set_content_height(50);
        state.scroll_to_bottom();
        assert_eq!(state.mode, ScrollMode::Tail);

        // Any scroll up should switch to pinned
        state.scroll_up(1);

        assert_eq!(state.offset, 39);
        assert_eq!(state.mode, ScrollMode::Pinned, "scrolling up from tail should pin");
    }
}
