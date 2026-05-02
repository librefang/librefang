//! Shared TUI widget helpers — single source of truth for visual patterns.
//!
//! Every screen should compose these helpers rather than hand-rolling blocks,
//! spinners, hint bars, etc.  Changing a helper here updates all screens at once.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Padding, Paragraph};
use ratatui::Frame;

use super::theme;

// ── Screen frame ──────────────────────────────────────────────────────────

/// Standard bordered screen block with rounded corners and accent title.
/// Used as the outer frame for almost every tab screen.
pub fn screen_block(title: &str) -> Block<'_> {
    Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" {title} "),
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1))
}

/// Inner card block with rounded corners — for stat cards and similar.
#[allow(dead_code)]
pub fn card_block(title: &str) -> Block<'_> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme::CYAN),
        ))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(theme::BORDER))
}

/// Render a screen_block, return the inner area.
pub fn render_screen_block(f: &mut Frame, area: Rect, title: &str) -> Rect {
    let block = screen_block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

/// Render a card_block, return the inner area.
#[allow(dead_code)]
pub fn render_card_block(f: &mut Frame, area: Rect, title: &str) -> Rect {
    let block = card_block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

// ── Standard layouts ──────────────────────────────────────────────────────

/// Most common layout: header (N rows) + scrollable content + 1-row hint bar.
pub fn layout_hch(inner: Rect, header_h: u16) -> (Rect, Rect, Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(inner);
    (chunks[0], chunks[1], chunks[2])
}

// ── Spinner ───────────────────────────────────────────────────────────────

/// Loading spinner with message: `  ◜ Loading…`
pub fn spinner(tick: usize, message: &str) -> Paragraph<'_> {
    let frame = theme::SPINNER_FRAMES[tick % theme::SPINNER_FRAMES.len()];
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("  {frame} "),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(message, theme::dim_style()),
    ]))
}

// ── Hint bar ──────────────────────────────────────────────────────────────

/// Bottom hint bar: dim text showing available keybindings.
pub fn hint_bar(text: &str) -> Paragraph<'_> {
    Paragraph::new(Line::from(vec![Span::styled(text, theme::hint_style())]))
}

// ── Status / confirm / hint combo ─────────────────────────────────────────

/// Three-way bottom bar: confirm prompt (yellow) → status (green) → hint (dim).
pub fn confirm_or_status_or_hint<'a>(
    confirming: bool,
    confirm_msg: &'a str,
    status_msg: &'a str,
    hint_text: &'a str,
) -> Paragraph<'a> {
    if confirming {
        Paragraph::new(Line::from(vec![Span::styled(
            confirm_msg,
            Style::default().fg(theme::YELLOW),
        )]))
    } else if !status_msg.is_empty() {
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {status_msg}"),
            Style::default().fg(theme::GREEN),
        )]))
    } else {
        hint_bar(hint_text)
    }
}

/// Two-way bottom bar: status (green) → hint (dim).
pub fn status_or_hint<'a>(status_msg: &'a str, hint_text: &'a str) -> Paragraph<'a> {
    confirm_or_status_or_hint(false, "", status_msg, hint_text)
}

// ── Empty state ───────────────────────────────────────────────────────────

/// "No X found." placeholder for empty lists — centered with dimmed icon.
pub fn empty_state(message: &str) -> Paragraph<'_> {
    Paragraph::new(Line::from(vec![
        Span::styled("  \u{2500}\u{2500} ", Style::default().fg(theme::BORDER)),
        Span::styled(message, theme::dim_style()),
        Span::styled(" \u{2500}\u{2500}", Style::default().fg(theme::BORDER)),
    ]))
}

// ── Themed list ───────────────────────────────────────────────────────────

/// Standard highlighted list with `▸ ` selection marker.
pub fn themed_list(items: Vec<ListItem<'_>>) -> List<'_> {
    List::new(items)
        .highlight_style(theme::selected_style())
        .highlight_symbol("\u{25b8} ")
}

// ── Search input ──────────────────────────────────────────────────────────

/// Search mode input line: `  / query█`
pub fn search_input(query: &str) -> Paragraph<'_> {
    Paragraph::new(Line::from(vec![
        Span::styled("  / ", Style::default().fg(theme::ACCENT)),
        Span::styled(query, theme::input_style()),
        Span::styled(
            "\u{2588}",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]))
}

// ── Separator ─────────────────────────────────────────────────────────────

/// Horizontal rule (─) spanning `width` characters.
pub fn separator(width: u16) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "\u{2500}".repeat(width as usize),
        Style::default().fg(theme::BORDER),
    )))
}

