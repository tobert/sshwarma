//! Render API for terminal drawing
//!
//! Provides a render buffer and drawing primitives that Lua scripts use
//! to compose the terminal UI.

use crossterm::style::{Attribute, Color};
use mlua::prelude::*;

/// A single cell in the render buffer
#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub char: char,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Cell {
    pub fn new(char: char) -> Self {
        Self {
            char,
            ..Default::default()
        }
    }

    pub fn with_style(char: char, style: &Style) -> Self {
        Self {
            char,
            fg: style.fg,
            bg: style.bg,
            bold: style.bold,
            dim: style.dim,
            italic: style.italic,
            underline: style.underline,
        }
    }
}

/// Style configuration for drawing
#[derive(Debug, Clone, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// Parse a color from string
    /// Supports: hex (#rrggbb), named colors (red, blue, etc.)
    pub fn parse_color(s: &str) -> Option<Color> {
        // Hex color
        if s.starts_with('#') && s.len() == 7 {
            let r = u8::from_str_radix(&s[1..3], 16).ok()?;
            let g = u8::from_str_radix(&s[3..5], 16).ok()?;
            let b = u8::from_str_radix(&s[5..7], 16).ok()?;
            return Some(Color::Rgb { r, g, b });
        }

        // Named colors (Tokyo Night palette + basics)
        Some(match s.to_lowercase().as_str() {
            // Tokyo Night palette
            "dim" => Color::Rgb {
                r: 86,
                g: 95,
                b: 137,
            },
            "cyan" => Color::Rgb {
                r: 125,
                g: 207,
                b: 255,
            },
            "blue" => Color::Rgb {
                r: 122,
                g: 162,
                b: 247,
            },
            "green" => Color::Rgb {
                r: 158,
                g: 206,
                b: 106,
            },
            "yellow" => Color::Rgb {
                r: 224,
                g: 175,
                b: 104,
            },
            "red" => Color::Rgb {
                r: 247,
                g: 118,
                b: 142,
            },
            "orange" => Color::Rgb {
                r: 255,
                g: 158,
                b: 100,
            },
            "magenta" => Color::Rgb {
                r: 187,
                g: 154,
                b: 247,
            },

            // Basic colors
            "black" => Color::Black,
            "white" => Color::White,
            "grey" | "gray" => Color::Grey,
            "darkgrey" | "darkgray" => Color::DarkGrey,

            _ => return None,
        })
    }

    /// Parse style from Lua table
    pub fn from_lua_table(table: &LuaTable) -> Self {
        let mut style = Style::new();

        if let Ok(fg) = table.get::<String>("fg") {
            style.fg = Self::parse_color(&fg);
        }
        if let Ok(bg) = table.get::<String>("bg") {
            style.bg = Self::parse_color(&bg);
        }
        if let Ok(true) = table.get::<bool>("bold") {
            style.bold = true;
        }
        if let Ok(true) = table.get::<bool>("dim") {
            style.dim = true;
        }
        if let Ok(true) = table.get::<bool>("italic") {
            style.italic = true;
        }
        if let Ok(true) = table.get::<bool>("underline") {
            style.underline = true;
        }

        style
    }
}

