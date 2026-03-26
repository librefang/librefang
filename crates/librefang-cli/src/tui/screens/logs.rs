//! Logs screen: real-time log viewer with level filter and search.

use crate::tui::{theme, widgets};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub action: String,
    pub detail: String,
    pub agent: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
}

impl LogLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Error => "ERR",
            Self::Warn => "WRN",
            Self::Info => "INF",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::Error => Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
            Self::Warn => Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
            Self::Info => Style::default().fg(theme::BLUE),
        }
    }
}

/// Classify log level from action/detail keywords.
pub fn classify_level(action: &str, detail: &str) -> LogLevel {
    let combined = format!("{action} {detail}").to_lowercase();
    if combined.contains("error")
        || combined.contains("fail")
        || combined.contains("crash")
        || combined.contains("panic")
    {
        LogLevel::Error
    } else if combined.contains("warn")
        || combined.contains("deny")
        || combined.contains("denied")
        || combined.contains("block")
        || combined.contains("timeout")
    {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

// ── Filter ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LevelFilter {
    All,
    Error,
    Warn,
    Info,
}

impl LevelFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Error => "Error",
            Self::Warn => "Warn",
            Self::Info => "Info",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::All => Self::Error,
            Self::Error => Self::Warn,
            Self::Warn => Self::Info,
            Self::Info => Self::All,
        }
    }
    fn matches(self, level: LogLevel) -> bool {
        match self {
            Self::All => true,
            Self::Error => level == LogLevel::Error,
            Self::Warn => level == LogLevel::Warn,
            Self::Info => level == LogLevel::Info,
        }
    }
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct LogsState {
    pub entries: Vec<LogEntry>,
    pub filtered: Vec<usize>,
    pub level_filter: LevelFilter,
    pub search_buf: String,
    pub search_mode: bool,
    pub auto_refresh: bool,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub poll_tick: usize,
}

pub enum LogsAction {
    Continue,
    Refresh,
}

impl LogsState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            filtered: Vec::new(),
            level_filter: LevelFilter::All,
            search_buf: String::new(),
            search_mode: false,
            auto_refresh: true,
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

    /// Returns true if it's time to auto-refresh (every ~2s at 20fps tick rate).
    pub fn should_poll(&self) -> bool {
        self.auto_refresh && self.poll_tick > 0 && self.poll_tick.is_multiple_of(40)
    }

    pub fn refilter(&mut self) {
        let search_lower = self.search_buf.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if !self.level_filter.matches(e.level) {
                    return false;
                }
                if !search_lower.is_empty() {
                    let haystack = format!("{} {}", e.action, e.detail).to_lowercase();
                    if !haystack.contains(&search_lower) {
                        return false;
                    }
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        // Auto-scroll to bottom on new entries
        if !self.filtered.is_empty() {
            self.list_state.select(Some(self.filtered.len() - 1));
        } else {
            self.list_state.select(None);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> LogsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return LogsAction::Continue;
        }

        if self.search_mode {
            match key.code {
                KeyCode::Esc => {
                    self.search_mode = false;
                    self.search_buf.clear();
                    self.refilter();
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                    self.refilter();
                }
                KeyCode::Backspace => {
                    self.search_buf.pop();
                    self.refilter();
                }
                KeyCode::Char(c) => {
                    self.search_buf.push(c);
                    self.refilter();
                }
                _ => {}
            }
            return LogsAction::Continue;
        }

        let total = self.filtered.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.list_state.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.list_state.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.list_state.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.list_state.select(Some(next));
                }
            }
            KeyCode::Char('f') => {
                self.level_filter = self.level_filter.next();
                self.refilter();
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_buf.clear();
            }
            KeyCode::Char('a') => {
                self.auto_refresh = !self.auto_refresh;
            }
            KeyCode::Char('r') => return LogsAction::Refresh,
            KeyCode::End => {
                if total > 0 {
                    self.list_state.select(Some(total - 1));
                }
            }
            KeyCode::Home => {
                if total > 0 {
                    self.list_state.select(Some(0));
                }
            }
            _ => {}
        }
        LogsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut LogsState) {
    let inner = widgets::render_screen_block(f, area, "Logs");

    let chunks = Layout::vertical([
        Constraint::Length(2), // header: filter + search
        Constraint::Min(3),    // log list
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Header ──
    let table_header_text = format!(
        "  {:<20} {:<6} {:<16} {:<14} {}",
        "Timestamp", "Level", "Action", "Agent", "Detail"
    );

    if state.search_mode {
        let header_rows =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[0]);
        f.render_widget(widgets::search_input(&state.search_buf), header_rows[0]);
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                &table_header_text,
                theme::table_header(),
            )])),
            header_rows[1],
        );
    } else {
        let auto_badge = if state.auto_refresh {
            Span::styled(" [auto-refresh ON]", Style::default().fg(theme::GREEN))
        } else {
            Span::styled(" [auto-refresh OFF]", theme::dim_style())
        };
        let search_hint = if state.search_buf.is_empty() {
            Span::raw("")
        } else {
            Span::styled(
                format!("  filter: \"{}\"", state.search_buf),
                Style::default().fg(theme::YELLOW),
            )
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("  Level: ", theme::dim_style()),
                    Span::styled(
                        format!("[{}]", state.level_filter.label()),
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  ({} entries)", state.filtered.len()),
                        theme::dim_style(),
                    ),
                    auto_badge,
                    search_hint,
                ]),
                Line::from(vec![Span::styled(
                    &table_header_text,
                    theme::table_header(),
                )]),
            ]),
            chunks[0],
        );
    }

    // ── Log list ──
    if state.loading && state.entries.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, "Loading logs\u{2026}"),
            chunks[1],
        );
    } else if state.filtered.is_empty() {
        f.render_widget(
            widgets::empty_state("No log entries match the current filter."),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .filtered
            .iter()
            .map(|&idx| {
                let e = &state.entries[idx];
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<20}", widgets::truncate(&e.timestamp, 19)),
                        theme::dim_style(),
                    ),
                    Span::styled(format!(" {:<6}", e.level.label()), e.level.style()),
                    Span::styled(
                        format!(" {:<16}", widgets::truncate(&e.action, 15)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {:<14}", widgets::truncate(&e.agent, 13)),
                        Style::default().fg(theme::PURPLE),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&e.detail, 30)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    // ── Hints ──
    f.render_widget(
        widgets::hint_bar("  [\u{2191}\u{2193}] Navigate  [f] Filter Level  [/] Search  [a] Toggle Auto-refresh  [r] Refresh"),
        chunks[2],
    );
}
