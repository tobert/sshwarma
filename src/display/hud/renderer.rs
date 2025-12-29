//! HUD renderer
//!
//! Renders the HUD state to ANSI-styled strings for terminal output.

use crossterm::style::Stylize;

use super::spinner::spinner_char;
use super::state::{HudState, ParticipantKind, ParticipantStatus, Presence};
use crate::display::styles::{colors, ctrl, HudBox};

/// HUD dimensions
pub const HUD_HEIGHT: u16 = 8; // Including borders
pub const HUD_CONTENT_LINES: usize = 6; // Lines inside the box

/// Render the complete HUD (8 lines including borders)
///
/// Returns a string with ANSI escape codes. Does NOT include trailing CRLF
/// on the last line (the input line).
pub fn render_hud(state: &HudState, width: u16) -> String {
    let mut output = String::new();
    let inner_width = width.saturating_sub(2) as usize;

    // Line 1: Top border
    output.push_str(&render_top_border(inner_width));
    output.push_str(ctrl::CRLF);

    // Line 2: Participants (users + models with glyphs)
    output.push_str(&render_participants_row(state, inner_width));
    output.push_str(ctrl::CRLF);

    // Line 3: Status row (status text for active participants)
    output.push_str(&render_status_row(state, inner_width));
    output.push_str(ctrl::CRLF);

    // Line 4: Reserved / empty
    output.push_str(&render_empty_row(inner_width));
    output.push_str(ctrl::CRLF);

    // Line 5: MCP connections
    output.push_str(&render_mcp_row(state, inner_width));
    output.push_str(ctrl::CRLF);

    // Line 6: Room info
    output.push_str(&render_room_row(state, inner_width));
    output.push_str(ctrl::CRLF);

    // Line 7: Bottom border (with notification if present)
    output.push_str(&render_bottom_border(state, inner_width));
    output.push_str(ctrl::CRLF);

    // Line 8: Input line (bare, just clear it)
    output.push_str(&ctrl::clear_line());

    output
}

/// Render top border
fn render_top_border(inner_width: usize) -> String {
    let border = HudBox::HORIZONTAL.repeat(inner_width);
    format!(
        "{}{}{}",
        HudBox::TOP_LEFT.with(colors::CYAN),
        border.with(colors::CYAN),
        HudBox::TOP_RIGHT.with(colors::CYAN)
    )
}

/// Render bottom border, with optional right-aligned notification
fn render_bottom_border(state: &HudState, inner_width: usize) -> String {
    if let Some(ref notif) = state.notification {
        // Right-aligned notification in border
        let notif_text = format!(" ⚡ {} ", &notif.message);
        let notif_display_len = visible_len(&notif_text);

        // Ensure we keep at least some border visible
        let min_border = 4;
        if notif_display_len + min_border < inner_width {
            let left_border_len = inner_width - notif_display_len - 2; // -2 for trailing border chars
            let left_border = HudBox::HORIZONTAL.repeat(left_border_len);
            let right_border = HudBox::HORIZONTAL.repeat(2);

            return format!(
                "{}{}{}{}{}",
                HudBox::BOTTOM_LEFT.with(colors::CYAN),
                left_border.with(colors::CYAN),
                notif_text.with(colors::YELLOW),
                right_border.with(colors::CYAN),
                HudBox::BOTTOM_RIGHT.with(colors::CYAN)
            );
        }
    }

    // No notification - plain border
    let border = HudBox::HORIZONTAL.repeat(inner_width);
    format!(
        "{}{}{}",
        HudBox::BOTTOM_LEFT.with(colors::CYAN),
        border.with(colors::CYAN),
        HudBox::BOTTOM_RIGHT.with(colors::CYAN)
    )
}

/// Render participants row (users and models mixed)
fn render_participants_row(state: &HudState, inner_width: usize) -> String {
    let mut content = String::new();
    content.push_str("  "); // Left padding

    for (i, p) in state.participants.iter().enumerate() {
        if i > 0 {
            content.push_str("  ");
        }
        content.push_str(&format_participant(p, state.spinner_frame));
    }

    pad_row(content, inner_width)
}

/// Format a single participant for display
fn format_participant(p: &Presence, spinner_frame: u8) -> String {
    match p.kind {
        ParticipantKind::User => {
            // Users: just name, with optional status glyph if non-idle
            match &p.status {
                ParticipantStatus::Idle => p.name.clone(),
                ParticipantStatus::Emoji(e) => format!("{} {}", e, p.name),
                _ => format!("{} {}", p.status.glyph(), p.name),
            }
        }
        ParticipantKind::Model => {
            // Models: always show glyph (spinner if active)
            let glyph = if p.status.is_active() {
                spinner_char(spinner_frame).to_string()
            } else {
                p.status.glyph().to_string()
            };

            let styled_glyph = match &p.status {
                ParticipantStatus::Idle => glyph.with(colors::DIM).to_string(),
                ParticipantStatus::Thinking | ParticipantStatus::RunningTool(_) => {
                    glyph.with(colors::CYAN).to_string()
                }
                ParticipantStatus::Error(_) => glyph.with(colors::RED).to_string(),
                ParticipantStatus::Offline => glyph.with(colors::DIM).to_string(),
                ParticipantStatus::Emoji(_) => glyph,
            };

            format!("{} {}", styled_glyph, p.name.as_str().with(colors::MAGENTA))
        }
    }
}

