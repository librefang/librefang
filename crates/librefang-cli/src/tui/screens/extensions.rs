//! Extensions screen: browse MCP catalog, install/remove MCP servers, view health.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub icon: String,
    pub installed: bool,
    pub status: String,
    pub tags: Vec<String>,
    #[allow(dead_code)]
    pub has_oauth: bool,
}

#[derive(Clone, Default)]
pub struct ExtensionHealthInfo {
    pub id: String,
    pub status: String,
    pub tool_count: usize,
    #[allow(dead_code)]
    pub last_ok: String,
    pub last_error: String,
    pub consecutive_failures: u32,
    pub reconnecting: bool,
    pub connected_since: String,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExtSub {
    Browse,
    Installed,
    Health,
}

pub struct ExtensionsState {
    pub sub: ExtSub,
    pub all_extensions: Vec<ExtensionInfo>,
    pub health_entries: Vec<ExtensionHealthInfo>,
    pub browse_list: ListState,
    pub installed_list: ListState,
    pub health_list: ListState,
    pub search_query: String,
    pub searching: bool,
    pub loading: bool,
    pub tick: usize,
    pub confirm_remove: bool,
    pub status_msg: String,
}

pub enum ExtensionsAction {
    Continue,
    RefreshAll,
    RefreshHealth,
    Install(String),
    Remove(String),
    Reconnect(String),
}

impl ExtensionsState {
    pub fn new() -> Self {
        Self {
            sub: ExtSub::Browse,
            all_extensions: Vec::new(),
            health_entries: Vec::new(),
            browse_list: ListState::default(),
            installed_list: ListState::default(),
            health_list: ListState::default(),
            search_query: String::new(),
            searching: false,
            loading: false,
            tick: 0,
            confirm_remove: false,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn filtered(&self) -> Vec<&ExtensionInfo> {
        let q = self.search_query.to_lowercase();
        self.all_extensions
            .iter()
            .filter(|e| {
                if q.is_empty() {
                    return true;
                }
                e.name.to_lowercase().contains(&q)
                    || e.id.to_lowercase().contains(&q)
                    || e.category.to_lowercase().contains(&q)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }

    fn installed_list_data(&self) -> Vec<&ExtensionInfo> {
        self.all_extensions.iter().filter(|e| e.installed).collect()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ExtensionsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return ExtensionsAction::Continue;
        }

        // Search mode
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.search_query.clear();
                }
                KeyCode::Enter => {
                    self.searching = false;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return ExtensionsAction::Continue;
        }

        // Sub-tab switching (1/2/3)
        match key.code {
            KeyCode::Char('1') => {
                self.sub = ExtSub::Browse;
                return ExtensionsAction::RefreshAll;
            }
            KeyCode::Char('2') => {
                self.sub = ExtSub::Installed;
                return ExtensionsAction::RefreshAll;
            }
            KeyCode::Char('3') => {
                self.sub = ExtSub::Health;
                return ExtensionsAction::RefreshHealth;
            }
            KeyCode::Char('/') if self.sub == ExtSub::Browse => {
                self.searching = true;
                self.search_query.clear();
                return ExtensionsAction::Continue;
            }
            _ => {}
        }

        match self.sub {
            ExtSub::Browse => self.handle_browse(key),
            ExtSub::Installed => self.handle_installed(key),
            ExtSub::Health => self.handle_health(key),
        }
    }

    fn handle_browse(&mut self, key: KeyEvent) -> ExtensionsAction {
        let total = self.filtered().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.browse_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.browse_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.browse_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.browse_list.select(Some(next));
            }
            KeyCode::Enter => {
                let filtered = self.filtered();
                if let Some(sel) = self.browse_list.selected() {
                    if sel < filtered.len() {
                        let ext = filtered[sel];
                        if !ext.installed {
                            return ExtensionsAction::Install(ext.id.clone());
                        }
                    }
                }
            }
            KeyCode::Char('r') => return ExtensionsAction::RefreshAll,
            _ => {}
        }
        ExtensionsAction::Continue
    }

    fn handle_installed(&mut self, key: KeyEvent) -> ExtensionsAction {
        if self.confirm_remove {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_remove = false;
                    let installed = self.installed_list_data();
                    if let Some(sel) = self.installed_list.selected() {
                        if sel < installed.len() {
                            return ExtensionsAction::Remove(installed[sel].id.clone());
                        }
                    }
                }
                _ => self.confirm_remove = false,
            }
            return ExtensionsAction::Continue;
        }

