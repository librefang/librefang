//! Groups screen: the configured user groups and who is on them (#7745).
//!
//! Read-only, like the peers screen it is modelled on.
//! Group writes are Owner-gated on the daemon and the terminal path for them is `librefang group create / add-member / remove-member`, so making this screen mutate would duplicate that surface without adding anything the operator cannot already do — and the TUI has no credential prompt to fall back on when the gate rejects the call.
//!
//! There is no expand-into-children affordance because groups do not nest; a group's membership list is the whole of what it denotes.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct GroupInfo {
    pub name: String,
    pub description: String,
    pub member_count: u64,
    /// Already comma-joined by the fetcher — the row renders it as one cell and
    /// keeping it as a `Vec` would mean re-joining on every frame.
    pub roles: String,
    /// True when at least one member has no `[[users]]` entry. Not an error
    /// condition; it is how an identity-provider sync looks before the person
    /// first signs in, and it is worth showing rather than hiding.
    pub has_unregistered_members: bool,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct GroupsState {
    pub groups: Vec<GroupInfo>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub poll_tick: usize,
}

pub enum GroupsAction {
    Continue,
    Refresh,
}

impl GroupsState {
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            tick: 0,
            poll_tick: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.poll_tick = self.poll_tick.wrapping_add(1);
    }

    /// Auto-refresh cadence. Groups change when an operator edits them, which
    /// is far rarer than peer state churn, so this polls every ~30s at the 20fps
    /// tick rate rather than the 15s the peers screen uses.
    pub fn should_poll(&self) -> bool {
        self.poll_tick > 0 && self.poll_tick.is_multiple_of(600)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> GroupsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return GroupsAction::Continue;
        }
        let total = self.groups.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.list_state.select(Some(next));
            }
            KeyCode::Char('r') => return GroupsAction::Refresh,
            _ => {}
        }
        GroupsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut GroupsState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("◌ {}", crate::i18n::t("tui-groups-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![Span::styled(
                format!(
                    "  {}",
                    crate::i18n::t_args(
                        "tui-groups-count",
                        &[("count", &state.groups.len().to_string())]
                    )
                ),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  ", theme::table_header()),
                Span::styled(
                    format!("{:<20}", crate::i18n::t("tui-groups-header-name")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    format!("{:<8}", crate::i18n::t("tui-groups-header-members")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    format!("{:<22}", crate::i18n::t("tui-groups-header-roles")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    crate::i18n::t("tui-groups-header-description"),
                    theme::table_header(),
                ),
            ]),
        ]),
        chunks[0],
    );

    if state.loading && state.groups.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-groups-loading")),
            chunks[1],
        );
    } else if state.groups.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-groups-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .groups
            .iter()
            .map(|g| {
                let member_cell = if g.has_unregistered_members {
                    // A trailing marker rather than a separate column: the fact
                    // is a footnote on the member count, not a dimension of its
                    // own, and the row is already four columns wide.
                    format!("{}*", g.member_count)
                } else {
                    g.member_count.to_string()
                };
                let member_style = if g.has_unregistered_members {
                    Style::default().fg(theme::YELLOW)
                } else {
                    Style::default().fg(theme::GREEN)
                };
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        format!("{:<20}", widgets::truncate(&g.name, 19)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(format!("{member_cell:<8}"), member_style),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{:<22}", widgets::truncate(&g.roles, 21)),
                        Style::default().fg(theme::PURPLE),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        widgets::truncate(&g.description, 40),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    f.render_widget(
        widgets::hint_bar(&format!("  {}", crate::i18n::t("tui-groups-hints"))),
        chunks[2],
    );
}
