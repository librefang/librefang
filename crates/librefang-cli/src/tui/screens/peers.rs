//! Peers screen: OFP peer network status with auto-refresh.

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
pub struct PeerInfo {
    pub node_id: String,
    pub node_name: String,
    pub address: String,
    pub state: String,
    pub agent_count: u64,
    pub protocol_version: String,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct PeersState {
    pub peers: Vec<PeerInfo>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub poll_tick: usize,
}

pub enum PeersAction {
    Continue,
    Refresh,
}

impl PeersState {
    pub fn new() -> Self {
        Self {
            peers: Vec::new(),
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

    /// Returns true if it's time to auto-refresh (every ~15s at 20fps tick rate).
    pub fn should_poll(&self) -> bool {
        self.poll_tick > 0 && self.poll_tick.is_multiple_of(300)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PeersAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return PeersAction::Continue;
        }
        let total = self.peers.len();
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
            KeyCode::Char('r') => return PeersAction::Refresh,
            _ => {}
        }
        PeersAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut PeersState) {
    let inner =
        widgets::render_screen_block(f, area, &format!("◌ {}", crate::i18n::t("tui-peers-title")));

    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // Header
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!("  {}", crate::i18n::t("tui-peers-network")),
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  │ {}",
                        crate::i18n::t_args(
                            "tui-peers-count",
                            &[("count", &state.peers.len().to_string())]
                        )
                    ),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ]),
            Line::from(vec![
                Span::styled("  ", theme::table_header()),
                Span::styled(
                    format!("{:<14}", crate::i18n::t("tui-peers-header-node-id")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    format!("{:<16}", crate::i18n::t("tui-peers-header-name")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    format!("{:<20}", crate::i18n::t("tui-peers-header-address")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    format!("{:<10}", crate::i18n::t("tui-peers-header-status")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    format!("{:<6}", crate::i18n::t("tui-peers-header-agents")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    crate::i18n::t("tui-peers-header-protocol"),
                    theme::table_header(),
                ),
            ]),
        ]),
        chunks[0],
    );

    // List
    if state.loading && state.peers.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-peers-loading")),
            chunks[1],
        );
    } else if state.peers.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-peers-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .peers
            .iter()
            .map(|p| {
                let id_short = if p.node_id.len() > 12 {
                    format!("{}…", librefang_types::truncate_str(&p.node_id, 12))
                } else {
                    p.node_id.clone()
                };
                let (state_badge, state_style) = match p.state.to_lowercase().as_str() {
                    "connected" | "active" => (
                        format!("● {}", crate::i18n::t("tui-peers-status-active")),
                        Style::default().fg(theme::GREEN),
                    ),
                    "disconnected" | "inactive" => (
                        format!("○ {}", crate::i18n::t("tui-peers-status-offline")),
                        theme::dim_style(),
                    ),
                    "connecting" | "pending" => (
                        format!("○ {}", crate::i18n::t("tui-peers-status-pending")),
                        Style::default().fg(theme::YELLOW),
                    ),
                    _ => (p.state.clone(), theme::dim_style()),
                };
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        format!("{:<14}", id_short),
                        Style::default().fg(theme::PURPLE),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{:<16}", widgets::truncate(&p.node_name, 15)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{:<20}", widgets::truncate(&p.address, 19)),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(format!("{:<10}", state_badge), state_style),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{:<6}", p.agent_count),
                        Style::default().fg(theme::GREEN),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        p.protocol_version.clone(),
                        Style::default().fg(theme::TEXT_TERTIARY),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    // Hints
    f.render_widget(
        widgets::hint_bar(&format!("  {}", crate::i18n::t("tui-peers-hints"))),
        chunks[2],
    );
}
