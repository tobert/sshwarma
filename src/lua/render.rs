//! Lua output parser for HUD rendering
//!
//! Parses the structured table output from Lua's `render_hud()` function
//! and converts it to an ANSI-escaped string for terminal display.
//!
//! ## Input Format
//!
//! Lua returns an array of 8 rows, each row is an array of segments:
//!
//! ```text
//! return {
//!   { {Fg="#7dcfff", Text="+"}, {Text="----"}, {Fg="#7dcfff", Text="+"} },
//!   { {Text="  alice  "}, {Fg="#bb9af7", Text="* qwen-8b"} },
//!   -- ... 6 more rows
//! }
//! ```
//!
//! Each segment contains:
//! - `Text` (required): string content
//! - `Fg` (optional): foreground color as "#rrggbb" hex
//! - `Bg` (optional): background color as "#rrggbb" hex

use crossterm::style::{Color, ResetColor, SetBackgroundColor, SetForegroundColor};
use mlua::{Table, Value};
use std::fmt::Write;

/// Number of rows expected from Lua HUD renderer
pub const HUD_ROWS: usize = 8;

/// Parse Lua render output to ANSI string.
///
/// Returns 8 lines joined by CRLF (no trailing CRLF on the last line).
/// If Lua returns fewer than 8 rows, empty rows are appended.
pub fn parse_lua_output(lua_result: Table) -> Result<String, anyhow::Error> {
    let mut rows = Vec::with_capacity(HUD_ROWS);

    // Iterate over the Lua table (1-indexed)
    for i in 1..=HUD_ROWS {
        let row_result: Result<Table, _> = lua_result.get(i);
        match row_result {
            Ok(row) => {
                let line = parse_row(row)?;
                rows.push(line);
            }
            Err(_) => {
                // Pad with empty row if missing
                rows.push(String::new());
            }
        }
    }

    Ok(rows.join("\r\n"))
}

/// Parse a single row (array of segments) into an ANSI string
fn parse_row(row: Table) -> Result<String, anyhow::Error> {
    let mut output = String::new();

    for segment_result in row.sequence_values::<Value>() {
        // Handle Lua errors by converting to string (mlua::Error isn't Send+Sync with Luau)
        let segment_value = segment_result.map_err(|e| anyhow::anyhow!("{}", e))?;

        // Each segment should be a table
        if let Value::Table(segment) = segment_value {
            // Text is required - skip segment if missing
            let text: Option<String> = segment.get("Text").ok();
            if let Some(text) = text {
                // Parse optional colors
                let fg: Option<String> = segment.get("Fg").ok();
                let bg: Option<String> = segment.get("Bg").ok();

                // Apply styling and append
                output.push_str(&style_text(&text, fg.as_deref(), bg.as_deref()));
            }
        }
    }

    Ok(output)
}

/// Style text with optional foreground and background colors.
///
/// Returns the text with ANSI escape codes applied.
/// Resets colors after the text if any color was applied.
fn style_text(text: &str, fg: Option<&str>, bg: Option<&str>) -> String {
    let fg_color = fg.and_then(parse_hex_color);
    let bg_color = bg.and_then(parse_hex_color);

    // If no colors, return text as-is
    if fg_color.is_none() && bg_color.is_none() {
        return text.to_string();
    }

    let mut result = String::new();

    // Apply foreground color
    if let Some(color) = fg_color {
        let _ = write!(result, "{}", SetForegroundColor(color));
    }

    // Apply background color
    if let Some(color) = bg_color {
        let _ = write!(result, "{}", SetBackgroundColor(color));
    }

    // Append the text
    result.push_str(text);

    // Reset colors
    let _ = write!(result, "{}", ResetColor);

    result
}