// ── Toast ─────────────────────────────────────────────────────────────────

/// Compute the centered-bottom rectangle a toast of `msg` would occupy in
/// `area`. Width is `msg.len() + 4` clamped to `area.width`, x is centered,
/// y sits one row above the bottom edge (saturating for very small areas).
/// Extracted for testability — `render_toast` delegates the math here.
pub fn toast_rect(area: Rect, msg: &str) -> Rect {
    let w = (msg.len() as u16 + 4).min(area.width);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(2);
    Rect::new(x, y, w, 1)
}

/// Centered bottom toast notification.
pub fn render_toast(f: &mut Frame, area: Rect, msg: &str, color: ratatui::style::Color) {
    let toast_area = toast_rect(area, msg);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(color)))),
        toast_area,
    );
}

// ── String helpers ────────────────────────────────────────────────────────

/// Truncate a string to `max` characters, appending `…` if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}\u{2026}",
            librefang_types::truncate_str(s, max.saturating_sub(1))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── truncate ──────────────────────────────────────────────────────────

    #[test]
    fn truncate_passthrough_when_within_max() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_appends_ellipsis_when_too_long() {
        // "hello world" is 11 bytes; max=8 → keep 7 bytes + ellipsis.
        let out = truncate("hello world", 8);
        assert!(out.ends_with('\u{2026}'));
        assert_eq!(out, "hello w\u{2026}");
    }

    #[test]
    fn truncate_never_splits_utf8_boundary() {
        // "café" is 5 bytes (c,a,f,0xC3,0xA9); asking for max=5 must NOT
        // truncate (it fits) — and asking for max=4 must back off cleanly.
        assert_eq!(truncate("café", 5), "café");
        let out = truncate("café世界", 6);
        // Must be valid UTF-8 and end with the ellipsis.
        assert!(out.ends_with('\u{2026}'));
        // And must not have produced an invalid sequence (String guarantees
        // valid UTF-8, so reaching here without panic is the assertion).
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn truncate_max_zero_yields_just_ellipsis() {
        // saturating_sub(1) on 0 is still 0 → empty prefix + ellipsis.
        let out = truncate("hello", 0);
        assert_eq!(out, "\u{2026}");
    }

    // ── layout_hch ────────────────────────────────────────────────────────

    #[test]
    fn layout_hch_splits_header_content_hint() {
        let area = Rect::new(0, 0, 80, 24);
        let (header, content, hint) = layout_hch(area, 3);
        assert_eq!(header.height, 3);
        assert_eq!(hint.height, 1);
        // Three regions are vertically contiguous and fully cover `area`.
        assert_eq!(header.y, 0);
        assert_eq!(content.y, header.y + header.height);
        assert_eq!(hint.y, content.y + content.height);
        assert_eq!(hint.y + hint.height, area.y + area.height);
        // All regions share the same width / x as the parent.
        for r in [header, content, hint] {
            assert_eq!(r.x, area.x);
            assert_eq!(r.width, area.width);
        }
    }

    #[test]
    fn layout_hch_respects_min_content_height() {
        // header_h=3 + min_content=3 + hint=1 = 7 → content gets the rest.
        let area = Rect::new(0, 0, 40, 20);
        let (_h, content, _hint) = layout_hch(area, 3);
        assert!(content.height >= 3);
    }

    // ── toast_rect ────────────────────────────────────────────────────────

    #[test]
    fn toast_rect_centers_horizontally_and_anchors_above_bottom() {
        let area = Rect::new(0, 0, 80, 24);
        let msg = "saved";
        let rect = toast_rect(area, msg);
        assert_eq!(rect.height, 1);
        assert_eq!(rect.width, msg.len() as u16 + 4);
        assert_eq!(rect.y, 24 - 2);
        // Centered: equal padding on both sides (within 1 px for parity).
        let right_pad = area.width - rect.x - rect.width;
        assert!(rect.x.abs_diff(right_pad) <= 1);
    }

    #[test]
    fn toast_rect_clamps_width_to_area_when_msg_overflows() {
        let area = Rect::new(0, 0, 10, 5);
        // 20-byte message, way wider than area.width=10.
        let rect = toast_rect(area, "this-is-a-long-toast");
        assert_eq!(rect.width, area.width);
        assert_eq!(rect.x, 0);
    }

    #[test]
    fn toast_rect_y_saturates_for_tiny_areas() {
        // height=1 → saturating_sub(2) = 0, must not panic / underflow.
        let area = Rect::new(0, 0, 20, 1);
        let rect = toast_rect(area, "hi");
        assert_eq!(rect.y, 0);
        assert_eq!(rect.height, 1);
    }
}
