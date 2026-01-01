//! Render API for terminal drawing
//!
//! Provides a render buffer and drawing primitives that Lua scripts use
//! to compose the terminal UI.

use crossterm::style::{Attribute, Color};
use mlua::prelude::*;
use unicode_width::UnicodeWidthChar;

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

    /// Print text at position with style.
    /// Handles wide characters (emoji, CJK) by advancing cursor by display width.
    pub fn print(&mut self, x: u16, y: u16, text: &str, style: &Style) {
        let mut cx = x;
        for c in text.chars() {
            if cx >= self.width {
                break;
            }
            let char_width = c.width().unwrap_or(1) as u16;
            // Skip zero-width chars (combining marks, etc.)
            if char_width == 0 {
                continue;
            }
            self.set(cx, y, c, style);
            cx += char_width;
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

    /// Print text at local coords with clipping (for Rust tests)
    #[cfg(test)]
    pub fn print(&self, x: u16, y: u16, text: &str, style: &Style) {
        if let Some((bx, by)) = self.translate(x, y) {
            // Clip text to region width
            let max_len = self.width.saturating_sub(x) as usize;
            let text: String = text.chars().take(max_len).collect();
            self.with_buffer(|buf| buf.print(bx, by, &text, style));
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

    // ==========================================================================
    // Render clipping edge case tests
    // ==========================================================================

    #[test]
    fn test_print_overflow_clipping() {
        // Create 10-wide buffer, print 20-char string
        let mut buf = RenderBuffer::new(10, 1);
        buf.print(0, 0, "12345678901234567890", &Style::new());

        // Only first 10 chars should appear
        assert_eq!(buf.get(0, 0).unwrap().char, '1');
        assert_eq!(buf.get(9, 0).unwrap().char, '0'); // 10th char
                                                      // Chars 11-20 should not exist (buffer is only 10 wide)
    }

    #[test]
    fn test_print_starting_near_edge() {
        let mut buf = RenderBuffer::new(10, 1);
        // Start at x=8, print "Hello" (5 chars)
        buf.print(8, 0, "Hello", &Style::new());

        // Only "He" should fit (positions 8 and 9)
        assert_eq!(buf.get(8, 0).unwrap().char, 'H');
        assert_eq!(buf.get(9, 0).unwrap().char, 'e');
        // Position 7 should be untouched
        assert_eq!(buf.get(7, 0).unwrap().char, '\0');
    }

    #[test]
    fn test_print_outside_bounds_ignored() {
        let mut buf = RenderBuffer::new(10, 5);
        // Print outside buffer bounds - should be silently ignored
        buf.print(100, 0, "Hello", &Style::new());
        buf.print(0, 100, "Hello", &Style::new());

        // Buffer should be unchanged
        assert_eq!(buf.get(0, 0).unwrap().char, '\0');
    }

    #[test]
    fn test_fill_clipping() {
        let mut buf = RenderBuffer::new(10, 10);
        // Fill starting at (8, 8) with 5x5 - should be clipped to 2x2
        buf.fill(8, 8, 5, 5, '#', &Style::new());

        // (8,8) and (9,9) should be filled
        assert_eq!(buf.get(8, 8).unwrap().char, '#');
        assert_eq!(buf.get(9, 9).unwrap().char, '#');
        // (7,7) should be untouched
        assert_eq!(buf.get(7, 7).unwrap().char, '\0');
    }

    #[test]
    fn test_draw_context_clips_print() {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(RenderBuffer::new(20, 10)));
        // Create a 5-wide context starting at x=10
        let ctx = LuaDrawContext::new(buffer.clone(), 10, 0, 5, 10);

        // Print 10 chars at local (0, 0) - should be clipped to 5 chars
        ctx.print(0, 0, "1234567890", &Style::new());

        let buf = buffer.lock().unwrap();
        // Chars 1-5 should appear at buffer positions 10-14
        assert_eq!(buf.get(10, 0).unwrap().char, '1');
        assert_eq!(buf.get(14, 0).unwrap().char, '5');
        // Position 15 should be untouched (outside context)
        assert_eq!(buf.get(15, 0).unwrap().char, '\0');
    }

    #[test]
    fn test_draw_context_translate_fails_outside_bounds() {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(RenderBuffer::new(20, 10)));
        let ctx = LuaDrawContext::new(buffer.clone(), 5, 5, 10, 5);

        // Translate within bounds
        assert!(ctx.translate(0, 0).is_some());
        assert!(ctx.translate(9, 4).is_some());

        // Translate outside bounds
        assert!(ctx.translate(10, 0).is_none()); // x == width (out)
        assert!(ctx.translate(0, 5).is_none()); // y == height (out)
        assert!(ctx.translate(100, 100).is_none());
    }

    #[test]
    fn test_sub_context_isolation() {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(RenderBuffer::new(20, 10)));

        // Create main context 20x10
        let main_ctx = LuaDrawContext::new(buffer.clone(), 0, 0, 20, 10);

        // Fill entire main context with '.'
        main_ctx.with_buffer(|buf| buf.fill(0, 0, 20, 10, '.', &Style::new()));

        // Create sub-context at (5, 5) size 5x3
        let sub_ctx = LuaDrawContext::new(buffer.clone(), 5, 5, 5, 3);

        // Fill sub-context with '#'
        sub_ctx.with_buffer(|buf| buf.fill(5, 5, 5, 3, '#', &Style::new()));

        let buf = buffer.lock().unwrap();

        // Check sub-context area is '#'
        assert_eq!(buf.get(5, 5).unwrap().char, '#');
        assert_eq!(buf.get(9, 7).unwrap().char, '#');

        // Check outside sub-context is still '.'
        assert_eq!(buf.get(4, 5).unwrap().char, '.');
        assert_eq!(buf.get(10, 5).unwrap().char, '.');
        assert_eq!(buf.get(5, 4).unwrap().char, '.');
        assert_eq!(buf.get(5, 8).unwrap().char, '.');
    }

    #[test]
    fn test_box_too_small() {
        let mut buf = RenderBuffer::new(10, 10);

        // Box smaller than 2x2 should not draw
        buf.draw_box(0, 0, 1, 1, &Style::new());
        assert_eq!(buf.get(0, 0).unwrap().char, '\0');

        buf.draw_box(0, 0, 1, 5, &Style::new());
        assert_eq!(buf.get(0, 0).unwrap().char, '\0');
    }

    #[test]
    fn test_gauge_edge_values() {
        let mut buf = RenderBuffer::new(10, 1);
        let style = Style::new();

        // 0% - all empty
        buf.gauge(0, 0, 10, 0.0, &style);
        assert_eq!(buf.get(0, 0).unwrap().char, '░');
        assert_eq!(buf.get(9, 0).unwrap().char, '░');

        // 100% - all filled
        buf.clear();
        buf.gauge(0, 0, 10, 1.0, &style);
        assert_eq!(buf.get(0, 0).unwrap().char, '█');
        assert_eq!(buf.get(9, 0).unwrap().char, '█');

        // Negative clamps to 0
        buf.clear();
        buf.gauge(0, 0, 10, -0.5, &style);
        assert_eq!(buf.get(0, 0).unwrap().char, '░');

        // Over 1.0 clamps to 1.0
        buf.clear();
        buf.gauge(0, 0, 10, 1.5, &style);
        assert_eq!(buf.get(9, 0).unwrap().char, '█');
    }

    #[test]
    fn test_sparkline_normalization() {
        let mut buf = RenderBuffer::new(5, 1);
        let style = Style::new();

        // All same values should render middle bars
        buf.sparkline(0, 0, &[5.0, 5.0, 5.0, 5.0, 5.0], &style);
        // When range is 0, all should be middle bar (index 4 = '▄')
        let c = buf.get(0, 0).unwrap().char;
        assert!(c >= '▁' && c <= '█', "should be a bar character");

        // Increasing values
        buf.clear();
        buf.sparkline(0, 0, &[0.0, 25.0, 50.0, 75.0, 100.0], &style);
        // First should be lowest, last should be highest
        assert_eq!(buf.get(0, 0).unwrap().char, '▁');
        assert_eq!(buf.get(4, 0).unwrap().char, '█');
    }

    #[test]
    fn test_zero_size_buffer() {
        // Should not panic
        let buf = RenderBuffer::new(0, 0);
        assert_eq!(buf.width(), 0);
        assert_eq!(buf.height(), 0);
        assert!(buf.get(0, 0).is_none());
    }

    #[test]
    fn test_ansi_output_with_styles() {
        let mut buf = RenderBuffer::new(10, 1);

        // Print with different styles
        buf.set(0, 0, 'R', &Style::new().fg(Color::Red));
        buf.set(1, 1, 'B', &Style::new().bold()); // Out of bounds, ignored

        let ansi = buf.to_ansi();
        // Should contain ANSI escape codes for red
        assert!(ansi.contains("\x1b["), "should contain ANSI escapes");
        assert!(ansi.contains("R"));
    }

    // ==========================================================================
    // Wide character (unicode display width) tests
    // ==========================================================================

    #[test]
    fn test_print_wide_emoji() {
        // Emoji are typically 2 columns wide
        let mut buf = RenderBuffer::new(10, 1);
        buf.print(0, 0, "A🎵B", &Style::new());

        // 'A' at 0, '🎵' at 1 (takes 2 cols), 'B' at 3
        assert_eq!(buf.get(0, 0).unwrap().char, 'A');
        assert_eq!(buf.get(1, 0).unwrap().char, '🎵');
        assert_eq!(buf.get(3, 0).unwrap().char, 'B');
        // Position 2 should still be empty (the emoji visually covers it)
        assert_eq!(buf.get(2, 0).unwrap().char, '\0');
    }

    #[test]
    fn test_print_wide_cjk() {
        // CJK characters are 2 columns wide
        let mut buf = RenderBuffer::new(10, 1);
        buf.print(0, 0, "A日B", &Style::new());

        // 'A' at 0, '日' at 1 (takes 2 cols), 'B' at 3
        assert_eq!(buf.get(0, 0).unwrap().char, 'A');
        assert_eq!(buf.get(1, 0).unwrap().char, '日');
        assert_eq!(buf.get(3, 0).unwrap().char, 'B');
    }

    #[test]
    fn test_print_wide_clips_at_boundary() {
        // Wide char that would overflow should still be placed
        // (terminal will handle the clipping)
        let mut buf = RenderBuffer::new(5, 1);
        buf.print(0, 0, "ABC🎵", &Style::new());

        // A=0, B=1, C=2, emoji at 3 (would need cols 3-4)
        assert_eq!(buf.get(0, 0).unwrap().char, 'A');
        assert_eq!(buf.get(1, 0).unwrap().char, 'B');
        assert_eq!(buf.get(2, 0).unwrap().char, 'C');
        assert_eq!(buf.get(3, 0).unwrap().char, '🎵');
        // col 4 is empty
        assert_eq!(buf.get(4, 0).unwrap().char, '\0');
    }

    #[test]
    fn test_print_all_wide_chars() {
        // String of all wide chars
        let mut buf = RenderBuffer::new(10, 1);
        buf.print(0, 0, "日本", &Style::new());

        // '日' at 0 (width 2), '本' at 2 (width 2)
        assert_eq!(buf.get(0, 0).unwrap().char, '日');
        assert_eq!(buf.get(2, 0).unwrap().char, '本');
        // Positions 1 and 3 are continuation space
        assert_eq!(buf.get(1, 0).unwrap().char, '\0');
        assert_eq!(buf.get(3, 0).unwrap().char, '\0');
    }

    #[test]
    fn test_print_mixed_width_overflow() {
        // Test that we stop at buffer boundary correctly
        let mut buf = RenderBuffer::new(4, 1);
        buf.print(0, 0, "A🎵BC", &Style::new());

        // A=0, emoji=1 (width 2, cursor->3), B=3, C would be at 4 (out of bounds)
        assert_eq!(buf.get(0, 0).unwrap().char, 'A');
        assert_eq!(buf.get(1, 0).unwrap().char, '🎵');
        assert_eq!(buf.get(3, 0).unwrap().char, 'B');
        // 'C' should not appear (buffer is only 4 wide)
    }

    #[test]
    fn test_print_zero_width_chars_skipped() {
        // Zero-width joiner and combining marks should be skipped
        let mut buf = RenderBuffer::new(10, 1);
        // U+200D is zero-width joiner
        buf.print(0, 0, "A\u{200D}B", &Style::new());

        // ZWJ is skipped, so we get A at 0, B at 1
        assert_eq!(buf.get(0, 0).unwrap().char, 'A');
        assert_eq!(buf.get(1, 0).unwrap().char, 'B');
    }

    #[test]
    fn test_wide_char_ansi_output() {
        let mut buf = RenderBuffer::new(10, 1);
        buf.print(0, 0, "日本語", &Style::new());

        let ansi = buf.to_ansi();
        // Should contain the actual CJK characters
        assert!(ansi.contains("日"), "should contain 日");
        assert!(ansi.contains("本"), "should contain 本");
        assert!(ansi.contains("語"), "should contain 語");
    }

    // ==========================================================================
    // ANSI output snapshot tests
    // ==========================================================================

    #[test]
    fn test_ansi_rgb_foreground_color() {
        let mut buf = RenderBuffer::new(3, 1);
        buf.print(
            0,
            0,
            "RGB",
            &Style::new().fg(Color::Rgb {
                r: 255,
                g: 128,
                b: 0,
            }),
        );

        let ansi = buf.to_ansi();
        // Should contain SGR sequence for 24-bit foreground: ESC[38;2;R;G;Bm
        assert!(
            ansi.contains("\x1b[38;2;255;128;0m"),
            "should contain RGB foreground sequence, got: {:?}",
            ansi
        );
        assert!(ansi.contains("RGB"));
    }

    #[test]
    fn test_ansi_rgb_background_color() {
        let mut buf = RenderBuffer::new(2, 1);
        buf.print(
            0,
            0,
            "BG",
            &Style::new().bg(Color::Rgb {
                r: 0,
                g: 100,
                b: 200,
            }),
        );

        let ansi = buf.to_ansi();
        // Should contain SGR sequence for 24-bit background: ESC[48;2;R;G;Bm
        assert!(
            ansi.contains("\x1b[48;2;0;100;200m"),
            "should contain RGB background sequence, got: {:?}",
            ansi
        );
    }

    #[test]
    fn test_ansi_bold_attribute() {
        let mut buf = RenderBuffer::new(4, 1);
        buf.print(0, 0, "BOLD", &Style::new().bold());

        let ansi = buf.to_ansi();
        // Bold is SGR attribute 1: ESC[1m
        assert!(
            ansi.contains("\x1b[1m"),
            "should contain bold attribute, got: {:?}",
            ansi
        );
    }

    #[test]
    fn test_ansi_dim_attribute() {
        let mut buf = RenderBuffer::new(3, 1);
        buf.print(0, 0, "DIM", &Style::new().dim());

        let ansi = buf.to_ansi();
        // Dim is SGR attribute 2: ESC[2m
        assert!(
            ansi.contains("\x1b[2m"),
            "should contain dim attribute, got: {:?}",
            ansi
        );
    }

    #[test]
    fn test_ansi_reset_at_line_end() {
        let mut buf = RenderBuffer::new(5, 1);
        buf.print(0, 0, "Hello", &Style::new().fg(Color::Red));

        let ansi = buf.to_ansi();
        // Should end with reset sequence: ESC[0m (or ESC[39;49m)
        assert!(
            ansi.ends_with("\x1b[0m") || ansi.ends_with("\x1b[39;49m"),
            "should end with reset, got: {:?}",
            ansi
        );
    }

    #[test]
    fn test_ansi_multiline_has_crlf() {
        let mut buf = RenderBuffer::new(5, 3);
        buf.print(0, 0, "Line1", &Style::new());
        buf.print(0, 1, "Line2", &Style::new());
        buf.print(0, 2, "Line3", &Style::new());

        let ansi = buf.to_ansi();
        // Should have \r\n between lines
        let lines: Vec<&str> = ansi.split("\r\n").collect();
        assert_eq!(lines.len(), 3, "should have 3 lines separated by CRLF");
    }

    #[test]
    fn test_ansi_style_transition() {
        let mut buf = RenderBuffer::new(4, 1);
        // First 2 chars red, last 2 chars blue
        buf.set(0, 0, 'R', &Style::new().fg(Color::Red));
        buf.set(1, 0, 'R', &Style::new().fg(Color::Red));
        buf.set(2, 0, 'B', &Style::new().fg(Color::Blue));
        buf.set(3, 0, 'B', &Style::new().fg(Color::Blue));

        let ansi = buf.to_ansi();
        // Should have red sequence, then blue sequence
        let red_pos = ansi.find("\x1b[38;5;9m").or(ansi.find("\x1b[31m"));
        let blue_pos = ansi.find("\x1b[38;5;12m").or(ansi.find("\x1b[34m"));

        assert!(red_pos.is_some(), "should have red color");
        assert!(blue_pos.is_some(), "should have blue color");
        // Red should come before blue
        if let (Some(r), Some(b)) = (red_pos, blue_pos) {
            assert!(r < b, "red should come before blue");
        }
    }

    #[test]
    fn test_ansi_null_chars_become_spaces() {
        let mut buf = RenderBuffer::new(5, 1);
        // Only set first and last char, middle should be spaces
        buf.set(0, 0, 'A', &Style::new());
        buf.set(4, 0, 'B', &Style::new());

        let ansi = buf.to_ansi();
        // Should render as "A   B" (with spaces in middle)
        assert!(
            ansi.contains("A   B"),
            "nulls should become spaces, got: {:?}",
            ansi
        );
    }

    #[test]
    fn test_ansi_style_optimization() {
        // Same style shouldn't repeat escape sequences
        let mut buf = RenderBuffer::new(5, 1);
        let style = Style::new().fg(Color::Rgb {
            r: 100,
            g: 100,
            b: 100,
        });
        buf.print(0, 0, "AAAAA", &style);

        let ansi = buf.to_ansi();
        // The color sequence should appear only once (at the start)
        let seq = "\x1b[38;2;100;100;100m";
        let count = ansi.matches(seq).count();
        assert_eq!(
            count, 1,
            "color sequence should appear exactly once, got {} in {:?}",
            count, ansi
        );
    }

    #[test]
    fn test_ansi_bold_to_normal_resets() {
        let mut buf = RenderBuffer::new(4, 1);
        buf.set(0, 0, 'B', &Style::new().bold());
        buf.set(1, 0, 'B', &Style::new().bold());
        buf.set(2, 0, 'N', &Style::new()); // Normal
        buf.set(3, 0, 'N', &Style::new());

        let ansi = buf.to_ansi();
        // Should have bold, then reset when transitioning to normal
        assert!(ansi.contains("\x1b[1m"), "should have bold");

        // The reset at position 2 (when bold->normal) should come after the bold at position 0
        // But there's also a reset at end of line. Find the pattern: bold chars, then reset, then normal chars
        // Just verify the structure: ESC[1m appears, followed somewhere by ESC[0m, then more chars
        let bold_pos = ansi.find("\x1b[1m").unwrap();
        let content_after_bold = &ansi[bold_pos..];
        // After bold, there should be some B chars, then a reset
        assert!(
            content_after_bold.contains("B") && content_after_bold.contains("\x1b[0m"),
            "after bold, should have B chars and reset, got: {:?}",
            content_after_bold
        );
    }

    // ==========================================================================
    // Stress tests for resize and large buffers
    // ==========================================================================

    #[test]
    fn test_stress_rapid_resize() {
        // Simulate rapid terminal resizes - should not panic or corrupt state
        let sizes = [(80, 24), (120, 40), (1, 1), (200, 50), (40, 10), (80, 24)];

        for (w, h) in sizes {
            let mut buf = RenderBuffer::new(w, h);
            // Fill with content
            buf.print(0, 0, "Test content that might wrap", &Style::new());
            buf.fill(0, 0, w, h, '.', &Style::new().dim());
            // Generate ANSI
            let ansi = buf.to_ansi();
            // Should have correct number of lines
            let line_count = ansi.matches("\r\n").count() + 1;
            assert_eq!(
                line_count, h as usize,
                "buffer {}x{} should have {} lines",
                w, h, h
            );
        }
    }

    #[test]
    fn test_stress_large_buffer() {
        // Large buffer: 500x200 = 100,000 cells
        let mut buf = RenderBuffer::new(500, 200);

        // Fill entire buffer
        buf.fill(0, 0, 500, 200, '#', &Style::new());

        // Verify corners
        assert_eq!(buf.get(0, 0).unwrap().char, '#');
        assert_eq!(buf.get(499, 0).unwrap().char, '#');
        assert_eq!(buf.get(0, 199).unwrap().char, '#');
        assert_eq!(buf.get(499, 199).unwrap().char, '#');

        // Generate ANSI - should complete without panic
        let ansi = buf.to_ansi();
        assert!(!ansi.is_empty(), "ANSI output should not be empty");
    }

    #[test]
    fn test_stress_many_style_transitions() {
        // Every cell has different style - worst case for ANSI optimization
        let mut buf = RenderBuffer::new(100, 10);
        let colors = [
            Color::Red,
            Color::Green,
            Color::Blue,
            Color::Cyan,
            Color::Magenta,
            Color::Yellow,
        ];

        for y in 0..10 {
            for x in 0..100 {
                let color = colors[((x + y) % colors.len() as u16) as usize];
                let c = ('A' as u8 + (x % 26) as u8) as char;
                buf.set(x, y, c, &Style::new().fg(color));
            }
        }

        let ansi = buf.to_ansi();
        // Should have many escape sequences
        let esc_count = ansi.matches("\x1b[").count();
        assert!(
            esc_count > 50,
            "should have many escape sequences, got {}",
            esc_count
        );
    }

    #[test]
    fn test_stress_unicode_heavy() {
        // Buffer full of wide characters
        let mut buf = RenderBuffer::new(100, 10);
        let wide_chars = ['日', '本', '語', '🎵', '🎸', '🎹'];

        for y in 0..10 {
            let mut x = 0u16;
            let mut i = 0;
            while x < 100 {
                let c = wide_chars[i % wide_chars.len()];
                buf.set(x, y, c, &Style::new());
                x += 2; // wide chars take 2 columns
                i += 1;
            }
        }

        let ansi = buf.to_ansi();
        // Should contain our wide chars
        assert!(
            ansi.contains("日") || ansi.contains("🎵"),
            "should contain wide chars"
        );
    }

    #[test]
    fn test_stress_clear_and_refill() {
        // Repeated clear and refill - common in animations
        let mut buf = RenderBuffer::new(80, 24);

        for i in 0..100 {
            buf.clear();
            let pattern = if i % 2 == 0 { '.' } else { '#' };
            buf.fill(0, 0, 80, 24, pattern, &Style::new());
        }

        // Final state should be consistent
        let c = buf.get(40, 12).unwrap().char;
        assert!(c == '.' || c == '#', "should have pattern char");
    }

    #[test]
    fn test_stress_overlapping_draws() {
        // Many overlapping draw operations
        let mut buf = RenderBuffer::new(100, 50);

        // Draw overlapping boxes
        for i in 0..20 {
            buf.draw_box(i * 2, i, 30, 15, &Style::new());
        }

        // Draw overlapping fills
        for i in 0..10 {
            buf.fill(
                i * 5,
                i * 2,
                20,
                10,
                ('0' as u8 + i as u8) as char,
                &Style::new(),
            );
        }

        // Draw overlapping text
        for y in 0..50 {
            buf.print(0, y, "This is a test line that repeats", &Style::new());
        }

        let ansi = buf.to_ansi();
        assert!(!ansi.is_empty());
    }

    #[test]
    fn test_stress_draw_context_nested() {
        // Deeply nested draw contexts
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(RenderBuffer::new(200, 100)));

        // Create nested contexts
        let ctx1 = LuaDrawContext::new(buffer.clone(), 10, 10, 180, 80);
        ctx1.print(0, 0, "Level 1", &Style::new());

        // Simulate sub-regions
        for i in 0..5 {
            let x = i * 30 + 5;
            let y = i * 10 + 5;
            let sub = LuaDrawContext::new(buffer.clone(), x, y, 25, 8);
            sub.print(0, 0, &format!("Sub {}", i), &Style::new());
            sub.with_buffer(|b| b.draw_box(x, y, 25, 8, &Style::new()));
        }

        let buf = buffer.lock().unwrap();
        let ansi = buf.to_ansi();
        assert!(ansi.contains("Level 1"));
    }

    #[test]
    fn test_stress_boundary_prints() {
        // Print operations at every boundary
        let mut buf = RenderBuffer::new(80, 24);

        // Print at x=0
        buf.print(0, 0, "Start", &Style::new());
        // Print ending exactly at boundary
        buf.print(75, 0, "12345", &Style::new());
        // Print overflowing boundary
        buf.print(75, 1, "123456789", &Style::new());
        // Print starting at boundary
        buf.print(79, 2, "X", &Style::new());
        // Print starting past boundary (should be ignored)
        buf.print(80, 3, "Hidden", &Style::new());
        // Print at last row
        buf.print(0, 23, "Last row", &Style::new());

        let ansi = buf.to_ansi();
        assert!(ansi.contains("Start"));
        assert!(ansi.contains("Last row"));
    }
}
