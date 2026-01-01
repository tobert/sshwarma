//! Region constraint resolver
//!
//! Parses layout definitions from Lua and resolves constraints to pixel coordinates.
//! Supports anchoring, sizing, and nested regions.
//!
//! # Example Layout (Lua)
//!
//! ```lua
//! local layout = {
//!     { name = "main", top = 0, bottom = -8 },       -- fill, leave 8 lines for HUD
//!     { name = "hud",  bottom = 0, height = 8 },     -- 8 lines at bottom
//! }
//! ```
//!
//! # Constraint Rules
//!
//! - `top` and `bottom` define vertical anchors (negative = from bottom)
//! - `left` and `right` define horizontal anchors (negative = from right)
//! - `width` and `height` define explicit sizes (number = pixels, string "50%" = percentage)
//! - If both anchors given, size is computed; if one anchor + size, other anchor computed
//! - `fill = true` means take remaining space after other regions placed

use mlua::{Lua, Result as LuaResult, Table, UserData, UserDataMethods, Value};
use std::collections::HashMap;
use tracing::debug;

/// A resolved rectangular region with pixel coordinates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Create a rect representing the full terminal
    pub fn full(cols: u16, rows: u16) -> Self {
        Self {
            x: 0,
            y: 0,
            width: cols,
            height: rows,
        }
    }

    /// Get the right edge (x + width)
    pub fn right(&self) -> u16 {
        self.x + self.width
    }

    /// Get the bottom edge (y + height)
    pub fn bottom(&self) -> u16 {
        self.y + self.height
    }

    /// Check if this rect contains a point
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Create a sub-rect within this rect (relative coordinates)
    pub fn sub(&self, x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x: self.x + x,
            y: self.y + y,
            width: width.min(self.width.saturating_sub(x)),
            height: height.min(self.height.saturating_sub(y)),
        }
    }

    /// Shrink rect by margins
    pub fn shrink(&self, top: u16, right: u16, bottom: u16, left: u16) -> Self {
        Self {
            x: self.x + left,
            y: self.y + top,
            width: self.width.saturating_sub(left + right),
            height: self.height.saturating_sub(top + bottom),
        }
    }
}

/// A constraint value that can be absolute or relative
#[derive(Debug, Clone, Copy)]
pub enum Constraint {
    /// Absolute pixels from the anchor
    Absolute(i32),
    /// Percentage of parent dimension (0.0 - 1.0)
    Percent(f64),
}

impl Constraint {
    /// Parse from Lua value
    pub fn from_lua(value: &Value) -> Option<Self> {
        match value {
            Value::Integer(i) => Some(Constraint::Absolute(*i)),
            Value::Number(n) => Some(Constraint::Absolute(*n as i32)),
            Value::String(s) => {
                let s = s.to_str().ok()?;
                if s.ends_with('%') {
                    let pct: f64 = s.trim_end_matches('%').parse().ok()?;
                    Some(Constraint::Percent(pct / 100.0))
                } else {
                    let n: i32 = s.parse().ok()?;
                    Some(Constraint::Absolute(n))
                }
            }
            _ => None,
        }
    }

    /// Resolve to absolute pixels given parent dimension
    pub fn resolve(&self, parent_size: u16) -> i32 {
        match self {
            Constraint::Absolute(n) => *n,
            Constraint::Percent(p) => (parent_size as f64 * p) as i32,
        }
    }

    /// Resolve a position constraint (handles negative = from end)
    pub fn resolve_position(&self, parent_size: u16) -> u16 {
        let val = self.resolve(parent_size);
        if val < 0 {
            (parent_size as i32 + val).max(0) as u16
        } else {
            val as u16
        }
    }
}

/// Unresolved region definition parsed from Lua
#[derive(Debug, Clone)]
pub struct RegionDef {
    pub name: String,
    pub top: Option<Constraint>,
    pub bottom: Option<Constraint>,
    pub left: Option<Constraint>,
    pub right: Option<Constraint>,
    pub width: Option<Constraint>,
    pub height: Option<Constraint>,
    pub fill: bool,
    pub children: Vec<RegionDef>,
    pub render_fn: Option<String>, // Name of Lua function to call for rendering
}

