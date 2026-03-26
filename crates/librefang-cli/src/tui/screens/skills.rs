//! Skills screen: installed skills, ClawHub marketplace, MCP servers.

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
pub struct SkillInfo {
    pub name: String,
    pub runtime: String,
    pub source: String,
    pub description: String,
}

#[derive(Clone, Default)]
pub struct ClawHubResult {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub downloads: u64,
    pub runtime: String,
}

#[derive(Clone, Default)]
pub struct McpServerInfo {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SkillsSub {
    Installed,
    ClawHub,
    Mcp,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClawHubSort {
    Trending,
    Popular,
    Recent,
}

impl ClawHubSort {
    fn label(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::Popular => "popular",
            Self::Recent => "recent",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::Trending => Self::Popular,
            Self::Popular => Self::Recent,
            Self::Recent => Self::Trending,
        }
    }
}

pub struct SkillsState {
    pub sub: SkillsSub,
    pub installed: Vec<SkillInfo>,
    pub clawhub_results: Vec<ClawHubResult>,
    pub mcp_servers: Vec<McpServerInfo>,
    pub installed_list: ListState,
    pub clawhub_list: ListState,
    pub mcp_list: ListState,
    pub search_buf: String,
    pub search_mode: bool,
    pub sort: ClawHubSort,
    pub loading: bool,
    pub tick: usize,
    pub confirm_uninstall: bool,
    pub status_msg: String,
}

pub enum SkillsAction {
    Continue,
    RefreshInstalled,
    SearchClawHub(String),
    BrowseClawHub(String),
    InstallSkill(String),
    UninstallSkill(String),
    RefreshMcp,
}

impl SkillsState {
    pub fn new() -> Self {
        Self {
            sub: SkillsSub::Installed,
            installed: Vec::new(),
            clawhub_results: Vec::new(),
            mcp_servers: Vec::new(),
            installed_list: ListState::default(),
            clawhub_list: ListState::default(),
            mcp_list: ListState::default(),
            search_buf: String::new(),
            search_mode: false,
            sort: ClawHubSort::Trending,
            loading: false,
            tick: 0,
            confirm_uninstall: false,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SkillsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SkillsAction::Continue;
        }

        // Tab switching within Skills (1/2/3)
        if !self.search_mode {
            match key.code {
                KeyCode::Char('1') => {
                    self.sub = SkillsSub::Installed;
                    return SkillsAction::RefreshInstalled;
                }
                KeyCode::Char('2') => {
                    self.sub = SkillsSub::ClawHub;
                    return SkillsAction::BrowseClawHub(self.sort.label().to_string());
                }
                KeyCode::Char('3') => {
                    self.sub = SkillsSub::Mcp;
                    return SkillsAction::RefreshMcp;
                }
                _ => {}
            }
        }

        match self.sub {
            SkillsSub::Installed => self.handle_installed(key),
            SkillsSub::ClawHub => self.handle_clawhub(key),
            SkillsSub::Mcp => self.handle_mcp(key),
        }
    }

    fn handle_installed(&mut self, key: KeyEvent) -> SkillsAction {
        if self.confirm_uninstall {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_uninstall = false;
                    if let Some(sel) = self.installed_list.selected() {
                        if sel < self.installed.len() {
                            return SkillsAction::UninstallSkill(self.installed[sel].name.clone());
                        }
                    }
                }
                _ => self.confirm_uninstall = false,
            }
            return SkillsAction::Continue;
        }

