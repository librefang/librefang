//! Audit screen: audit log viewer with action filter and chain verification.

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
pub struct AuditEntry {
    pub timestamp: String,
    pub action: String,
    pub agent: String,
    pub detail: String,
    pub tip_hash: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuditFilter {
    All,
    AgentSpawn,
    AgentKill,
    ToolInvoke,
    NetworkAccess,
    ShellExec,
}

impl AuditFilter {
    fn label(self) -> String {
        match self {
            Self::All => crate::i18n::t("tui-audit-filter-all"),
            Self::AgentSpawn => crate::i18n::t("tui-audit-filter-spawn"),
            Self::AgentKill => crate::i18n::t("tui-audit-filter-kill"),
            Self::ToolInvoke => crate::i18n::t("tui-audit-filter-tool"),
            Self::NetworkAccess => crate::i18n::t("tui-audit-filter-network"),
            Self::ShellExec => crate::i18n::t("tui-audit-filter-shell"),
        }
    }
    fn next(self) -> Self {
        match self {
            Self::All => Self::AgentSpawn,
            Self::AgentSpawn => Self::AgentKill,
            Self::AgentKill => Self::ToolInvoke,
            Self::ToolInvoke => Self::NetworkAccess,
            Self::NetworkAccess => Self::ShellExec,
            Self::ShellExec => Self::All,
        }
    }
    fn matches(self, action: &str) -> bool {
        match self {
            Self::All => true,
            Self::AgentSpawn => {
                action.contains("Spawn")
                    || action.contains("spawn")
                    || action.contains("Create")
                    || action.contains("create")
            }
            Self::AgentKill => {
                action.contains("Kill")
                    || action.contains("kill")
                    || action.contains("Stop")
                    || action.contains("stop")
            }
            Self::ToolInvoke => {
                action.contains("Tool")
                    || action.contains("tool")
                    || action.contains("Invoke")
                    || action.contains("invoke")
            }
            Self::NetworkAccess => {
                action.contains("Net")
                    || action.contains("net")
                    || action.contains("Fetch")
                    || action.contains("fetch")
                    || action.contains("Http")
                    || action.contains("http")
            }
            Self::ShellExec => {
                action.contains("Shell")
                    || action.contains("shell")
                    || action.contains("Exec")
                    || action.contains("exec")
                    || action.contains("Process")
                    || action.contains("process")
            }
        }
    }
}

/// Map raw action names to friendly display names.
fn friendly_action(action: &str) -> String {
    match action {
        "AgentSpawn" | "AgentSpawned" => crate::i18n::t("tui-audit-action-spawn"),
        "AgentKill" | "AgentKilled" => crate::i18n::t("tui-audit-action-kill"),
        "ToolInvoke" | "ToolInvocation" => crate::i18n::t("tui-audit-action-tool"),
        "NetworkAccess" | "NetFetch" => crate::i18n::t("tui-audit-action-network"),
        "ShellExec" | "ShellCommand" => crate::i18n::t("tui-audit-action-shell"),
        "CapabilityDenied" => crate::i18n::t("tui-audit-action-denied"),
        "ConfigChange" => crate::i18n::t("tui-audit-action-config"),
        other => other.to_string(),
    }
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct AuditState {
    pub entries: Vec<AuditEntry>,
    pub filtered: Vec<usize>,
    pub action_filter: AuditFilter,
    pub list_state: ListState,
    pub chain_verified: Option<bool>,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum AuditUIAction {
    Continue,
    Refresh,
    VerifyChain,
}

impl AuditState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            filtered: Vec::new(),
            action_filter: AuditFilter::All,
            list_state: ListState::default(),
            chain_verified: None,
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn refilter(&mut self) {
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.action_filter.matches(&e.action))
            .map(|(i, _)| i)
            .collect();
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AuditUIAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return AuditUIAction::Continue;
        }

        let total = self.filtered.len();
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
            KeyCode::Char('f') => {
                self.action_filter = self.action_filter.next();
                self.refilter();
            }
            KeyCode::Char('v') => return AuditUIAction::VerifyChain,
            KeyCode::Char('r') => return AuditUIAction::Refresh,
            _ => {}
        }
        AuditUIAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut AuditState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("{} {}", "\u{25c8}", crate::i18n::t("tui-audit-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(3), // filter + header separator + column headers
        Constraint::Min(3),    // list
        Constraint::Length(2), // chain status + hints
    ])
    .split(inner);