impl RegionDef {
    /// Create a new region definition
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            top: None,
            bottom: None,
            left: None,
            right: None,
            width: None,
            height: None,
            fill: false,
            children: Vec::new(),
            render_fn: None,
        }
    }

    /// Parse from Lua table
    pub fn from_lua_table(table: &Table) -> LuaResult<Self> {
        let name: String = table.get("name").unwrap_or_else(|_| "unnamed".to_string());

        let mut def = Self::new(name);

        // Parse position constraints
        if let Ok(v) = table.get::<Value>("top") {
            def.top = Constraint::from_lua(&v);
        }
        if let Ok(v) = table.get::<Value>("bottom") {
            def.bottom = Constraint::from_lua(&v);
        }
        if let Ok(v) = table.get::<Value>("left") {
            def.left = Constraint::from_lua(&v);
        }
        if let Ok(v) = table.get::<Value>("right") {
            def.right = Constraint::from_lua(&v);
        }

        // Parse size constraints
        if let Ok(v) = table.get::<Value>("width") {
            def.width = Constraint::from_lua(&v);
        }
        if let Ok(v) = table.get::<Value>("height") {
            def.height = Constraint::from_lua(&v);
        }

        // Parse fill flag
        def.fill = table.get("fill").unwrap_or(false);

        // Parse render function name
        if let Ok(s) = table.get::<String>("render") {
            def.render_fn = Some(s);
        }

        // Parse children
        if let Ok(children_table) = table.get::<Table>("children") {
            for (_, child) in children_table.pairs::<i64, Table>().flatten() {
                def.children.push(Self::from_lua_table(&child)?);
            }
        }

        Ok(def)
    }

    /// Resolve this region to a Rect within the given parent bounds
    pub fn resolve(&self, parent: Rect) -> Rect {
        // Resolve horizontal bounds
        let (x, width) = self.resolve_horizontal(parent.width);

        // Resolve vertical bounds
        let (y, height) = self.resolve_vertical(parent.height);

        Rect {
            x: parent.x + x,
            y: parent.y + y,
            width,
            height,
        }
    }

    /// Clamp a dimension, logging if clamping occurs
    fn clamp_dimension(&self, requested: u16, available: u16, axis: &str) -> u16 {
        if requested > available {
            debug!(
                region = %self.name,
                axis,
                requested,
                available,
                "layout constraint clamped"
            );
            available
        } else {
            requested
        }
    }

    fn resolve_horizontal(&self, parent_width: u16) -> (u16, u16) {
        match (&self.left, &self.right, &self.width) {
            // Both anchors: compute width
            (Some(l), Some(r), _) => {
                let left = l.resolve_position(parent_width);
                let right = r.resolve_position(parent_width);
                let width = right.saturating_sub(left);
                (left, width)
            }
            // Left anchor + width
            (Some(l), None, Some(w)) => {
                let left = l.resolve_position(parent_width);
                let width = w.resolve(parent_width).max(0) as u16;
                let available = parent_width.saturating_sub(left);
                (left, self.clamp_dimension(width, available, "width"))
            }
            // Right anchor + width
            (None, Some(r), Some(w)) => {
                let right = r.resolve_position(parent_width);
                let width = w.resolve(parent_width).max(0) as u16;
                let clamped = self.clamp_dimension(width, right, "width");
                let left = right.saturating_sub(clamped);
                (left, clamped)
            }
            // Just left anchor: extend to right edge
            (Some(l), None, None) => {
                let left = l.resolve_position(parent_width);
                (left, parent_width.saturating_sub(left))
            }
            // Just right anchor: extend from left edge
            (None, Some(r), None) => {
                let right = r.resolve_position(parent_width);
                (0, right)
            }
            // Just width: center, clamp to parent
            (None, None, Some(w)) => {
                let width = w.resolve(parent_width).max(0) as u16;
                let clamped = self.clamp_dimension(width, parent_width, "width");
                let left = (parent_width.saturating_sub(clamped)) / 2;
                (left, clamped)
            }
            // Nothing specified: fill parent
            (None, None, None) => (0, parent_width),
        }
    }

    fn resolve_vertical(&self, parent_height: u16) -> (u16, u16) {
        // `bottom` is always relative to parent's bottom edge:
        //   bottom = 0 → at parent bottom (y = parent_height)
        //   bottom = -8 → 8 above parent bottom (y = parent_height - 8)
        // Formula: bottom_y = parent_height + bottom_value
        match (&self.top, &self.bottom, &self.height) {
            // Both anchors: compute height from top position to bottom edge
            (Some(t), Some(b), _) => {
                let top = t.resolve_position(parent_height);
                let bottom_y = (parent_height as i32 + b.resolve(parent_height)).max(0) as u16;
                let height = bottom_y.saturating_sub(top);
                (top, height)
            }
            // Top anchor + height
            (Some(t), None, Some(h)) => {
                let top = t.resolve_position(parent_height);
                let height = h.resolve(parent_height).max(0) as u16;
                let available = parent_height.saturating_sub(top);
                (top, self.clamp_dimension(height, available, "height"))
            }
            // Bottom anchor + height: position above bottom edge
            (None, Some(b), Some(h)) => {
                let bottom_y = (parent_height as i32 + b.resolve(parent_height)).max(0) as u16;
                let height = h.resolve(parent_height).max(0) as u16;
                let clamped = self.clamp_dimension(height, bottom_y, "height");
                let top = bottom_y.saturating_sub(clamped);
                (top, clamped)
            }
            // Just top anchor: extend to bottom edge
            (Some(t), None, None) => {
                let top = t.resolve_position(parent_height);
                (top, parent_height.saturating_sub(top))
            }
            // Just bottom anchor: extend from top edge to bottom position
            (None, Some(b), None) => {
                let bottom_y = (parent_height as i32 + b.resolve(parent_height)).max(0) as u16;
                (0, bottom_y)
            }
            // Just height: position at top, clamp to parent
            (None, None, Some(h)) => {
                let height = h.resolve(parent_height).max(0) as u16;
                (0, self.clamp_dimension(height, parent_height, "height"))
            }
            // Nothing specified: fill parent
            (None, None, None) => (0, parent_height),
        }
    }
}