/// A 2D buffer of cells for composing terminal output
#[derive(Debug, Clone)]
pub struct RenderBuffer {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl RenderBuffer {
    pub fn new(width: u16, height: u16) -> Self {
        let size = (width as usize) * (height as usize);
        Self {
            width,
            height,
            cells: vec![Cell::default(); size],
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y as usize) * (self.width as usize) + (x as usize))
        } else {
            None
        }
    }

    /// Get a cell (returns None if out of bounds)
    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    /// Get a mutable cell (returns None if out of bounds)
    pub fn get_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        self.index(x, y).map(|i| &mut self.cells[i])
    }

    /// Set a character at position with style
    pub fn set(&mut self, x: u16, y: u16, c: char, style: &Style) {
        if let Some(cell) = self.get_mut(x, y) {
            *cell = Cell::with_style(c, style);
        }
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::default();
        }
    }

    /// Fill a region with a character and style
    pub fn fill(&mut self, x: u16, y: u16, w: u16, h: u16, c: char, style: &Style) {
        for dy in 0..h {
            for dx in 0..w {
                self.set(x + dx, y + dy, c, style);
            }
        }
    }

    /// Print text at position with style
    pub fn print(&mut self, x: u16, y: u16, text: &str, style: &Style) {
        let mut cx = x;
        for c in text.chars() {
            if cx >= self.width {
                break;
            }
            self.set(cx, y, c, style);
            cx += 1;
        }
    }

    /// Draw horizontal line
    pub fn hline(&mut self, x: u16, y: u16, len: u16, style: &Style) {
        self.fill(x, y, len, 1, '─', style);
    }

    /// Draw vertical line
    pub fn vline(&mut self, x: u16, y: u16, len: u16, style: &Style) {
        self.fill(x, y, 1, len, '│', style);
    }

    /// Draw a box (single line)
    pub fn draw_box(&mut self, x: u16, y: u16, w: u16, h: u16, style: &Style) {
        if w < 2 || h < 2 {
            return;
        }

        // Corners
        self.set(x, y, '╭', style);
        self.set(x + w - 1, y, '╮', style);
        self.set(x, y + h - 1, '╰', style);
        self.set(x + w - 1, y + h - 1, '╯', style);

        // Horizontal edges
        for dx in 1..w - 1 {
            self.set(x + dx, y, '─', style);
            self.set(x + dx, y + h - 1, '─', style);
        }

        // Vertical edges
        for dy in 1..h - 1 {
            self.set(x, y + dy, '│', style);
            self.set(x + w - 1, y + dy, '│', style);
        }
    }

    /// Draw a horizontal gauge (progress bar)
    /// value: 0.0 to 1.0
    pub fn gauge(&mut self, x: u16, y: u16, w: u16, value: f64, style: &Style) {
        let filled = ((value.clamp(0.0, 1.0) * w as f64) as u16).min(w);

        for dx in 0..w {
            let c = if dx < filled { '█' } else { '░' };
            self.set(x + dx, y, c, style);
        }
    }

    /// Draw a sparkline from values
    pub fn sparkline(&mut self, x: u16, y: u16, values: &[f64], style: &Style) {
        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

        let max = values.iter().cloned().fold(0.0f64, f64::max);
        let min = values.iter().cloned().fold(f64::MAX, f64::min);
        let range = max - min;

        for (i, &val) in values.iter().enumerate() {
            let normalized = if range > 0.0 {
                (val - min) / range
            } else {
                0.5
            };
            let bar_idx = ((normalized * 7.0) as usize).min(7);
            self.set(x + i as u16, y, BARS[bar_idx], style);
        }
    }

    /// Draw a meter with label
    pub fn meter(&mut self, x: u16, y: u16, w: u16, value: f64, label: &str, style: &Style) {
        // Print label first
        self.print(x, y, label, style);

        // Draw gauge after label
        let label_len = label.chars().count() as u16;
        let gauge_x = x + label_len + 1;
        let gauge_w = w.saturating_sub(label_len + 1);

        if gauge_w > 0 {
            self.gauge(gauge_x, y, gauge_w, value, style);
        }
    }

    /// Render buffer to ANSI string
    pub fn to_ansi(&self) -> String {
        use crossterm::style::{ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
        use crossterm::Command;

        let mut output = String::new();

        for y in 0..self.height {
            if y > 0 {
                output.push_str("\r\n");
            }

            let mut last_fg: Option<Color> = None;
            let mut last_bg: Option<Color> = None;
            let mut last_bold = false;
            let mut last_dim = false;

            for x in 0..self.width {
                let cell = &self.cells[(y as usize) * (self.width as usize) + (x as usize)];

                // Apply style changes
                let needs_reset = (cell.bold != last_bold) || (cell.dim != last_dim);
                if needs_reset {
                    let _ = ResetColor.write_ansi(&mut output);
                    last_fg = None;
                    last_bg = None;
                    last_bold = false;
                    last_dim = false;
                }

                if cell.bold && !last_bold {
                    let _ = SetAttribute(Attribute::Bold).write_ansi(&mut output);
                    last_bold = true;
                }

                if cell.dim && !last_dim {
                    let _ = SetAttribute(Attribute::Dim).write_ansi(&mut output);
                    last_dim = true;
                }

                if cell.fg != last_fg {
                    if let Some(color) = cell.fg {
                        let _ = SetForegroundColor(color).write_ansi(&mut output);
                    }
                    last_fg = cell.fg;
                }

                if cell.bg != last_bg {
                    if let Some(color) = cell.bg {
                        let _ = SetBackgroundColor(color).write_ansi(&mut output);
                    }
                    last_bg = cell.bg;
                }

                // Output character
                output.push(if cell.char == '\0' { ' ' } else { cell.char });
            }

            // Reset at end of line
            let _ = ResetColor.write_ansi(&mut output);
        }

        output
    }
}

