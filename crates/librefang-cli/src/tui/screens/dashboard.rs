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
    let inner = widgets::render_screen_block(f, area, "\u{25a3} Dashboard");

    let chunks = Layout::vertical([
        Constraint::Length(3), // stat row (compact)
        Constraint::Length(1), // separator
        Constraint::Length(1), // audit header
        Constraint::Min(3),    // audit content
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Stat row (inline, no card borders — cleaner) ──
    draw_stat_row(f, chunks[0], state);

    // ── Separator ──
    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // ── Audit trail ──
    draw_audit_header(f, chunks[2]);
    draw_audit_body(f, chunks[3], state);

    // ── Hints ──
    f.render_widget(
        widgets::hint_bar(
            "  [r] Refresh  [a] Agents  [\u{2191}\u{2193}] Scroll  [PgUp/PgDn] Fast scroll",
        ),
        chunks[4],
    );
}

fn draw_stat_row(f: &mut Frame, area: Rect, state: &DashboardState) {
    let cols = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .split(area);

    // Agents
    draw_stat_cell(
        f,
        cols[0],
        "AGENTS",
        &format!("{}", state.agent_count),
        if state.agent_count > 0 {
            theme::GREEN
        } else {
            theme::TEXT_TERTIARY
        },
    );

    // Uptime
    draw_stat_cell(
        f,
        cols[1],
        "UPTIME",
        &format_uptime(state.uptime_secs),
        theme::BLUE,
    );

    // Provider
    let prov = if state.provider.is_empty() {
        "\u{2014}".to_string()
    } else {
        state.provider.clone()
    };
    draw_stat_cell(f, cols[2], "PROVIDER", &prov, theme::ACCENT);

    // Model
    let model = if state.model.is_empty() {
        "\u{2014}".to_string()
    } else {
        widgets::truncate(&state.model, 16)
    };
    draw_stat_cell(f, cols[3], "MODEL", &model, theme::PURPLE);
}

fn draw_stat_cell(
    f: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    color: ratatui::style::Color,
) {
    let rows = Layout::vertical([
        Constraint::Length(1), // label
        Constraint::Length(1), // value
        Constraint::Min(0),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {label}"),
            Style::default().fg(theme::TEXT_TERTIARY),
        )),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {value}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        rows[1],
    );
}

fn draw_audit_header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<18} {:<12} {:<14} {}",
                "Time", "Agent", "Action", "Detail"
            ),
            theme::table_header(),
        )])),
        area,
    );
}

fn draw_audit_body(f: &mut Frame, area: Rect, state: &DashboardState) {
    if state.loading {
        f.render_widget(widgets::spinner(state.tick, "Loading\u{2026}"), area);
        return;
    }

    if state.recent_audit.is_empty() {
        f.render_widget(widgets::empty_state("No audit entries yet."), area);
        return;
    }

    let lines = items_to_lines(&state.recent_audit);
    let total = lines.len() as u16;
    let visible = area.height;
    let max_scroll = total.saturating_sub(visible);
    let scroll = max_scroll
        .saturating_sub(state.audit_scroll)
        .min(max_scroll);

    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn items_to_lines(rows: &[AuditRow]) -> Vec<Line<'_>> {
    rows.iter()
        .map(|row| {
            let time_short = if row.timestamp.len() > 16 {
                &row.timestamp[row.timestamp.len() - 16..]
            } else {
                &row.timestamp
            };
            Line::from(vec![
                Span::styled(format!("  {:<18}", time_short), theme::dim_style()),
                Span::styled(
                    format!(" {:<12}", widgets::truncate(&row.agent, 11)),
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(
                    format!(" {:<14}", widgets::truncate(&row.action, 13)),
                    Style::default().fg(theme::YELLOW),
                ),
                Span::styled(
                    format!(" {}", widgets::truncate(&row.detail, 28)),
                    theme::dim_style(),
                ),
            ])
        })
        .collect()
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
