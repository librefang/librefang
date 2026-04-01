//! Welcome screen: branded logo, daemon/provider status, mode selection menu.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::tui::theme;
use crate::tui::widgets;

// ── ASCII Logo ───────────────────────────────────────────────────────────────

const LOGO: &str = r#"██╗     ██╗██████╗ ██████╗ ███████╗███████╗ █████╗ ███╗   ██╗ ██████╗
██║     ██║██╔══██╗██╔══██╗██╔════╝██╔════╝██╔══██╗████╗  ██║██╔════╝
██║     ██║██████╔╝██████╔╝█████╗  █████╗  ███████║██╔██╗ ██║██║  ███╗
██║     ██║██╔══██╗██╔══██╗██╔══╝  ██╔══╝  ██╔══██║██║╚██╗██║██║   ██║
███████╗██║██████╔╝██║  ██║███████╗██║     ██║  ██║██║ ╚████║╚██████╔╝
╚══════╝╚═╝╚═════╝ ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝  ╚═╝╚═╝  ╚═══╝ ╚═════╝"#;

const LOGO_HEIGHT: u16 = 6;
const LOGO_MIN_WIDTH: u16 = 75;
const COMPACT_LOGO: &str = "L I B R E F A N G";

// ── Provider detection ───────────────────────────────────────────────────────

/// Known provider env vars, checked in priority order.
const PROVIDER_ENV_VARS: &[(&str, &str)] = &[
    ("ANTHROPIC_API_KEY", "Anthropic"),
    ("OPENAI_API_KEY", "OpenAI"),
    ("DEEPSEEK_API_KEY", "DeepSeek"),
    ("GEMINI_API_KEY", "Gemini"),
    ("GOOGLE_API_KEY", "Gemini"),
    ("GROQ_API_KEY", "Groq"),
    ("OPENROUTER_API_KEY", "OpenRouter"),
    ("TOGETHER_API_KEY", "Together"),
    ("MISTRAL_API_KEY", "Mistral"),
    ("FIREWORKS_API_KEY", "Fireworks"),
    ("BRAVE_API_KEY", "Brave Search"),
    ("TAVILY_API_KEY", "Tavily"),
    ("PERPLEXITY_API_KEY", "Perplexity"),
];

/// Returns (provider_name, env_var_name) for the first detected key, or None.
fn detect_provider() -> Option<(&'static str, &'static str)> {
    for &(var, name) in PROVIDER_ENV_VARS {
        if std::env::var(var).map_or(false, |v| !v.trim().is_empty()) {
            return Some((name, var));
        }
    }
    None
}

// ── State ────────────────────────────────────────────────────────────────────

pub struct WelcomeState {
    pub menu: ListState,
    pub daemon_url: Option<String>,
    pub daemon_agents: u64,
    pub menu_items: Vec<MenuItem>,
    pub detecting: bool,
    pub tick: usize,
    pub ctrl_c_pending: bool,
    ctrl_c_tick: usize,
    pub setup_just_completed: bool,
}

pub struct MenuItem {
    pub label: &'static str,
    pub hint: &'static str,
    pub icon: &'static str,
    pub action: WelcomeAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WelcomeAction {
    ConnectDaemon,
    InProcess,
    Wizard,
    Exit,
}

impl WelcomeState {
    const CTRL_C_TIMEOUT: usize = 40;

    pub fn new() -> Self {
        Self {
            menu: ListState::default(),
            daemon_url: None,
            daemon_agents: 0,
            menu_items: Vec::new(),
            detecting: true,
            tick: 0,
            ctrl_c_pending: false,
            ctrl_c_tick: 0,
            setup_just_completed: false,
        }
    }

    pub fn on_daemon_detected(&mut self, url: Option<String>, agent_count: u64) {
        self.detecting = false;
        self.daemon_url = url;
        self.daemon_agents = agent_count;
        self.rebuild_menu();
    }