    // ── Filter bar + column headers ──
    let filter_style = match state.action_filter {
        AuditFilter::All => Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
        AuditFilter::AgentSpawn => Style::default()
            .fg(theme::GREEN)
            .add_modifier(Modifier::BOLD),
        AuditFilter::AgentKill => Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        AuditFilter::ToolInvoke => Style::default()
            .fg(theme::BLUE)
            .add_modifier(Modifier::BOLD),
        AuditFilter::NetworkAccess => Style::default()
            .fg(theme::YELLOW)
            .add_modifier(Modifier::BOLD),
        AuditFilter::ShellExec => Style::default()
            .fg(theme::PURPLE)
            .add_modifier(Modifier::BOLD),
    };

    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!("  {} ", crate::i18n::t("tui-audit-label-filter")),
                    theme::dim_style(),
                ),
                Span::styled(format!("[{}]", state.action_filter.label()), filter_style),
                Span::styled(
                    format!(
                        "  │ {}",
                        crate::i18n::t_args(
                            "tui-audit-entries-count",
                            &[("count", &state.filtered.len().to_string())]
                        )
                    ),
                    theme::dim_style(),
                ),
            ]),
            Line::from(vec![Span::styled(
                "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
                Style::default().fg(theme::BORDER),
            )]),
            Line::from(vec![
                Span::styled(
                    format!("  {:<20}", crate::i18n::t("tui-audit-header-timestamp")),
                    theme::table_header(),
                ),
                Span::styled(
                    format!(" {:<16}", crate::i18n::t("tui-audit-header-action")),
                    theme::table_header(),
                ),
                Span::styled(
                    format!(" {:<14}", crate::i18n::t("tui-audit-header-agent")),
                    theme::table_header(),
                ),
                Span::styled(
                    format!(" {:<10}", crate::i18n::t("tui-audit-header-hash")),
                    theme::table_header(),
                ),
                Span::styled(
                    format!(" {}", crate::i18n::t("tui-audit-header-detail")),
                    theme::table_header(),
                ),
            ]),
        ]),
        chunks[0],
    );

    // ── List ──
    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-audit-loading")),
            chunks[1],
        );
    } else if state.filtered.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-audit-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .filtered
            .iter()
            .map(|&idx| {
                let e = &state.entries[idx];
                let action_display = friendly_action(&e.action);
                let action_style = if e.action.contains("Kill") || e.action.contains("Denied") {
                    Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
                } else if e.action.contains("Spawn") || e.action.contains("Create") {
                    Style::default().fg(theme::GREEN)
                } else if e.action.contains("Tool") {
                    Style::default().fg(theme::BLUE)
                } else if e.action.contains("Shell")
                    || e.action.contains("Exec")
                    || e.action.contains("Process")
                {
                    Style::default().fg(theme::PURPLE)
                } else if e.action.contains("Net")
                    || e.action.contains("Fetch")
                    || e.action.contains("Http")
                {
                    Style::default().fg(theme::YELLOW)
                } else if e.action.contains("Config") {
                    Style::default().fg(theme::ACCENT_DIM)
                } else {
                    Style::default().fg(theme::TEXT_SECONDARY)
                };
                let hash_short = if e.tip_hash.len() > 8 {
                    &e.tip_hash[..8]
                } else {
                    &e.tip_hash
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<20}", widgets::truncate(&e.timestamp, 19)),
                        theme::dim_style(),
                    ),
                    Span::styled(
                        format!(" {:<16}", widgets::truncate(&action_display, 15)),
                        action_style,
                    ),
                    Span::styled(
                        format!(" {:<14}", widgets::truncate(&e.agent, 13)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {:<10}", hash_short),
                        Style::default().fg(theme::PURPLE),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&e.detail, 24)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    // ── Chain status + hints ──
    let chain_line = match state.chain_verified {
        None => Line::from(vec![Span::styled(
            format!("  ○ {}", crate::i18n::t("tui-audit-chain-unverified")),
            theme::dim_style(),
        )]),
        Some(true) => Line::from(vec![Span::styled(
            format!("  ✔ {}", crate::i18n::t("tui-audit-chain-verified")),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )]),
        Some(false) => Line::from(vec![Span::styled(
            format!("  ✘ {}", crate::i18n::t("tui-audit-chain-failed")),
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        )]),
    };

    let hints = if !state.status_msg.is_empty() {
        Line::from(vec![Span::styled(
            format!("  {}", state.status_msg),
            Style::default().fg(theme::GREEN),
        )])
    } else {
        Line::from(vec![Span::styled(
            crate::i18n::t("tui-audit-hints"),
            theme::hint_style(),
        )])
    };

    f.render_widget(Paragraph::new(vec![chain_line, hints]), chunks[2]);
}