/// Lua userdata for drawing context
/// Wraps RenderBuffer with region bounds for clipped drawing
#[derive(Clone)]
pub struct LuaDrawContext {
    /// The underlying buffer (shared via Rc<RefCell>)
    buffer: std::sync::Arc<std::sync::Mutex<RenderBuffer>>,
    /// Region bounds for clipping
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl LuaDrawContext {
    pub fn new(
        buffer: std::sync::Arc<std::sync::Mutex<RenderBuffer>>,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Self {
        Self {
            buffer,
            x,
            y,
            width,
            height,
        }
    }

    fn with_buffer<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut RenderBuffer) -> R,
    {
        let mut buf = self.buffer.lock().unwrap();
        f(&mut buf)
    }

    /// Translate local coords to buffer coords, checking bounds
    fn translate(&self, lx: u16, ly: u16) -> Option<(u16, u16)> {
        if lx < self.width && ly < self.height {
            Some((self.x + lx, self.y + ly))
        } else {
            None
        }
    }
}

impl LuaUserData for LuaDrawContext {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Field access
        methods.add_meta_method(mlua::MetaMethod::Index, |_lua, this, key: String| match key
            .as_str()
        {
            "x" => Ok(LuaValue::Integer(this.x as i32)),
            "y" => Ok(LuaValue::Integer(this.y as i32)),
            "w" | "width" => Ok(LuaValue::Integer(this.width as i32)),
            "h" | "height" => Ok(LuaValue::Integer(this.height as i32)),
            _ => Ok(LuaValue::Nil),
        });

        // ctx:print(x, y, text, style?)
        methods.add_method(
            "print",
            |_lua, this, (x, y, text, style): (u16, u16, String, Option<LuaTable>)| {
                let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
                if let Some((bx, by)) = this.translate(x, y) {
                    // Clip text to region width
                    let max_len = this.width.saturating_sub(x) as usize;
                    let text: String = text.chars().take(max_len).collect();
                    this.with_buffer(|buf| buf.print(bx, by, &text, &style));
                }
                Ok(())
            },
        );

        // ctx:fill(x, y, w, h, char?, style?)
        methods.add_method("fill", |_lua, this, (x, y, w, h, c, style): (u16, u16, u16, u16, Option<String>, Option<LuaTable>)| {
            let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
            let c = c.and_then(|s| s.chars().next()).unwrap_or(' ');
            if let Some((bx, by)) = this.translate(x, y) {
                // Clip dimensions
                let w = w.min(this.width.saturating_sub(x));
                let h = h.min(this.height.saturating_sub(y));
                this.with_buffer(|buf| buf.fill(bx, by, w, h, c, &style));
            }
            Ok(())
        });

        // ctx:clear()
        methods.add_method("clear", |_lua, this, ()| {
            let style = Style::default();
            this.with_buffer(|buf| {
                buf.fill(this.x, this.y, this.width, this.height, ' ', &style);
            });
            Ok(())
        });

        // ctx:hline(x, y, len, style?)
        methods.add_method(
            "hline",
            |_lua, this, (x, y, len, style): (u16, u16, u16, Option<LuaTable>)| {
                let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
                if let Some((bx, by)) = this.translate(x, y) {
                    let len = len.min(this.width.saturating_sub(x));
                    this.with_buffer(|buf| buf.hline(bx, by, len, &style));
                }
                Ok(())
            },
        );

        // ctx:vline(x, y, len, style?)
        methods.add_method(
            "vline",
            |_lua, this, (x, y, len, style): (u16, u16, u16, Option<LuaTable>)| {
                let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
                if let Some((bx, by)) = this.translate(x, y) {
                    let len = len.min(this.height.saturating_sub(y));
                    this.with_buffer(|buf| buf.vline(bx, by, len, &style));
                }
                Ok(())
            },
        );

        // ctx:box(x, y, w, h, style?)
        methods.add_method(
            "box",
            |_lua, this, (x, y, w, h, style): (u16, u16, u16, u16, Option<LuaTable>)| {
                let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
                if let Some((bx, by)) = this.translate(x, y) {
                    let w = w.min(this.width.saturating_sub(x));
                    let h = h.min(this.height.saturating_sub(y));
                    this.with_buffer(|buf| buf.draw_box(bx, by, w, h, &style));
                }
                Ok(())
            },
        );