        let total = self.installed_list_data().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.installed_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.installed_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.installed_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.installed_list.select(Some(next));
            }
            KeyCode::Char('d') | KeyCode::Delete if self.installed_list.selected().is_some() => {
                self.confirm_remove = true;
            }
            KeyCode::Char('r') => return ExtensionsAction::RefreshAll,
            _ => {}
        }
        ExtensionsAction::Continue
    }

    fn handle_health(&mut self, key: KeyEvent) -> ExtensionsAction {
        let total = self.health_entries.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.health_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.health_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.health_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.health_list.select(Some(next));
            }
            KeyCode::Char('r') | KeyCode::Enter => {
                if let Some(sel) = self.health_list.selected() {
                    if sel < self.health_entries.len() {
                        return ExtensionsAction::Reconnect(self.health_entries[sel].id.clone());
                    }
                }
            }
            _ => {}
        }
        ExtensionsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut ExtensionsState) {
    let title = format!("⧉ {}", crate::i18n::t("tui-extensions-title-screen"));
    let inner = widgets::render_screen_block(f, area, &title);

    let chunks = Layout::vertical([
        Constraint::Length(1), // sub-tab bar
        Constraint::Length(1), // separator
        Constraint::Min(3),    // content
    ])
    .split(inner);

    draw_sub_tabs(f, chunks[0], state);

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    match state.sub {
        ExtSub::Browse => draw_browse(f, chunks[2], state),
        ExtSub::Installed => draw_installed(f, chunks[2], state),
        ExtSub::Health => draw_health(f, chunks[2], state),
    }
}

fn draw_sub_tabs(f: &mut Frame, area: Rect, state: &ExtensionsState) {
    let tabs = [
        (ExtSub::Browse, crate::i18n::t("tui-extensions-tab-browse")),
        (
            ExtSub::Installed,
            crate::i18n::t("tui-extensions-tab-installed"),
        ),
        (ExtSub::Health, crate::i18n::t("tui-extensions-tab-health")),
    ];
    let mut spans = vec![Span::raw("  ")];
    for (i, (sub, label)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(theme::BORDER)));
        }
        if *sub == state.sub {
            spans.push(Span::styled(format!(" ● {label} "), theme::tab_active()));
        } else {
            spans.push(Span::styled(format!(" ○ {label} "), theme::tab_inactive()));
        }
    }

    // Show search query if active
    if state.searching {
        spans.push(Span::raw("   "));
        spans.push(Span::styled("🔍 ", Style::default().fg(theme::YELLOW)));
        spans.push(Span::styled(
            format!("{}█", state.search_query),
            theme::input_style(),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn status_badge(status: &str) -> (String, Style) {
    let lower = status.to_lowercase();
    if lower.contains("ready") || lower.contains("connected") {
        (
            format!("● {}", crate::i18n::t("tui-extensions-status-ready")),
            Style::default().fg(theme::GREEN),
        )
    } else if lower.contains("setup") {
        (
            format!("◑ {}", crate::i18n::t("tui-extensions-status-setup")),
            Style::default().fg(theme::YELLOW),
        )
    } else if lower.contains("error") {
        (
            format!("● {}", crate::i18n::t("tui-extensions-status-error")),
            Style::default().fg(theme::RED),
        )
    } else if lower.contains("disabled") {
        (
            format!("○ {}", crate::i18n::t("tui-extensions-status-off")),
            theme::dim_style(),
        )
    } else {
        ("○ ---".to_string(), theme::dim_style())
    }
}

fn draw_browse(f: &mut Frame, area: Rect, state: &mut ExtensionsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<3} {:<18} {:<12} {:<10} {}",
                "",
                crate::i18n::t("tui-extensions-header-name"),
                crate::i18n::t("tui-extensions-header-category"),
                crate::i18n::t("tui-extensions-header-status"),
                crate::i18n::t("tui-extensions-header-desc")
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-extensions-loading")),
            chunks[1],
        );
    } else if state.all_extensions.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-extensions-empty")),
            chunks[1],
        );
    } else {
        // Collect filtered data to avoid borrow conflict with browse_list
        let items: Vec<ListItem> = state
            .filtered()
            .iter()
            .map(|ext| {
                let (badge, badge_style) = if ext.installed {
                    (
                        format!("● {}", crate::i18n::t("tui-extensions-status-installed")),
                        Style::default().fg(theme::GREEN),
                    )
                } else {
                    (
                        format!("○ {}", crate::i18n::t("tui-extensions-status-available")),
                        theme::dim_style(),
                    )
                };
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{} ", ext.icon), Style::default()),
                    Span::styled(
                        format!("{:<16} ", ext.name),
                        Style::default().fg(theme::TEXT_PRIMARY),
                    ),
                    Span::styled(format!("{:<12} ", ext.category), theme::dim_style()),
                    Span::styled(format!("{:<10} ", badge), badge_style),
                    Span::styled(ext.description.clone(), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.browse_list);
    }

    let hints = if state.searching {
        crate::i18n::t("tui-extensions-hints-search")
    } else {
        crate::i18n::t("tui-extensions-hints-browse")
    };
    f.render_widget(widgets::hint_bar(&hints), chunks[2]);
}

fn draw_installed(f: &mut Frame, area: Rect, state: &mut ExtensionsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<3} {:<18} {:<12} {:<10} {}",
                "",
                crate::i18n::t("tui-extensions-header-name"),
                crate::i18n::t("tui-extensions-header-category"),
                crate::i18n::t("tui-extensions-header-status"),
                crate::i18n::t("tui-extensions-header-id")
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    // Collect installed items into owned data to avoid borrow conflict with installed_list
    let items: Vec<ListItem> = state
        .all_extensions
        .iter()
        .filter(|e| e.installed)
        .map(|ext| {
            let (badge, badge_style) = status_badge(&ext.status);
            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{} ", ext.icon), Style::default()),
                Span::styled(
                    format!("{:<16} ", ext.name),
                    Style::default().fg(theme::TEXT_PRIMARY),
                ),
                Span::styled(format!("{:<12} ", ext.category), theme::dim_style()),
                Span::styled(format!("{:<10} ", badge), badge_style),
                Span::styled(ext.id.clone(), theme::dim_style()),
            ]))
        })
        .collect();

    if items.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-extensions-empty")),
            chunks[1],
        );
    } else {
        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.installed_list);
    }

    f.render_widget(
        widgets::confirm_or_status_or_hint(
            state.confirm_remove,
            &crate::i18n::t("tui-extensions-remove-confirm"),
            &state.status_msg,
            &crate::i18n::t("tui-extensions-hints-installed"),
        ),
        chunks[2],
    );
}