    fn rebuild_menu(&mut self) {
        self.menu_items.clear();
        if self.daemon_url.is_some() {
            self.menu_items.push(MenuItem {
                label: "Connect to daemon",
                hint: "talk to running agents via API",
                icon: "\u{25cf}",
                action: WelcomeAction::ConnectDaemon,
            });
        }
        self.menu_items.push(MenuItem {
            label: "Quick chat",
            hint: "boot kernel locally, no daemon needed",
            icon: "\u{25b8}",
            action: WelcomeAction::InProcess,
        });
        self.menu_items.push(MenuItem {
            label: "Setup wizard",
            hint: "configure providers & channels",
            icon: "\u{2699}",
            action: WelcomeAction::Wizard,
        });
        self.menu_items.push(MenuItem {
            label: "Exit",
            hint: "quit LibreFang",
            icon: "\u{2190}",
            action: WelcomeAction::Exit,
        });
        self.menu.select(Some(0));
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.ctrl_c_pending && self.tick.wrapping_sub(self.ctrl_c_tick) > Self::CTRL_C_TIMEOUT {
            self.ctrl_c_pending = false;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<WelcomeAction> {
        let is_ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);

        if self.detecting {
            if is_ctrl_c {
                if self.ctrl_c_pending {
                    return Some(WelcomeAction::Exit);
                }
                self.ctrl_c_pending = true;
                self.ctrl_c_tick = self.tick;
                return None;
            }
            if key.code == KeyCode::Char('q') {
                return Some(WelcomeAction::Exit);
            }
            self.ctrl_c_pending = false;
            return None;
        }

        if is_ctrl_c {
            if self.ctrl_c_pending {
                return Some(WelcomeAction::Exit);
            }
            self.ctrl_c_pending = true;
            self.ctrl_c_tick = self.tick;
            return None;
        }
        self.ctrl_c_pending = false;

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Some(WelcomeAction::Exit),
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.menu.selected().unwrap_or(0);
                let next = if i == 0 {
                    self.menu_items.len() - 1
                } else {
                    i - 1
                };
                self.menu.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.menu.selected().unwrap_or(0);
                let next = (i + 1) % self.menu_items.len();
                self.menu.select(Some(next));
            }
            KeyCode::Enter => {
                if let Some(i) = self.menu.selected() {
                    return Some(self.menu_items[i].action);
                }
            }
            _ => {}
        }
        None
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut WelcomeState) {
    // Fill background
    f.render_widget(
        Block::default().style(Style::default().bg(theme::BG_PRIMARY)),
        area,
    );

    let version = env!("CARGO_PKG_VERSION");
    let compact = area.width < LOGO_MIN_WIDTH;
    let logo_h: u16 = if compact { 1 } else { LOGO_HEIGHT };

    // Center content vertically (upper third)
    let content = centered_content(area);

    let chunks = Layout::vertical([
        Constraint::Length(2),      // top padding
        Constraint::Length(logo_h), // logo
        Constraint::Length(1),      // tagline
        Constraint::Length(1),      // blank
        Constraint::Length(3),      // status card
        Constraint::Length(1),      // blank
        Constraint::Min(1),         // menu
        Constraint::Length(1),      // hints
        Constraint::Min(0),         // remaining
    ])
    .split(content);

    // ── Logo ──
    draw_logo(f, chunks[1], compact);

    // ── Tagline ──
    let tagline = Line::from(vec![
        Span::styled(
            "Agent Operating System",
            Style::default()
                .fg(theme::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  v{version}"), theme::dim_style()),
    ]);
    f.render_widget(Paragraph::new(tagline), chunks[2]);

    // ── Status card ──
    draw_status_card(f, chunks[4], state);

    // ── Menu ──
    if !state.detecting {
        draw_menu(f, chunks[6], state);
    }

    // ── Hints ──
    if state.ctrl_c_pending {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Press Ctrl+C again to exit",
                Style::default().fg(theme::YELLOW),
            )),
            chunks[7],
        );
    } else {
        f.render_widget(
            widgets::hint_bar("\u{2191}\u{2193} navigate  enter select  q quit"),
            chunks[7],
        );
    }
}

fn centered_content(area: Rect) -> Rect {
    if area.width < 10 || area.height < 5 {
        return area;
    }
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    Rect {
        x,
        y: area.y,
        width: w,
        height: area.height,
    }
}

fn draw_logo(f: &mut Frame, area: Rect, compact: bool) {
    if compact {
        f.render_widget(
            Paragraph::new(Span::styled(
                COMPACT_LOGO,
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
            area,
        );
    } else {
        let lines: Vec<Line> = LOGO
            .lines()
            .map(|l| Line::from(Span::styled(l, Style::default().fg(theme::ACCENT))))
            .collect();
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
    }
}

fn draw_status_card(f: &mut Frame, area: Rect, state: &WelcomeState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme::BORDER))
        .title(Span::styled(" Status ", theme::dim_style()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.detecting {
        f.render_widget(
            widgets::spinner(state.tick, "Checking for daemon\u{2026}"),
            inner,
        );
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Daemon
    if let Some(ref url) = state.daemon_url {
        let suffix = if state.daemon_agents > 0 {
            format!(
                " \u{2022} {} agent{}",
                state.daemon_agents,
                if state.daemon_agents == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        lines.push(Line::from(vec![
            Span::styled(" \u{25cf} ", Style::default().fg(theme::GREEN)),
            Span::styled(
                format!("Daemon {url}"),
                Style::default().fg(theme::TEXT_PRIMARY),
            ),
            Span::styled(suffix, Style::default().fg(theme::GREEN)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" \u{25cb} ", theme::dim_style()),
            Span::styled("No daemon running", theme::dim_style()),
        ]));
    }

    // Provider
    if let Some((provider, _env_var)) = detect_provider() {
        lines.push(Line::from(vec![
            Span::styled(" \u{25cf} ", Style::default().fg(theme::GREEN)),
            Span::styled(
                format!("Provider: {provider}"),
                Style::default().fg(theme::TEXT_PRIMARY),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" \u{25cb} ", Style::default().fg(theme::YELLOW)),
            Span::styled("No API keys", Style::default().fg(theme::YELLOW)),
            Span::styled(" \u{2014} run ", theme::dim_style()),
            Span::styled("librefang init", Style::default().fg(theme::ACCENT)),
        ]));
    }

    // Post-wizard
    if state.setup_just_completed {
        lines.push(Line::from(vec![
            Span::styled(" \u{2714} ", Style::default().fg(theme::GREEN)),
            Span::styled("Setup complete!", Style::default().fg(theme::GREEN)),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_menu(f: &mut Frame, area: Rect, state: &mut WelcomeState) {
    let items: Vec<ListItem> = state
        .menu_items
        .iter()
        .map(|item| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", item.icon),
                    Style::default().fg(theme::ACCENT),
                ),
                Span::styled(
                    format!("{:<22}", item.label),
                    Style::default()
                        .fg(theme::TEXT_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(item.hint, theme::dim_style()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(theme::selected_style())
        .highlight_symbol("\u{25b8} ");

    f.render_stateful_widget(list, area, &mut state.menu);
}