        // ctx:gauge(x, y, w, value, style?)
        methods.add_method(
            "gauge",
            |_lua, this, (x, y, w, value, style): (u16, u16, u16, f64, Option<LuaTable>)| {
                let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
                if let Some((bx, by)) = this.translate(x, y) {
                    let w = w.min(this.width.saturating_sub(x));
                    this.with_buffer(|buf| buf.gauge(bx, by, w, value, &style));
                }
                Ok(())
            },
        );

        // ctx:sparkline(x, y, values, style?)
        methods.add_method(
            "sparkline",
            |_lua, this, (x, y, values, style): (u16, u16, Vec<f64>, Option<LuaTable>)| {
                let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
                if let Some((bx, by)) = this.translate(x, y) {
                    // Clip values to width
                    let max_len = this.width.saturating_sub(x) as usize;
                    let values: Vec<f64> = values.into_iter().take(max_len).collect();
                    this.with_buffer(|buf| buf.sparkline(bx, by, &values, &style));
                }
                Ok(())
            },
        );

        // ctx:meter(x, y, w, value, label, style?)
        methods.add_method("meter", |_lua, this, (x, y, w, value, label, style): (u16, u16, u16, f64, String, Option<LuaTable>)| {
            let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
            if let Some((bx, by)) = this.translate(x, y) {
                let w = w.min(this.width.saturating_sub(x));
                this.with_buffer(|buf| buf.meter(bx, by, w, value, &label, &style));
            }
            Ok(())
        });

        // ctx:progress(x, y, w, value, style?)
        // Alias for gauge with different visual
        methods.add_method(
            "progress",
            |_lua, this, (x, y, w, value, style): (u16, u16, u16, f64, Option<LuaTable>)| {
                let style = style.map(|t| Style::from_lua_table(&t)).unwrap_or_default();
                if let Some((bx, by)) = this.translate(x, y) {
                    let w = w.min(this.width.saturating_sub(x));
                    this.with_buffer(|buf| buf.gauge(bx, by, w, value, &style));
                }
                Ok(())
            },
        );

        // ctx:sub(x, y, w, h) -> LuaDrawContext
        methods.add_method("sub", |_lua, this, (x, y, w, h): (u16, u16, u16, u16)| {
            let new_x = this.x + x.min(this.width);
            let new_y = this.y + y.min(this.height);
            let new_w = w.min(this.width.saturating_sub(x));
            let new_h = h.min(this.height.saturating_sub(y));

            Ok(LuaDrawContext {
                buffer: this.buffer.clone(),
                x: new_x,
                y: new_y,
                width: new_w,
                height: new_h,
            })
        });
    }
}