fn draw_health(f: &mut Frame, area: Rect, state: &mut ExtensionsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<18} {:<10} {:<6} {:<12} {:<6} {}",
                crate::i18n::t("tui-extensions-header-server"),
                crate::i18n::t("tui-extensions-header-status"),
                crate::i18n::t("tui-extensions-header-tools"),
                crate::i18n::t("tui-extensions-header-connected"),
                crate::i18n::t("tui-extensions-header-fails"),
                crate::i18n::t("tui-extensions-header-last-error")
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.health_entries.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-extensions-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .health_entries
            .iter()
            .map(|h| {
                let (badge, badge_style) = status_badge(&h.status);
                let error_display = if h.last_error.is_empty() {
                    "—".to_string()
                } else if h.last_error.len() > 30 {
                    format!("{}…", librefang_types::truncate_str(&h.last_error, 27))
                } else {
                    h.last_error.clone()
                };
                let reconn = if h.reconnecting { " ↻" } else { "" };
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{:<16} ", h.id),
                        Style::default().fg(theme::TEXT_PRIMARY),
                    ),
                    Span::styled(format!("{:<10} ", badge), badge_style),
                    Span::styled(
                        format!("{:<6} ", h.tool_count),
                        Style::default().fg(theme::BLUE),
                    ),
                    Span::styled(
                        format!(
                            "{:<12} ",
                            if h.connected_since.is_empty() {
                                "—"
                            } else {
                                &h.connected_since
                            }
                        ),
                        theme::dim_style(),
                    ),
                    Span::styled(
                        format!("{:<6}", h.consecutive_failures),
                        if h.consecutive_failures > 0 {
                            Style::default().fg(theme::RED)
                        } else {
                            theme::dim_style()
                        },
                    ),
                    Span::styled(format!(" {error_display}{reconn}"), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.health_list);
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-extensions-hints-health")),
        chunks[2],
    );
}