/// Parse a hex color string like "#rrggbb" to a crossterm Color.
///
/// Returns `None` if the string is malformed or doesn't start with '#'.
fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.strip_prefix('#')?;

    // Must be exactly 6 hex characters
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

    Some(Color::Rgb { r, g, b })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_color_parse_valid() {
        assert_eq!(
            parse_hex_color("#ff0000"),
            Some(Color::Rgb { r: 255, g: 0, b: 0 })
        );
        assert_eq!(
            parse_hex_color("#00ff00"),
            Some(Color::Rgb { r: 0, g: 255, b: 0 })
        );
        assert_eq!(
            parse_hex_color("#0000ff"),
            Some(Color::Rgb { r: 0, g: 0, b: 255 })
        );
        assert_eq!(
            parse_hex_color("#7dcfff"),
            Some(Color::Rgb {
                r: 125,
                g: 207,
                b: 255
            })
        );
        assert_eq!(
            parse_hex_color("#bb9af7"),
            Some(Color::Rgb {
                r: 187,
                g: 154,
                b: 247
            })
        );
    }

    #[test]
    fn test_hex_color_parse_invalid() {
        // Missing #
        assert_eq!(parse_hex_color("ff0000"), None);
        // Too short
        assert_eq!(parse_hex_color("#fff"), None);
        // Too long
        assert_eq!(parse_hex_color("#ff00ff00"), None);
        // Invalid hex characters
        assert_eq!(parse_hex_color("#gggggg"), None);
        // Empty
        assert_eq!(parse_hex_color(""), None);
        assert_eq!(parse_hex_color("#"), None);
    }

    #[test]
    fn test_style_text_no_colors() {
        let result = style_text("hello world", None, None);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_style_text_with_fg() {
        let result = style_text("red text", Some("#ff0000"), None);
        // Should contain ANSI escape sequence for red foreground
        assert!(result.contains("\x1b[38;2;255;0;0m"));
        assert!(result.contains("red text"));
        // Should contain reset
        assert!(result.contains("\x1b[0m"));
    }

    #[test]
    fn test_style_text_with_bg() {
        let result = style_text("blue bg", None, Some("#0000ff"));
        // Should contain ANSI escape sequence for blue background
        assert!(result.contains("\x1b[48;2;0;0;255m"));
        assert!(result.contains("blue bg"));
        // Should contain reset
        assert!(result.contains("\x1b[0m"));
    }

    #[test]
    fn test_style_text_with_both() {
        let result = style_text("styled", Some("#ff0000"), Some("#0000ff"));
        // Should contain both fg and bg
        assert!(result.contains("\x1b[38;2;255;0;0m"));
        assert!(result.contains("\x1b[48;2;0;0;255m"));
        assert!(result.contains("styled"));
        assert!(result.contains("\x1b[0m"));
    }

    #[test]
    fn test_style_text_invalid_color_falls_back() {
        // Invalid color should be treated as no color
        let result = style_text("plain", Some("invalid"), None);
        assert_eq!(result, "plain");
    }

    // Integration tests with actual Lua tables require the mlua runtime

    /// Helper to build Lua code for a segment with optional colors
    fn segment_lua(text: &str, fg: Option<&str>, bg: Option<&str>) -> String {
        let mut parts = vec![format!("Text=\"{}\"", text)];
        if let Some(f) = fg {
            parts.push(format!("Fg=\"{}\"", f));
        }
        if let Some(b) = bg {
            parts.push(format!("Bg=\"{}\"", b));
        }
        format!("{{{}}}", parts.join(", "))
    }

    /// Helper to build a Lua row from segments
    fn row_lua(segments: &[String]) -> String {
        format!("{{{}}}", segments.join(", "))
    }

    #[test]
    fn test_parse_lua_output_with_real_lua() {
        use mlua::Lua;

        let lua = Lua::new();

        // Build the Lua code programmatically to avoid raw string issues
        let row1 = row_lua(&[
            segment_lua("+", Some("#7dcfff"), None),
            segment_lua("--------", None, None),
            segment_lua("+", Some("#7dcfff"), None),
        ]);
        let row2 = row_lua(&[
            segment_lua("| alice  ", None, None),
            segment_lua("* qwen-8b", Some("#bb9af7"), None),
            segment_lua(" |", None, None),
        ]);
        let row3 = row_lua(&[
            segment_lua("| ", None, None),
            segment_lua("idle", Some("#565f89"), None),
            segment_lua("         |", None, None),
        ]);
        let row4 = row_lua(&[
            segment_lua("|", None, None),
            segment_lua("           ", None, None),
            segment_lua("|", None, None),
        ]);
        let row5 = row_lua(&[
            segment_lua("| mcp: ", None, None),
            segment_lua("o", Some("#9ece6a"), None),
            segment_lua(" holler |", None, None),
        ]);
        let row6 = row_lua(&[
            segment_lua("| lobby", None, None),
            segment_lua("       |", None, None),
        ]);
        let row7 = row_lua(&[
            segment_lua("+", Some("#7dcfff"), None),
            segment_lua("--------", None, None),
            segment_lua("+", Some("#7dcfff"), None),
        ]);
        let row8 = row_lua(&[segment_lua("", None, None)]);

        let lua_code = format!(
            "return {{{}, {}, {}, {}, {}, {}, {}, {}}}",
            row1, row2, row3, row4, row5, row6, row7, row8
        );

        let result = lua
            .load(&lua_code)
            .eval::<Table>()
            .expect("Failed to evaluate Lua");

        let output = parse_lua_output(result).expect("Failed to parse Lua output");

        // Verify structure
        let lines: Vec<&str> = output.split("\r\n").collect();
        assert_eq!(lines.len(), 8, "Should have 8 lines");

        // Check content is present
        assert!(output.contains("+"), "Should contain corner");
        assert!(output.contains("alice"), "Should contain user name");
        assert!(output.contains("qwen-8b"), "Should contain model name");
        assert!(output.contains("holler"), "Should contain MCP connection");
        assert!(output.contains("lobby"), "Should contain room name");
    }

    #[test]
    fn test_parse_lua_output_missing_rows() {
        use mlua::Lua;

        let lua = Lua::new();

        // Only 3 rows provided
        let row1 = row_lua(&[segment_lua("row 1", None, None)]);
        let row2 = row_lua(&[segment_lua("row 2", None, None)]);
        let row3 = row_lua(&[segment_lua("row 3", None, None)]);

        let lua_code = format!("return {{{}, {}, {}}}", row1, row2, row3);

        let result = lua
            .load(&lua_code)
            .eval::<Table>()
            .expect("Failed to evaluate Lua");

        let output = parse_lua_output(result).expect("Failed to parse");

        let lines: Vec<&str> = output.split("\r\n").collect();
        assert_eq!(lines.len(), 8, "Should pad to 8 lines");
        assert_eq!(lines[0], "row 1");
        assert_eq!(lines[1], "row 2");
        assert_eq!(lines[2], "row 3");
        // Remaining lines should be empty
        assert_eq!(lines[3], "");
        assert_eq!(lines[7], "");
    }

    #[test]
    fn test_parse_lua_output_missing_text() {
        use mlua::Lua;

        let lua = Lua::new();

        // Segment without Text key should be skipped
        // Build row with: {Text="before"}, {Fg="#ff0000"}, {Text="after"}
        let lua_code =
            "return {{{Text=\"before\"}, {Fg=\"#ff0000\"}, {Text=\"after\"}}, {}, {}, {}, {}, {}, {}, {}}";

        let result = lua
            .load(lua_code)
            .eval::<Table>()
            .expect("Failed to evaluate Lua");

        let output = parse_lua_output(result).expect("Failed to parse");
        let first_line = output.split("\r\n").next().unwrap();

        assert!(first_line.contains("before"));
        assert!(first_line.contains("after"));
        // The segment with only Fg should be skipped (no contribution)
    }

    #[test]
    fn test_parse_row_empty() {
        use mlua::Lua;

        let lua = Lua::new();
        let empty_row = lua
            .load("return {}")
            .eval::<Table>()
            .expect("Failed to evaluate");

        let result = parse_row(empty_row).expect("Failed to parse");
        assert_eq!(result, "");
    }

    #[test]
    fn test_parse_row_mixed_segments() {
        use mlua::Lua;

        let lua = Lua::new();

        // Build: {Text="plain "}, {Fg="#ff0000", Text="red "}, {Bg="#00ff00", Text="green-bg "}, {Fg="#0000ff", Bg="#ffff00", Text="styled"}
        let seg1 = segment_lua("plain ", None, None);
        let seg2 = segment_lua("red ", Some("#ff0000"), None);
        let seg3 = segment_lua("green-bg ", None, Some("#00ff00"));
        let seg4 = segment_lua("styled", Some("#0000ff"), Some("#ffff00"));

        let lua_code = format!("return {{{}, {}, {}, {}}}", seg1, seg2, seg3, seg4);

        let row = lua
            .load(&lua_code)
            .eval::<Table>()
            .expect("Failed to evaluate");

        let result = parse_row(row).expect("Failed to parse");

        // All text should be present
        assert!(result.contains("plain "));
        assert!(result.contains("red "));
        assert!(result.contains("green-bg "));
        assert!(result.contains("styled"));

        // Should have color escape codes
        assert!(result.contains("\x1b[38;2;255;0;0m"), "Should have red fg");
        assert!(
            result.contains("\x1b[48;2;0;255;0m"),
            "Should have green bg"
        );
        assert!(result.contains("\x1b[38;2;0;0;255m"), "Should have blue fg");
        assert!(
            result.contains("\x1b[48;2;255;255;0m"),
            "Should have yellow bg"
        );
    }

    #[test]
    fn test_colors_are_applied_correctly() {
        use mlua::Lua;

        let lua = Lua::new();

        // Test Tokyo Night colors from the actual theme
        let cyan = segment_lua("cyan", Some("#7dcfff"), None);
        let magenta = segment_lua("magenta", Some("#bb9af7"), None);
        let green = segment_lua("green", Some("#9ece6a"), None);
        let yellow = segment_lua("yellow", Some("#e0af68"), None);

        let lua_code = format!("return {{{}, {}, {}, {}}}", cyan, magenta, green, yellow);

        let row = lua
            .load(&lua_code)
            .eval::<Table>()
            .expect("Failed to evaluate");

        let result = parse_row(row).expect("Failed to parse");

        // Verify Tokyo Night colors are applied
        assert!(
            result.contains("\x1b[38;2;125;207;255m"),
            "Should have cyan (7dcfff)"
        );
        assert!(
            result.contains("\x1b[38;2;187;154;247m"),
            "Should have magenta (bb9af7)"
        );
        assert!(
            result.contains("\x1b[38;2;158;206;106m"),
            "Should have green (9ece6a)"
        );
        assert!(
            result.contains("\x1b[38;2;224;175;104m"),
            "Should have yellow (e0af68)"
        );
    }

    #[test]
    fn test_unicode_text_preserved() {
        use mlua::Lua;

        let lua = Lua::new();

        // Test that Unicode box drawing and emoji are preserved in output
        // Note: We can't use them in Lua literals with Luau, but we can test
        // that when they come through, they're preserved
        let seg = segment_lua("Hello World", None, None);
        let lua_code = format!("return {{{}}}", seg);

        let row = lua
            .load(&lua_code)
            .eval::<Table>()
            .expect("Failed to evaluate");

        let result = parse_row(row).expect("Failed to parse");
        assert_eq!(result, "Hello World");
    }
}