/// Register render functions in Lua
pub fn register_render_functions(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    let sshwarma: LuaTable = globals.get("sshwarma")?;

    // sshwarma.render_buffer(width, height) -> RenderBuffer handle (table)
    sshwarma.set(
        "render_buffer",
        lua.create_function(|lua, (width, height): (u16, u16)| {
            let buffer = RenderBuffer::new(width, height);
            let buffer_arc = std::sync::Arc::new(std::sync::Mutex::new(buffer));

            // Create a table with buffer methods
            let tbl = lua.create_table()?;
            tbl.set("width", width)?;
            tbl.set("height", height)?;

            // ctx(x, y, w, h) -> LuaDrawContext
            let buffer_clone = buffer_arc.clone();
            tbl.set(
                "ctx",
                lua.create_function(move |_lua, (x, y, w, h): (u16, u16, u16, u16)| {
                    Ok(LuaDrawContext::new(buffer_clone.clone(), x, y, w, h))
                })?,
            )?;

            // to_ansi() -> string
            let buffer_clone = buffer_arc.clone();
            tbl.set(
                "to_ansi",
                lua.create_function(move |_lua, ()| {
                    let buf = buffer_clone.lock().unwrap();
                    Ok(buf.to_ansi())
                })?,
            )?;

            Ok(tbl)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_default() {
        let cell = Cell::default();
        assert_eq!(cell.char, '\0');
        assert!(cell.fg.is_none());
        assert!(!cell.bold);
    }

    #[test]
    fn test_style_parse_color() {
        // Hex colors
        let color = Style::parse_color("#ff0000").unwrap();
        assert!(matches!(color, Color::Rgb { r: 255, g: 0, b: 0 }));

        // Named colors
        let cyan = Style::parse_color("cyan").unwrap();
        assert!(matches!(
            cyan,
            Color::Rgb {
                r: 125,
                g: 207,
                b: 255
            }
        ));

        // Unknown
        assert!(Style::parse_color("notacolor").is_none());
    }

    #[test]
    fn test_render_buffer_basics() {
        let mut buf = RenderBuffer::new(10, 5);
        assert_eq!(buf.width(), 10);
        assert_eq!(buf.height(), 5);

        let style = Style::new().fg(Color::Red);
        buf.print(0, 0, "Hello", &style);

        assert_eq!(buf.get(0, 0).unwrap().char, 'H');
        assert_eq!(buf.get(4, 0).unwrap().char, 'o');
        assert_eq!(buf.get(5, 0).unwrap().char, '\0'); // Not printed
    }

    #[test]
    fn test_render_buffer_fill() {
        let mut buf = RenderBuffer::new(10, 5);
        let style = Style::new();

        buf.fill(2, 1, 3, 2, '#', &style);

        assert_eq!(buf.get(2, 1).unwrap().char, '#');
        assert_eq!(buf.get(4, 2).unwrap().char, '#');
        assert_eq!(buf.get(1, 1).unwrap().char, '\0'); // Outside fill
    }

    #[test]
    fn test_render_buffer_gauge() {
        let mut buf = RenderBuffer::new(10, 1);
        let style = Style::new();

        buf.gauge(0, 0, 10, 0.5, &style);

        // First 5 should be filled
        assert_eq!(buf.get(0, 0).unwrap().char, '█');
        assert_eq!(buf.get(4, 0).unwrap().char, '█');
        assert_eq!(buf.get(5, 0).unwrap().char, '░');
        assert_eq!(buf.get(9, 0).unwrap().char, '░');
    }

    #[test]
    fn test_render_buffer_box() {
        let mut buf = RenderBuffer::new(5, 3);
        let style = Style::new();

        buf.draw_box(0, 0, 5, 3, &style);

        assert_eq!(buf.get(0, 0).unwrap().char, '╭');
        assert_eq!(buf.get(4, 0).unwrap().char, '╮');
        assert_eq!(buf.get(0, 2).unwrap().char, '╰');
        assert_eq!(buf.get(4, 2).unwrap().char, '╯');
        assert_eq!(buf.get(2, 0).unwrap().char, '─');
        assert_eq!(buf.get(0, 1).unwrap().char, '│');
    }

    #[test]
    fn test_draw_context_clipping() {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(RenderBuffer::new(20, 10)));
        let ctx = LuaDrawContext::new(buffer.clone(), 5, 2, 10, 5);

        // Drawing at (0,0) in context should appear at (5,2) in buffer
        ctx.with_buffer(|buf| {
            buf.print(5, 2, "Hi", &Style::new());
        });

        let buf = buffer.lock().unwrap();
        assert_eq!(buf.get(5, 2).unwrap().char, 'H');
        assert_eq!(buf.get(6, 2).unwrap().char, 'i');
    }

    #[test]
    fn test_to_ansi_output() {
        let mut buf = RenderBuffer::new(5, 2);
        buf.print(0, 0, "Hello", &Style::new());
        buf.print(0, 1, "World", &Style::new());

        let ansi = buf.to_ansi();
        assert!(ansi.contains("Hello"));
        assert!(ansi.contains("World"));
        assert!(ansi.contains("\r\n"));
    }

    #[test]
    fn test_lua_render_integration() -> anyhow::Result<()> {
        let lua = Lua::new();

        // Create sshwarma table
        let sshwarma = lua.create_table()?;
        lua.globals().set("sshwarma", sshwarma)?;

        register_render_functions(&lua)?;

        lua.load(
            r#"
            local buf = sshwarma.render_buffer(20, 5)
            assert(buf.width == 20)
            assert(buf.height == 5)

            local ctx = buf.ctx(0, 0, 20, 5)
            ctx:print(0, 0, "Hello", { fg = "cyan", bold = true })
            ctx:fill(0, 1, 10, 1, "=")
            ctx:gauge(0, 2, 10, 0.75)
            ctx:box(0, 3, 5, 2)

            local ansi = buf.to_ansi()
            assert(ansi:find("Hello"), "Should contain Hello")
        "#,
        )
        .exec()?;

        Ok(())
    }
}
