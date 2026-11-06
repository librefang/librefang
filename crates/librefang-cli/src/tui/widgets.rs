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
    confirm_or_status_or_hint_styled(
        confirming,
        confirm_msg,
        status_msg,
        Style::default().fg(theme::GREEN),
        hint_text,
    )
}

/// [`confirm_or_status_or_hint`] with an explicit status color.
///
/// Screens whose status line carries failures and refusals as well as successes
/// keep it yellow, so routing it through the combo widget does not repaint a
/// failure as a success.
pub fn confirm_or_status_or_hint_styled<'a>(
    confirming: bool,
    confirm_msg: &'a str,
    status_msg: &'a str,
    status_style: Style,
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
            status_style,
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

// ── Checkbox list ─────────────────────────────────────────────────────────

/// A cursor-driven list of named checkboxes.
///
/// One source of truth for the selectable-set screens: cursor movement saturates at both ends, `Space` toggles only the row under the cursor, and the checked names keep the list's order.
pub struct CheckboxList {
    pub items: Vec<(String, bool)>,
    pub cursor: usize,
}

impl CheckboxList {
    pub fn new(items: Vec<(String, bool)>) -> Self {
        Self { items, cursor: 0 }
    }

    /// Build a list from `names`, checking the ones `selected` accepts.
    pub fn from_names(
        names: impl IntoIterator<Item = String>,
        selected: impl Fn(&str) -> bool,
    ) -> Self {
        Self::new(
            names
                .into_iter()
                .map(|name| {
                    let checked = selected(&name);
                    (name, checked)
                })
                .collect(),
        )
    }

    /// Flip the checkbox under the cursor; a no-op on an empty list.
    pub fn toggle(&mut self) {
        if let Some((_, checked)) = self.items.get_mut(self.cursor) {
            *checked = !*checked;
        }
    }

    /// Move the cursor up, stopping at the first row.
    pub fn up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move the cursor down, stopping at the last row.
    pub fn down(&mut self) {
        if self.cursor + 1 < self.len() {
            self.cursor += 1;
        }
    }

    /// The checked names, in item order.
    pub fn selected_names(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|(_, checked)| *checked)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// How many items are checked.
    pub fn selected_count(&self) -> usize {
        self.items.iter().filter(|(_, checked)| *checked).count()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Prompt line, checkbox rows or the shared empty state, and a hint bar.
    pub fn render(&self, f: &mut Frame, area: Rect, prompt: &str, hints: &str) {
        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

        f.render_widget(Paragraph::new(format!("  {prompt}")), chunks[0]);

        if self.items.is_empty() {
            f.render_widget(
                empty_state(&crate::i18n::t("tui-agents-label-none-available")),
                chunks[1],
            );
        } else {
            let list_items: Vec<ListItem> = self
                .items
                .iter()
                .enumerate()
                .map(|(i, (name, checked))| {
                    let check = if *checked { "\u{25c9}" } else { "\u{25cb}" };
                    let highlight = if i == self.cursor {
                        Style::default()
                            .fg(theme::CYAN)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(vec![Span::styled(
                        format!("  {check} {name}"),
                        highlight,
                    )]))
                })
                .collect();

            f.render_widget(List::new(list_items), chunks[1]);
        }

        f.render_widget(hint_bar(hints), chunks[2]);
    }
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

    // ── CheckboxList ──────────────────────────────────────────────────────

    fn three_items() -> CheckboxList {
        CheckboxList::new(vec![
            ("alpha".to_string(), false),
            ("beta".to_string(), false),
            ("gamma".to_string(), false),
        ])
    }

    #[test]
    fn checkbox_list_toggle_flips_only_the_cursor_item() {
        let mut list = three_items();
        list.toggle();
        list.down();
        list.toggle();
        assert_eq!(
            list.items,
            vec![
                ("alpha".to_string(), true),
                ("beta".to_string(), true),
                ("gamma".to_string(), false),
            ]
        );
        assert_eq!(list.selected_count(), 2);
    }

    #[test]
    fn checkbox_list_cursor_saturates_at_both_ends() {
        let mut list = three_items();
        list.up();
        assert_eq!(list.cursor, 0);
        for _ in 0..10 {
            list.down();
        }
        assert_eq!(list.cursor, 2);
        list.up();
        assert_eq!(list.cursor, 1);
    }

    #[test]
    fn checkbox_list_selected_names_keep_item_order() {
        let mut list = three_items();
        list.items[2].1 = true;
        list.items[0].1 = true;
        assert_eq!(list.selected_names(), vec!["alpha", "gamma"]);
    }

    #[test]
    fn checkbox_list_from_names_seeds_from_the_predicate() {
        let list =
            CheckboxList::from_names(vec!["alpha".to_string(), "beta".to_string()], |name| {
                name.starts_with('b')
            });
        assert_eq!(
            list.items,
            vec![("alpha".to_string(), false), ("beta".to_string(), true),]
        );
    }

    #[test]
    fn checkbox_list_empty_tolerates_navigation_and_toggle() {
        let mut list = CheckboxList::new(Vec::new());
        list.up();
        list.down();
        list.toggle();
        assert_eq!(list.cursor, 0);
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert!(list.selected_names().is_empty());
        assert_eq!(list.selected_count(), 0);
    }
}