/// Render status row (status text for participants that have one)
fn render_status_row(state: &HudState, inner_width: usize) -> String {
    let mut content = String::new();
    content.push_str("  "); // Match participant row padding

    for (i, p) in state.participants.iter().enumerate() {
        if i > 0 {
            content.push_str("  ");
        }

        let status_text = p.status.text();
        let name_width = visible_len(&p.name) + if p.is_model() { 2 } else { 0 }; // +2 for glyph + space

        if status_text.is_empty() {
            // Pad to align with name above
            content.push_str(&" ".repeat(name_width));
        } else {
            // Show status, padded to align
            let status_visible = visible_len(&status_text);
            let styled = status_text.as_str().with(colors::DIM).to_string();
            if status_visible < name_width {
                content.push_str(&styled);
                content.push_str(&" ".repeat(name_width - status_visible));
            } else {
                content.push_str(&styled);
            }
        }
    }

    pad_row(content, inner_width)
}

/// Render MCP connections row
fn render_mcp_row(state: &HudState, inner_width: usize) -> String {
    let mut content = String::new();
    content.push_str("  ");

    if state.mcp_connections.is_empty() {
        content.push_str(&"no MCP connections".with(colors::DIM).to_string());
    } else {
        content.push_str(&"mcp: ".with(colors::DIM).to_string());
        for (i, conn) in state.mcp_connections.iter().enumerate() {
            if i > 0 {
                content.push_str("  ");
            }
            let indicator = if conn.connected {
                "●".with(colors::GREEN).to_string()
            } else {
                "○".with(colors::RED).to_string()
            };

            // Format: ● name (tools/calls) [last_tool]
            let stats = format!("{}/{}", conn.tool_count, conn.call_count);
            content.push_str(&format!("{} {} ", indicator, conn.name));
            content.push_str(&format!("({})", stats).with(colors::DIM).to_string());

            if let Some(ref last) = conn.last_tool {
                content.push_str(" ");
                content.push_str(&last.as_str().with(colors::CYAN).to_string());
            }
        }
    }

    pad_row(content, inner_width)
}

/// Tick indicator characters (cycles every 100ms)
const TICK_CHARS: [char; 4] = ['·', ':', '·', ' '];

/// Render room info row
fn render_room_row(state: &HudState, inner_width: usize) -> String {
    let room_name = state.room_name.as_deref().unwrap_or("lobby");
    let exits = state.exit_arrows();
    let duration = state.duration_string();

    // Tick indicator shows refresh is working
    let tick = TICK_CHARS[(state.spinner_frame as usize / 2) % TICK_CHARS.len()];

    // Build room info with separators
    let mut parts = vec![format!("  {}", room_name.with(colors::CYAN))];

    if !exits.is_empty() {
        parts.push(format!("│ {}", exits));
    }

    parts.push(format!("│ {} {}", duration.with(colors::DIM), tick.to_string().with(colors::DIM)));

    let content = parts.join(" ");
    pad_row(content, inner_width)
}

/// Render an empty row
fn render_empty_row(inner_width: usize) -> String {
    pad_row(String::new(), inner_width)
}

/// Pad content to fill row width, wrapped with vertical borders
fn pad_row(content: String, inner_width: usize) -> String {
    let content_visible_len = visible_len(&content);
    let padding = inner_width.saturating_sub(content_visible_len);

    format!(
        "{}{}{}{}",
        HudBox::VERTICAL.with(colors::CYAN),
        content,
        " ".repeat(padding),
        HudBox::VERTICAL.with(colors::CYAN)
    )
}

/// Approximate visible length of a string (strips ANSI escape codes)
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;

    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            // Count visible characters
            // Note: This is a simplification. For proper Unicode width,
            // we'd need unicode-width crate
            len += 1;
        }
    }

    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_len() {
        assert_eq!(visible_len("hello"), 5);
        assert_eq!(visible_len("\x1b[31mred\x1b[0m"), 3);
        assert_eq!(visible_len("◇ qwen"), 6); // glyph counts as 1
    }

    #[test]
    fn test_render_empty_hud() {
        let state = HudState::new();
        let output = render_hud(&state, 80);
        assert!(!output.is_empty());
        assert!(output.contains(HudBox::TOP_LEFT));
        assert!(output.contains(HudBox::BOTTOM_LEFT));
    }

    #[test]
    fn test_render_with_participants() {
        let mut state = HudState::new();
        state.add_user("alice".to_string());
        state.add_model("qwen-8b".to_string());

        let output = render_hud(&state, 80);
        assert!(output.contains("alice"));
        assert!(output.contains("qwen-8b"));
    }

    #[test]
    fn test_render_with_notification() {
        let mut state = HudState::new();
        state.notify("bob joined".to_string(), 5);

        let output = render_hud(&state, 80);
        assert!(output.contains("bob joined"));
        assert!(output.contains("⚡"));
    }
}