/// Resolved layout with named regions
#[derive(Debug, Clone)]
pub struct Layout {
    regions: HashMap<String, Rect>,
    order: Vec<String>, // Rendering order
}

impl Layout {
    /// Create a new empty layout
    pub fn new() -> Self {
        Self {
            regions: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Resolve a list of region definitions within the given bounds
    pub fn resolve(defs: &[RegionDef], bounds: Rect) -> Self {
        let mut layout = Self::new();

        // First pass: resolve non-fill regions
        let mut remaining = bounds;
        let mut fill_regions: Vec<&RegionDef> = Vec::new();

        for def in defs {
            if def.fill {
                fill_regions.push(def);
            } else {
                let rect = def.resolve(bounds);
                layout.add(def.name.clone(), rect);

                // Adjust remaining space (simple: just track vertical space used from top/bottom)
                if let Some(Constraint::Absolute(0)) = def.top {
                    remaining.y = rect.bottom();
                    remaining.height = remaining.height.saturating_sub(rect.height);
                }
                if let Some(Constraint::Absolute(0)) = def.bottom {
                    remaining.height = remaining.height.saturating_sub(rect.height);
                }
            }
        }

        // Second pass: fill regions get remaining space
        for def in fill_regions {
            layout.add(def.name.clone(), remaining);
        }

        layout
    }

    /// Add a region
    pub fn add(&mut self, name: String, rect: Rect) {
        if !self.regions.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.regions.insert(name, rect);
    }

    /// Get a region by name
    pub fn get(&self, name: &str) -> Option<Rect> {
        self.regions.get(name).copied()
    }

    /// Get all region names in rendering order
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// Iterate over regions in rendering order
    pub fn iter(&self) -> impl Iterator<Item = (&str, Rect)> {
        self.order
            .iter()
            .filter_map(|name| self.regions.get(name).map(|rect| (name.as_str(), *rect)))
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::new()
    }
}

/// Lua userdata for a resolved region (Area)
///
/// Provides bounds information and sub-area creation.
#[derive(Clone)]
pub struct LuaArea {
    pub rect: Rect,
    pub name: String,
}

impl UserData for LuaArea {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Field access
        methods.add_meta_method(mlua::MetaMethod::Index, |_lua, this, key: String| match key
            .as_str()
        {
            "x" => Ok(Value::Integer(this.rect.x as i32)),
            "y" => Ok(Value::Integer(this.rect.y as i32)),
            "w" | "width" => Ok(Value::Integer(this.rect.width as i32)),
            "h" | "height" => Ok(Value::Integer(this.rect.height as i32)),
            "name" => Ok(Value::String(_lua.create_string(&this.name)?)),
            "right" => Ok(Value::Integer(this.rect.right() as i32)),
            "bottom" => Ok(Value::Integer(this.rect.bottom() as i32)),
            _ => Ok(Value::Nil),
        });

        // area:sub(x, y, w, h) -> LuaArea
        methods.add_method("sub", |_lua, this, (x, y, w, h): (u16, u16, u16, u16)| {
            Ok(LuaArea {
                rect: this.rect.sub(x, y, w, h),
                name: format!("{}:sub", this.name),
            })
        });

        // area:shrink(top, right, bottom, left) -> LuaArea
        methods.add_method(
            "shrink",
            |_lua, this, (t, r, b, l): (u16, u16, u16, u16)| {
                Ok(LuaArea {
                    rect: this.rect.shrink(t, r, b, l),
                    name: format!("{}:shrink", this.name),
                })
            },
        );

        // area:shrink_uniform(n) -> LuaArea
        methods.add_method("shrink_uniform", |_lua, this, n: u16| {
            Ok(LuaArea {
                rect: this.rect.shrink(n, n, n, n),
                name: format!("{}:shrink", this.name),
            })
        });

        // area:contains(x, y) -> bool
        methods.add_method("contains", |_lua, this, (x, y): (u16, u16)| {
            Ok(this.rect.contains(x, y))
        });

        // area:split_vertical(at) -> (top_area, bottom_area)
        methods.add_method("split_vertical", |_lua, this, at: u16| {
            let top = LuaArea {
                rect: Rect::new(
                    this.rect.x,
                    this.rect.y,
                    this.rect.width,
                    at.min(this.rect.height),
                ),
                name: format!("{}:top", this.name),
            };
            let bottom = LuaArea {
                rect: Rect::new(
                    this.rect.x,
                    this.rect.y + at.min(this.rect.height),
                    this.rect.width,
                    this.rect.height.saturating_sub(at),
                ),
                name: format!("{}:bottom", this.name),
            };
            Ok((top, bottom))
        });

        // area:split_horizontal(at) -> (left_area, right_area)
        methods.add_method("split_horizontal", |_lua, this, at: u16| {
            let left = LuaArea {
                rect: Rect::new(
                    this.rect.x,
                    this.rect.y,
                    at.min(this.rect.width),
                    this.rect.height,
                ),
                name: format!("{}:left", this.name),
            };
            let right = LuaArea {
                rect: Rect::new(
                    this.rect.x + at.min(this.rect.width),
                    this.rect.y,
                    this.rect.width.saturating_sub(at),
                    this.rect.height,
                ),
                name: format!("{}:right", this.name),
            };
            Ok((left, right))
        });
    }
}

/// Parse layout definitions from Lua
pub fn parse_layout(_lua: &Lua, layout_table: Table) -> LuaResult<Vec<RegionDef>> {
    let mut defs = Vec::new();

    for (_, region_table) in layout_table.pairs::<i64, Table>().flatten() {
        defs.push(RegionDef::from_lua_table(&region_table)?);
    }

    Ok(defs)
}

/// Register layout functions in Lua
pub fn register_layout_functions(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // Get or create sshwarma table
    let sshwarma: Table = globals.get("sshwarma").unwrap_or_else(|_| {
        let t = lua.create_table().unwrap();
        globals.set("sshwarma", t.clone()).ok();
        t
    });

    // sshwarma.layout(defs, cols, rows) -> table of LuaArea
    let layout_fn = lua.create_function(|lua, (defs_table, cols, rows): (Table, u16, u16)| {
        let defs = parse_layout(lua, defs_table)?;
        let bounds = Rect::full(cols, rows);
        let layout = Layout::resolve(&defs, bounds);

        let result = lua.create_table()?;
        for (name, rect) in layout.iter() {
            result.set(
                name.to_string(),
                LuaArea {
                    rect,
                    name: name.to_string(),
                },
            )?;
        }
        Ok(result)
    })?;
    sshwarma.set("layout", layout_fn)?;

    // sshwarma.area(x, y, w, h) -> LuaArea
    let area_fn = lua.create_function(|_lua, (x, y, w, h): (u16, u16, u16, u16)| {
        Ok(LuaArea {
            rect: Rect::new(x, y, w, h),
            name: "custom".to_string(),
        })
    })?;
    sshwarma.set("area", area_fn)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_basic() {
        let r = Rect::new(10, 20, 100, 50);
        assert_eq!(r.right(), 110);
        assert_eq!(r.bottom(), 70);
        assert!(r.contains(10, 20));
        assert!(r.contains(109, 69));
        assert!(!r.contains(110, 70));
    }

    #[test]
    fn test_rect_sub() {
        let r = Rect::new(10, 20, 100, 50);
        let sub = r.sub(5, 5, 20, 10);
        assert_eq!(sub.x, 15);
        assert_eq!(sub.y, 25);
        assert_eq!(sub.width, 20);
        assert_eq!(sub.height, 10);
    }

    #[test]
    fn test_rect_shrink() {
        let r = Rect::new(0, 0, 100, 50);
        let shrunk = r.shrink(5, 10, 5, 10);
        assert_eq!(shrunk.x, 10);
        assert_eq!(shrunk.y, 5);
        assert_eq!(shrunk.width, 80);
        assert_eq!(shrunk.height, 40);
    }

    #[test]
    fn test_constraint_parse() {
        let lua = Lua::new();

        // Integer
        let c = Constraint::from_lua(&Value::Integer(10)).unwrap();
        assert!(matches!(c, Constraint::Absolute(10)));

        // Negative
        let c = Constraint::from_lua(&Value::Integer(-8)).unwrap();
        assert!(matches!(c, Constraint::Absolute(-8)));

        // Percentage string
        let s = lua.create_string("50%").unwrap();
        let c = Constraint::from_lua(&Value::String(s)).unwrap();
        if let Constraint::Percent(p) = c {
            assert!((p - 0.5).abs() < 0.001);
        } else {
            panic!("expected Percent");
        }
    }

    #[test]
    fn test_constraint_resolve() {
        let c = Constraint::Absolute(10);
        assert_eq!(c.resolve(100), 10);

        let c = Constraint::Percent(0.5);
        assert_eq!(c.resolve(100), 50);

        // Negative position
        let c = Constraint::Absolute(-8);
        assert_eq!(c.resolve_position(100), 92);
    }

    #[test]
    fn test_region_def_resolve_full() {
        let def = RegionDef::new("test");
        let rect = def.resolve(Rect::full(80, 24));
        assert_eq!(rect, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn test_region_def_resolve_anchors() {
        let mut def = RegionDef::new("test");
        def.top = Some(Constraint::Absolute(0));
        def.bottom = Some(Constraint::Absolute(-8)); // 8 from bottom

        let rect = def.resolve(Rect::full(80, 24));
        assert_eq!(rect.y, 0);
        assert_eq!(rect.height, 16); // 24 - 8 = 16
    }

    #[test]
    fn test_region_def_resolve_height_at_bottom() {
        let mut def = RegionDef::new("hud");
        def.bottom = Some(Constraint::Absolute(0));
        def.height = Some(Constraint::Absolute(8));

        let rect = def.resolve(Rect::full(80, 24));
        assert_eq!(rect.y, 16); // 24 - 8 = 16
        assert_eq!(rect.height, 8);
    }

    #[test]
    fn test_layout_resolve() {
        let defs = vec![
            {
                let mut d = RegionDef::new("main");
                d.top = Some(Constraint::Absolute(0));
                d.bottom = Some(Constraint::Absolute(-8));
                d
            },
            {
                let mut d = RegionDef::new("hud");
                d.bottom = Some(Constraint::Absolute(0));
                d.height = Some(Constraint::Absolute(8));
                d
            },
        ];

        let layout = Layout::resolve(&defs, Rect::full(80, 24));

        let main = layout.get("main").unwrap();
        assert_eq!(main.height, 16);

        let hud = layout.get("hud").unwrap();
        assert_eq!(hud.y, 16);
        assert_eq!(hud.height, 8);
    }

    #[test]
    fn test_layout_from_lua() -> anyhow::Result<()> {
        let lua = Lua::new();
        register_layout_functions(&lua)?;

        lua.load(
            r#"
            local regions = sshwarma.layout({
                { name = "main", top = 0, bottom = -8 },
                { name = "hud", bottom = 0, height = 8 },
            }, 80, 24)

            assert(regions.main ~= nil, "main should exist")
            assert(regions.hud ~= nil, "hud should exist")
            assert(regions.main.h == 16, "main height should be 16, got " .. regions.main.h)
            assert(regions.hud.h == 8, "hud height should be 8")
            assert(regions.hud.y == 16, "hud y should be 16")
        "#,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_area_operations() -> anyhow::Result<()> {
        let lua = Lua::new();
        register_layout_functions(&lua)?;

        lua.load(
            r#"
            local area = sshwarma.area(0, 0, 80, 24)
            assert(area.w == 80, "width should be 80")
            assert(area.h == 24, "height should be 24")

            -- Test shrink
            local inner = area:shrink(1, 1, 1, 1)
            assert(inner.x == 1)
            assert(inner.y == 1)
            assert(inner.w == 78)
            assert(inner.h == 22)

            -- Test split
            local top, bottom = area:split_vertical(10)
            assert(top.h == 10, "top height should be 10")
            assert(bottom.h == 14, "bottom height should be 14")
            assert(bottom.y == 10, "bottom y should be 10")
        "#,
        )
        .exec()?;

        Ok(())
    }

    // ==========================================================================
    // Layout constraint edge case tests
    // ==========================================================================

    #[test]
    fn test_tiny_terminal_2x2() {
        // Minimum usable terminal
        let bounds = Rect::full(2, 2);

        // Region requesting more space than available
        let mut def = RegionDef::new("hud");
        def.bottom = Some(Constraint::Absolute(0));
        def.height = Some(Constraint::Absolute(8)); // Wants 8, only 2 available

        let rect = def.resolve(bounds);
        // Should clamp to available space
        assert!(rect.height <= 2, "height {} should be <= 2", rect.height);
        // rect.y is u16, so always >= 0
    }

    #[test]
    fn test_zero_dimension_terminal() {
        // Degenerate case - should not panic
        let bounds = Rect::full(0, 0);
        let def = RegionDef::new("test");
        let rect = def.resolve(bounds);

        assert_eq!(rect.width, 0);
        assert_eq!(rect.height, 0);
    }

    #[test]
    fn test_height_larger_than_terminal() {
        let bounds = Rect::full(80, 24);

        let mut def = RegionDef::new("oversized");
        def.top = Some(Constraint::Absolute(0));
        def.height = Some(Constraint::Absolute(100)); // Way bigger than 24

        let rect = def.resolve(bounds);
        // Should be clamped to terminal height
        assert!(
            rect.bottom() <= 24,
            "bottom {} should not exceed terminal",
            rect.bottom()
        );
    }

    #[test]
    fn test_negative_anchor_past_boundary() {
        let bounds = Rect::full(80, 24);

        // bottom = -100 means "100 lines from bottom", but terminal is only 24 lines
        let mut def = RegionDef::new("weird");
        def.bottom = Some(Constraint::Absolute(-100));
        def.height = Some(Constraint::Absolute(5));

        let rect = def.resolve(bounds);
        // The bottom_y calculation: 24 + (-100) = -76, clamped to 0
        // So rect should be positioned reasonably (not negative)
        assert!(rect.y < 24, "y {} should be within bounds", rect.y);
    }

    #[test]
    fn test_percentage_constraints() {
        let bounds = Rect::full(100, 100);

        let mut def = RegionDef::new("half");
        def.width = Some(Constraint::Percent(0.5));
        def.height = Some(Constraint::Percent(0.5));

        let rect = def.resolve(bounds);
        assert_eq!(rect.width, 50);
        assert_eq!(rect.height, 50);
        // Just width specified = centered horizontally
        assert_eq!(rect.x, 25);
    }

    #[test]
    fn test_percentage_rounding() {
        // 50% of 3 = 1.5, should truncate to 1
        let bounds = Rect::full(3, 3);

        let mut def = RegionDef::new("half");
        def.width = Some(Constraint::Percent(0.5));
        def.height = Some(Constraint::Percent(0.5));

        let rect = def.resolve(bounds);
        assert_eq!(rect.width, 1); // floor(1.5)
        assert_eq!(rect.height, 1);
    }

    #[test]
    fn test_percentage_over_100() {
        let bounds = Rect::full(100, 100);

        let mut def = RegionDef::new("oversized");
        def.width = Some(Constraint::Percent(1.5)); // 150%

        let rect = def.resolve(bounds);
        // 150% of 100 = 150, clamped to parent width of 100
        assert_eq!(rect.width, 100);
        assert_eq!(rect.x, 0); // Centered: (100 - 100) / 2 = 0
    }

    #[test]
    fn test_both_anchors_ignore_explicit_size() {
        let bounds = Rect::full(80, 24);

        // When both anchors given, size is computed from them (explicit size ignored)
        let mut def = RegionDef::new("test");
        def.top = Some(Constraint::Absolute(5));
        def.bottom = Some(Constraint::Absolute(-5)); // y=19
        def.height = Some(Constraint::Absolute(100)); // Should be ignored

        let rect = def.resolve(bounds);
        // Height should be computed: 19 - 5 = 14, NOT 100
        assert_eq!(rect.y, 5);
        assert_eq!(rect.height, 14);
    }

    #[test]
    fn test_left_anchor_extends_to_right() {
        let bounds = Rect::full(80, 24);

        let mut def = RegionDef::new("sidebar");
        def.left = Some(Constraint::Absolute(60));
        // No right anchor, no width -> should extend to right edge

        let rect = def.resolve(bounds);
        assert_eq!(rect.x, 60);
        assert_eq!(rect.width, 20); // 80 - 60
        assert_eq!(rect.right(), 80);
    }

    #[test]
    fn test_right_anchor_extends_from_left() {
        let bounds = Rect::full(80, 24);

        let mut def = RegionDef::new("sidebar");
        def.right = Some(Constraint::Absolute(20)); // 20 from left edge
                                                    // No left anchor, no width -> should extend from x=0 to x=20

        let rect = def.resolve(bounds);
        assert_eq!(rect.x, 0);
        assert_eq!(rect.width, 20);
    }

    #[test]
    fn test_width_only_centers_horizontally() {
        let bounds = Rect::full(80, 24);

        let mut def = RegionDef::new("centered");
        def.width = Some(Constraint::Absolute(40));

        let rect = def.resolve(bounds);
        assert_eq!(rect.width, 40);
        assert_eq!(rect.x, 20); // (80 - 40) / 2
    }

    #[test]
    fn test_height_only_positions_at_top() {
        let bounds = Rect::full(80, 24);

        let mut def = RegionDef::new("header");
        def.height = Some(Constraint::Absolute(5));

        let rect = def.resolve(bounds);
        assert_eq!(rect.height, 5);
        assert_eq!(rect.y, 0); // Positioned at top (different from horizontal centering)
    }

    #[test]
    fn test_rect_sub_clipping() {
        let r = Rect::new(10, 10, 20, 20);

        // Sub-rect that would extend past bounds
        let sub = r.sub(15, 15, 100, 100);

        // Should be clipped to parent bounds
        assert_eq!(sub.x, 25);
        assert_eq!(sub.y, 25);
        assert_eq!(sub.width, 5); // 20 - 15
        assert_eq!(sub.height, 5);
    }

    #[test]
    fn test_rect_shrink_larger_than_rect() {
        let r = Rect::new(0, 0, 10, 10);

        // Shrink by more than the rect size
        let shrunk = r.shrink(20, 20, 20, 20);

        // Should saturate to 0, not underflow
        assert_eq!(shrunk.width, 0);
        assert_eq!(shrunk.height, 0);
    }

    #[test]
    fn test_split_at_zero() {
        let lua = Lua::new();
        register_layout_functions(&lua).unwrap();

        lua.load(
            r#"
            local area = sshwarma.area(0, 0, 80, 24)

            -- Split at 0 should give empty top, full bottom
            local top, bottom = area:split_vertical(0)
            assert(top.h == 0, "top height should be 0")
            assert(bottom.h == 24, "bottom height should be 24")
            assert(bottom.y == 0, "bottom y should be 0")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn test_split_past_bounds() {
        let lua = Lua::new();
        register_layout_functions(&lua).unwrap();

        lua.load(
            r#"
            local area = sshwarma.area(0, 0, 80, 24)

            -- Split at value larger than height
            local top, bottom = area:split_vertical(100)
            assert(top.h == 24, "top should be clamped to full height")
            assert(bottom.h == 0, "bottom should be empty")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn test_nested_sub_areas() {
        let lua = Lua::new();
        register_layout_functions(&lua).unwrap();

        lua.load(
            r#"
            local outer = sshwarma.area(10, 10, 100, 100)
            local inner = outer:sub(5, 5, 50, 50)
            local nested = inner:sub(10, 10, 20, 20)

            -- Coordinates should accumulate
            assert(outer.x == 10)
            assert(inner.x == 15, "inner x should be 10 + 5 = 15")
            assert(nested.x == 25, "nested x should be 15 + 10 = 25")
        "#,
        )
        .exec()
        .unwrap();
    }
}