        let total = self.installed.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.installed_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.installed_list.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.installed_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.installed_list.select(Some(next));
                }
            }
            KeyCode::Char('u') => {
                if self.installed_list.selected().is_some() {
                    self.confirm_uninstall = true;
                }
            }
            KeyCode::Char('r') => return SkillsAction::RefreshInstalled,
            _ => {}
        }
        SkillsAction::Continue
    }

    fn handle_clawhub(&mut self, key: KeyEvent) -> SkillsAction {
        if self.search_mode {
            match key.code {
                KeyCode::Esc => {
                    self.search_mode = false;
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                    if !self.search_buf.is_empty() {
                        return SkillsAction::SearchClawHub(self.search_buf.clone());
                    }
                }
                KeyCode::Backspace => {
                    self.search_buf.pop();
                }
                KeyCode::Char(c) => {
                    self.search_buf.push(c);
                }
                _ => {}
            }
            return SkillsAction::Continue;
        }

        let total = self.clawhub_results.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.clawhub_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.clawhub_list.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.clawhub_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.clawhub_list.select(Some(next));
                }
            }
            KeyCode::Char('i') => {
                if let Some(sel) = self.clawhub_list.selected() {
                    if sel < self.clawhub_results.len() {
                        return SkillsAction::InstallSkill(self.clawhub_results[sel].slug.clone());
                    }
                }
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_buf.clear();
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                return SkillsAction::BrowseClawHub(self.sort.label().to_string());
            }
            KeyCode::Char('r') => {
                return SkillsAction::BrowseClawHub(self.sort.label().to_string());
            }
            _ => {}
        }
        SkillsAction::Continue
    }

    fn handle_mcp(&mut self, key: KeyEvent) -> SkillsAction {
        let total = self.mcp_servers.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.mcp_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.mcp_list.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.mcp_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.mcp_list.select(Some(next));
                }
            }
            KeyCode::Char('r') => return SkillsAction::RefreshMcp,
            _ => {}
        }
        SkillsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let inner = widgets::render_screen_block(f, area, "\u{2605} Skills");

    let chunks = Layout::vertical([
        Constraint::Length(1), // sub-tab bar
        Constraint::Length(1), // separator
        Constraint::Min(3),    // content
    ])
    .split(inner);

    // Sub-tab bar
    draw_sub_tabs(f, chunks[0], state.sub);

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    match state.sub {
        SkillsSub::Installed => draw_installed(f, chunks[2], state),
        SkillsSub::ClawHub => draw_clawhub(f, chunks[2], state),
        SkillsSub::Mcp => draw_mcp(f, chunks[2], state),
    }
}

