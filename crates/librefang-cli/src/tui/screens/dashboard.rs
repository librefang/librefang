//! Dashboard screen: system overview with stat cards and scrollable audit trail.

use crate::tui::{theme, widgets};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct AuditRow {
    pub timestamp: String,
    pub agent: String,
    pub action: String,
    pub detail: String,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct DashboardState {
    pub agent_count: u64,
    pub uptime_secs: u64,
    pub version: String,
    pub provider: String,
    pub model: String,
    pub recent_audit: Vec<AuditRow>,
    pub loading: bool,
    pub tick: usize,
    pub audit_scroll: u16,
}

pub enum DashboardAction {
    Continue,
    Refresh,
    GoToAgents,
}

impl DashboardState {
    pub fn new() -> Self {
        Self {
            agent_count: 0,
            uptime_secs: 0,
            version: String::new(),
            provider: String::new(),
            model: String::new(),
            recent_audit: Vec::new(),
            loading: false,
            tick: 0,
            audit_scroll: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DashboardAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return DashboardAction::Continue;
        }
        match key.code {
            KeyCode::Char('r') => DashboardAction::Refresh,
            KeyCode::Char('a') => DashboardAction::GoToAgents,
            KeyCode::Up | KeyCode::Char('k') => {
                self.audit_scroll = self.audit_scroll.saturating_add(1);
                DashboardAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.audit_scroll = self.audit_scroll.saturating_sub(1);
                DashboardAction::Continue
            }
            KeyCode::PageUp => {
                self.audit_scroll = self.audit_scroll.saturating_add(10);
                DashboardAction::Continue
            }
            KeyCode::PageDown => {
                self.audit_scroll = self.audit_scroll.saturating_sub(10);
                DashboardAction::Continue
            }
            _ => DashboardAction::Continue,
        }
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut DashboardState) {
    let inner = widgets::render_screen_block(f, area, "Dashboard");

    let chunks = Layout::vertical([
        Constraint::Length(5), // stat cards
        Constraint::Length(1), // separator
        Constraint::Min(4),    // audit trail
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Stat cards ──────────────────────────────────────────────────────────
    draw_stat_cards(f, chunks[0], state);

    // ── Separator ───────────────────────────────────────────────────────────
    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // ── Audit trail ─────────────────────────────────────────────────────────
    draw_audit_trail(f, chunks[2], state);

    // ── Hints ───────────────────────────────────────────────────────────────
    f.render_widget(
        widgets::hint_bar("  [r] Refresh  [a] Go to Agents  [\u{2191}\u{2193}] Scroll audit"),
        chunks[3],
    );
}

fn draw_stat_cards(f: &mut Frame, area: Rect, state: &DashboardState) {
    let cols = Layout::horizontal([
        Constraint::Percentage(33),
        Constraint::Percentage(34),
        Constraint::Percentage(33),
    ])
    .split(area);

    // Agents card
    let agents_inner = widgets::render_card_block(f, cols[0], "Agents");
    let count_text = format!("{}", state.agent_count);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {count_text}"),
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" active", theme::dim_style()),
        ])),
        agents_inner,
    );

    // Uptime card
    let uptime_inner = widgets::render_card_block(f, cols[1], "Uptime");
    let uptime_str = format_uptime(state.uptime_secs);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(" {uptime_str}"),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )])),
        uptime_inner,
    );

    // Provider card
    let provider_inner = widgets::render_card_block(f, cols[2], "Provider");
    let provider_text = if state.provider.is_empty() {
        "not set".to_string()
    } else {
        format!("{}/{}", state.provider, state.model)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(" {provider_text}"),
            Style::default().fg(theme::CYAN),
        )])),
        provider_inner,
    );
}

fn draw_audit_trail(f: &mut Frame, area: Rect, state: &DashboardState) {
    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, "Loading audit trail\u{2026}"),
            area,
        );
        return;
    }

    if state.recent_audit.is_empty() {
        f.render_widget(widgets::empty_state("No audit entries yet."), area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![Span::styled(
        format!(
            "  {:<20} {:<14} {:<16} {}",
            "Timestamp", "Agent", "Action", "Detail"
        ),
        theme::table_header(),
    )]));

    for row in &state.recent_audit {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<20}", row.timestamp), theme::dim_style()),
            Span::styled(
                format!(" {:<14}", widgets::truncate(&row.agent, 13)),
                Style::default().fg(theme::CYAN),
            ),
            Span::styled(
                format!(" {:<16}", widgets::truncate(&row.action, 15)),
                Style::default().fg(theme::YELLOW),
            ),
            Span::styled(
                format!(" {}", widgets::truncate(&row.detail, 30)),
                theme::dim_style(),
            ),
        ]));
    }

    let total = lines.len() as u16;
    let visible = area.height;
    let max_scroll = total.saturating_sub(visible);
    let scroll = max_scroll
        .saturating_sub(state.audit_scroll)
        .min(max_scroll);

    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}
