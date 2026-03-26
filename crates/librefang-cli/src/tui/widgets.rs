//! Shared TUI widget helpers — single source of truth for visual patterns.
//!
//! Every screen should compose these helpers rather than hand-rolling blocks,
//! spinners, hint bars, etc.  Changing a helper here updates all screens at once.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Padding, Paragraph};
use ratatui::Frame;

use super::theme;

// ── Screen frame ──────────────────────────────────────────────────────────

/// Standard bordered screen block with accent-colored border and title.
/// Used as the outer frame for almost every tab screen.
pub fn screen_block(title: &str) -> Block<'_> {
    Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" {title} "),
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1))
}

/// Inner card block with dimmed border — for dashboard stat cards and similar.
pub fn card_block(title: &str) -> Block<'_> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme::CYAN),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::DIM))
}

/// Render a screen_block, return the inner area.
pub fn render_screen_block(f: &mut Frame, area: Rect, title: &str) -> Rect {
    let block = screen_block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

/// Render a card_block, return the inner area.
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

/// Loading spinner with message: `  ⣷ Loading…`
pub fn spinner(tick: usize, message: &str) -> Paragraph<'_> {
    let frame = theme::SPINNER_FRAMES[tick % theme::SPINNER_FRAMES.len()];
    Paragraph::new(Line::from(vec![
        Span::styled(format!("  {frame} "), Style::default().fg(theme::CYAN)),
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

/// "No X found." placeholder for empty lists.
pub fn empty_state(message: &str) -> Paragraph<'_> {
    Paragraph::new(Span::styled(format!("  {message}"), theme::dim_style()))
}

// ── Themed list ───────────────────────────────────────────────────────────

/// Standard highlighted list with `> ` selection marker.
pub fn themed_list(items: Vec<ListItem<'_>>) -> List<'_> {
    List::new(items)
        .highlight_style(theme::selected_style())
        .highlight_symbol("> ")
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

/// Centered bottom toast notification.
pub fn render_toast(f: &mut Frame, area: Rect, msg: &str, color: ratatui::style::Color) {
    let w = (msg.len() as u16 + 4).min(area.width);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(2);
    let toast_area = Rect::new(x, y, w, 1);
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