fn draw_sub_tabs(f: &mut Frame, area: Rect, active: SkillsSub) {
    let tabs = [
        (SkillsSub::Installed, "1 Installed"),
        (SkillsSub::ClawHub, "2 ClawHub"),
        (SkillsSub::Mcp, "3 MCP Servers"),
    ];
    let mut spans = vec![Span::raw("  ")];
    for (sub, label) in &tabs {
        let style = if *sub == active {
            theme::tab_active()
        } else {
            theme::tab_inactive()
        };
        spans.push(Span::styled(format!(" {label} "), style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_installed(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<22} {:<10} {:<12} {}",
                "Name", "Runtime", "Source", "Description"
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, "Loading skills\u{2026}"),
            chunks[1],
        );
    } else if state.installed.is_empty() {
        f.render_widget(
            widgets::empty_state("No skills installed. Browse ClawHub to find skills."),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .installed
            .iter()
            .map(|s| {
                let runtime_style = match s.runtime.as_str() {
                    "python" | "py" => Style::default().fg(theme::BLUE),
                    "node" | "js" => Style::default().fg(theme::YELLOW),
                    "wasm" => Style::default().fg(theme::PURPLE),
                    _ => Style::default().fg(theme::GREEN),
                };
                let runtime_badge = match s.runtime.as_str() {
                    "python" | "py" => "PY",
                    "node" | "js" => "JS",
                    "wasm" => "WASM",
                    "prompt" => "PROMPT",
                    _ => &s.runtime,
                };
                let (source_indicator, source_style) = match s.source.as_str() {
                    "clawhub" => (
                        "\u{25cf}",
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    "builtin" | "built-in" => (
                        "\u{25cf}",
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::BOLD),
                    ),
                    _ => ("\u{25cb}", theme::dim_style()),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {source_indicator} "), source_style),
                    Span::styled(
                        format!("{:<19}", widgets::truncate(&s.name, 18)),
                        Style::default()
                            .fg(theme::TEXT_PRIMARY)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {:<10}", runtime_badge), runtime_style),
                    Span::styled(format!("{:<12}", &s.source), source_style),
                    Span::styled(
                        format!(" {}", widgets::truncate(&s.description, 30)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.installed_list);
    }

    f.render_widget(
        widgets::confirm_or_status_or_hint(
            state.confirm_uninstall,
            "  Uninstall this skill? [y] Yes  [any] Cancel",
            &state.status_msg,
            "  [\u{2191}\u{2193}] Navigate  [u] Uninstall  [r] Refresh",
        ),
        chunks[2],
    );
}

fn draw_clawhub(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // search / sort
        Constraint::Min(3),    // results
        Constraint::Length(1), // hints
    ])
    .split(area);

    if state.search_mode {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  / ", Style::default().fg(theme::ACCENT)),
                Span::styled(&state.search_buf, theme::input_style()),
                Span::styled(
                    "\u{2588}",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ])),
            chunks[0],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(
                        "  {:<24} {:<10} {:<10} {}",
                        "Name", "Downloads", "Runtime", "Description"
                    ),
                    theme::table_header(),
                ),
                Span::styled(
                    format!("  Sort: {}", state.sort.label()),
                    Style::default().fg(theme::YELLOW),
                ),
            ])),
            chunks[0],
        );
    }

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, "Searching ClawHub\u{2026}"),
            chunks[1],
        );
    } else if state.clawhub_results.is_empty() {
        f.render_widget(
            widgets::empty_state("No results. Press [/] to search or [s] to change sort."),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .clawhub_results
            .iter()
            .map(|r| {
                let dl = format_count(r.downloads);
                let runtime_style = match r.runtime.as_str() {
                    "python" | "py" => Style::default().fg(theme::BLUE),
                    "node" | "js" => Style::default().fg(theme::YELLOW),
                    "wasm" => Style::default().fg(theme::PURPLE),
                    _ => Style::default().fg(theme::GREEN),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<24}", widgets::truncate(&r.name, 23)),
                        Style::default()
                            .fg(theme::TEXT_PRIMARY)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {:<10}", dl), Style::default().fg(theme::ACCENT)),
                    Span::styled(format!(" {:<10}", &r.runtime), runtime_style),
                    Span::styled(
                        format!(" {}", widgets::truncate(&r.description, 30)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.clawhub_list);
    }

    f.render_widget(
        widgets::hint_bar(
            "  [\u{2191}\u{2193}] Navigate  [i] Install  [/] Search  [s] Sort  [r] Refresh",
        ),
        chunks[2],
    );
}

fn draw_mcp(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<22} {:<16} {}", "Server", "Status", "Tools"),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, "Loading MCP servers\u{2026}"),
            chunks[1],
        );
    } else if state.mcp_servers.is_empty() {
        f.render_widget(
            widgets::empty_state("No MCP servers configured. Add servers in config.toml."),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .mcp_servers
            .iter()
            .map(|s| {
                let (indicator, label, style) = if s.connected {
                    (
                        "\u{25cf}",
                        "Connected",
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("\u{25cb}", "Disconnected", Style::default().fg(theme::RED))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {indicator} "), style),
                    Span::styled(
                        format!("{:<19}", widgets::truncate(&s.name, 18)),
                        Style::default()
                            .fg(theme::TEXT_PRIMARY)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {:<16}", label), style),
                    Span::styled(format!("{} tools", s.tool_count), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.mcp_list);
    }

    f.render_widget(
        widgets::hint_bar("  [\u{2191}\u{2193}] Navigate  [r] Refresh"),
        chunks[2],
    );
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}
