//! Interactive launcher — lightweight Ratatui one-shot menu.
//!
//! Shown when `librefang` is run with no subcommand in a TTY.
//! Full-width left-aligned layout, adapts for first-time vs returning users.

use ratatui::crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use std::path::PathBuf;
use std::time::Duration;

use crate::tui::theme;

// ── Provider detection ──────────────────────────────────────────────────────

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
];

fn detect_provider() -> Option<(&'static str, &'static str)> {
    for &(var, name) in PROVIDER_ENV_VARS {
        if std::env::var(var).is_ok() {
            return Some((name, var));
        }
    }
    None
}

fn is_first_run() -> bool {
    let of_home = if let Ok(h) = std::env::var("LIBREFANG_HOME") {
        std::path::PathBuf::from(h)
    } else {
        match dirs::home_dir() {
            Some(h) => h.join(".librefang"),
            None => return true,
        }
    };
    !of_home.join("config.toml").exists()
}

fn has_openclaw() -> bool {
    // Quick check: does ~/.openclaw exist?
    dirs::home_dir()
        .map(|h| h.join(".openclaw").exists())
        .unwrap_or(false)
}

fn has_openfang() -> bool {
    dirs::home_dir()
        .map(|h| h.join(".openfang").exists())
        .unwrap_or(false)
}

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LauncherChoice {
    GetStarted,
    Chat,
    Dashboard,
    DesktopApp,
    TerminalUI,
    ShowHelp,
    Quit,
}

struct MenuItem {
    label: &'static str,
    hint: &'static str,
    choice: LauncherChoice,
}

// Menu for first-time users: "Get started" is first and prominent
const MENU_FIRST_RUN: &[MenuItem] = &[
    MenuItem {
        label: "Get started",
        hint: "Providers, API keys, models, migration",
        choice: LauncherChoice::GetStarted,
    },
    MenuItem {
        label: "Chat with an agent",
        hint: "Quick chat in the terminal",
        choice: LauncherChoice::Chat,
    },
    MenuItem {
        label: "Open dashboard",
        hint: "Launch the web UI in your browser",
        choice: LauncherChoice::Dashboard,
    },
    MenuItem {
        label: "Open desktop app",
        hint: "Launch the native desktop app",
        choice: LauncherChoice::DesktopApp,
    },
    MenuItem {
        label: "Launch terminal UI",
        hint: "Full interactive TUI dashboard",
        choice: LauncherChoice::TerminalUI,
    },
    MenuItem {
        label: "Show all commands",
        hint: "Print full --help output",
        choice: LauncherChoice::ShowHelp,
    },
];

// Menu for returning users: action-first, setup at the bottom
const MENU_RETURNING: &[MenuItem] = &[
    MenuItem {
        label: "Chat with an agent",
        hint: "Quick chat in the terminal",
        choice: LauncherChoice::Chat,
    },
    MenuItem {
        label: "Open dashboard",
        hint: "Launch the web UI in your browser",
        choice: LauncherChoice::Dashboard,
    },
    MenuItem {
        label: "Launch terminal UI",
        hint: "Full interactive TUI dashboard",
        choice: LauncherChoice::TerminalUI,
    },
    MenuItem {
        label: "Open desktop app",
        hint: "Launch the native desktop app",
        choice: LauncherChoice::DesktopApp,
    },
    MenuItem {
        label: "Settings",
        hint: "Providers, API keys, models, routing",
        choice: LauncherChoice::GetStarted,
    },
    MenuItem {
        label: "Show all commands",
        hint: "Print full --help output",
        choice: LauncherChoice::ShowHelp,
    },
];

// ── Launcher state ──────────────────────────────────────────────────────────

enum Screen {
    Menu,
    Help {
        lines: Vec<String>,
        scroll: usize,
        /// Cached viewport height from the last render frame; 0 until first draw.
        viewport_height: usize,
    },
}

struct LauncherState {
    list: ListState,
    daemon_url: Option<String>,
    daemon_agents: u64,
    detecting: bool,
    tick: usize,
    first_run: bool,
    openclaw_detected: bool,
    openfang_detected: bool,
    screen: Screen,
}

impl LauncherState {
    fn new() -> Self {
        let first_run = is_first_run();
        let openclaw_detected = first_run && has_openclaw();
        let openfang_detected = first_run && has_openfang();
        let mut list = ListState::default();
        list.select(Some(0));
        Self {
            list,
            daemon_url: None,
            daemon_agents: 0,
            detecting: true,
            tick: 0,
            first_run,
            openclaw_detected,
            openfang_detected,
            screen: Screen::Menu,
        }
    }

    fn menu(&self) -> &'static [MenuItem] {
        if self.first_run {
            MENU_FIRST_RUN
        } else {
            MENU_RETURNING
        }
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run(_config: Option<PathBuf>) -> LauncherChoice {
    let mut terminal = ratatui::init();

    // Panic hook: restore terminal on panic (set AFTER init succeeds)
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ratatui::try_restore();
        original_hook(info);
    }));

    let mut state = LauncherState::new();

    // Spawn background daemon detection (catch_unwind protects against thread panics)
    let (daemon_tx, daemon_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(|| {
            let result = crate::find_daemon();
            let agent_count = result.as_ref().map_or(0, |base| {
                let client = crate::http_client::client_builder()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .ok();
                client
                    .and_then(|c| c.get(format!("{base}/api/agents")).send().ok())
                    .and_then(|r| r.json::<serde_json::Value>().ok())
                    .and_then(|v| v.as_array().map(|a| a.len() as u64))
                    .unwrap_or(0)
            });
            let _ = daemon_tx.send((result, agent_count));
        });
    });

    let choice;

    loop {
        // Check for daemon detection result
        if state.detecting {
            if let Ok((url, agents)) = daemon_rx.try_recv() {
                state.daemon_url = url;
                state.daemon_agents = agents;
                state.detecting = false;
            }
        }

        state.tick = state.tick.wrapping_add(1);

        // Draw (gracefully handle render failures)
        if terminal.draw(|frame| draw(frame, &mut state)).is_err() {
            choice = LauncherChoice::Quit;
            break;
        }

        // Poll for input (50ms = 20fps spinner)
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            if let Ok(CtEvent::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match &mut state.screen {
                    Screen::Help {
                        lines,
                        scroll,
                        viewport_height,
                    } => {
                        let total = lines.len();
                        // Maximum scroll offset: stop when the last line fills the bottom
                        // of the viewport rather than when the last line is at the top.
                        let vh = (*viewport_height).max(1);
                        let max_scroll = total.saturating_sub(vh);
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => {
                                state.screen = Screen::Menu;
                            }
                            KeyCode::Down | KeyCode::Char('j') if *scroll < max_scroll => {
                                *scroll += 1;
                            }
                            KeyCode::Up | KeyCode::Char('k') if *scroll > 0 => {
                                *scroll -= 1;
                            }
                            KeyCode::PageDown => {
                                *scroll = (*scroll + 20).min(max_scroll);
                            }
                            KeyCode::PageUp => {
                                *scroll = scroll.saturating_sub(20);
                            }
                            KeyCode::Home | KeyCode::Char('g') => {
                                *scroll = 0;
                            }
                            KeyCode::End | KeyCode::Char('G') => {
                                *scroll = max_scroll;
                            }
                            _ => {}
                        }
                    }
                    Screen::Menu => {
                        let menu = state.menu();
                        if menu.is_empty() {
                            choice = LauncherChoice::Quit;
                            break;
                        }
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                choice = LauncherChoice::Quit;
                                break;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = state.list.selected().unwrap_or(0);
                                let next = if i == 0 { menu.len() - 1 } else { i - 1 };
                                state.list.select(Some(next));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = state.list.selected().unwrap_or(0);
                                let next = (i + 1) % menu.len();
                                state.list.select(Some(next));
                            }
                            // Number shortcuts: 1-9 jump directly to menu item
                            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                                let idx = (c as usize) - ('1' as usize);
                                if idx < menu.len() {
                                    state.list.select(Some(idx));
                                    let selected = menu[idx].choice;
                                    if selected == LauncherChoice::ShowHelp {
                                        state.screen = Screen::Help {
                                            lines: build_help_lines(),
                                            scroll: 0,
                                            viewport_height: 0,
                                        };
                                    } else {
                                        choice = selected;
                                        break;
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(i) = state.list.selected() {
                                    if i < menu.len() {
                                        let selected = menu[i].choice;
                                        if selected == LauncherChoice::ShowHelp {
                                            state.screen = Screen::Help {
                                                lines: build_help_lines(),
                                                scroll: 0,
                                                viewport_height: 0,
                                            };
                                        } else {
                                            choice = selected;
                                            break;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let _ = ratatui::try_restore();
    choice
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut esc = false;
    for c in s.chars() {
        if c == '\x1b' {
            esc = true;
            continue;
        }
        if esc {
            if c.is_ascii_alphabetic() {
                esc = false;
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn build_help_lines() -> Vec<String> {
    use clap::CommandFactory;
    let mut buf = Vec::new();
    crate::Cli::command()
        .write_long_help(&mut buf)
        .unwrap_or(());
    String::from_utf8_lossy(&buf)
        .lines()
        .map(strip_ansi)
        .collect()
}

// ── Drawing ─────────────────────────────────────────────────────────────────

fn draw(frame: &mut ratatui::Frame, state: &mut LauncherState) {
    match &mut state.screen {
        Screen::Help {
            lines,
            scroll,
            viewport_height,
        } => {
            let vh = draw_help(frame, lines, *scroll);
            *viewport_height = vh;
        }
        Screen::Menu => draw_menu(frame, state),
    }
}

/// Left margin for content alignment.
const MARGIN_LEFT: u16 = 3;

/// Constrain content to a readable area within the terminal.
fn content_area(area: Rect) -> Rect {
    if area.width < 10 || area.height < 5 {
        // Terminal too small — use full area with no margin
        return area;
    }
    let margin = MARGIN_LEFT.min(area.width.saturating_sub(10));
    let w = 80u16.min(area.width.saturating_sub(margin));
    Rect {
        x: area.x.saturating_add(margin),
        y: area.y,
        width: w,
        height: area.height,
    }
}

fn draw_menu(frame: &mut ratatui::Frame, state: &mut LauncherState) {
    let area = frame.area();

    // Fill background
    frame.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(theme::BG_PRIMARY)),
        area,
    );

    let content = content_area(area);
    let version = env!("CARGO_PKG_VERSION");
    let has_provider = detect_provider().is_some();
    let menu = state.menu();

    // Compute dynamic heights
    let header_h: u16 = if state.first_run { 3 } else { 1 }; // welcome text or just title
    let status_h: u16 = if state.detecting {
        1
    } else if has_provider {
        2
    } else {
        3
    };
    let has_migration = state.first_run && (state.openclaw_detected || state.openfang_detected);
    let migration_hint_h: u16 = if has_migration { 2 } else { 0 };
    let menu_h = menu.len() as u16;

    let total_needed = 1 + header_h + 1 + status_h + 1 + menu_h + migration_hint_h + 1;

    // Vertical centering: place content block in the upper-third area
    let top_pad = if area.height > total_needed + 2 {
        ((area.height - total_needed) / 3).max(1)
    } else {
        1
    };

    let chunks = Layout::vertical([
        Constraint::Length(top_pad),          // top space
        Constraint::Length(header_h),         // header / welcome
        Constraint::Length(1),                // separator
        Constraint::Length(status_h),         // status indicators
        Constraint::Length(1),                // separator
        Constraint::Length(menu_h),           // menu items
        Constraint::Length(migration_hint_h), // openclaw migration hint (if any)
        Constraint::Length(1),                // keybind hints
        Constraint::Min(0),                   // remaining space
    ])
    .split(content);

    // ── Header ──────────────────────────────────────────────────────────────
    if state.first_run {
        let header_lines = vec![
            Line::from(vec![
                Span::styled(
                    "LibreFang",
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  v{version}"),
                    Style::default().fg(theme::TEXT_TERTIARY),
                ),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Welcome! Let's get you set up.",
                Style::default().fg(theme::TEXT_PRIMARY),
            )]),
        ];
        frame.render_widget(Paragraph::new(header_lines), chunks[1]);
    } else {
        let header = Line::from(vec![
            Span::styled(
                "LibreFang",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  v{version}"),
                Style::default().fg(theme::TEXT_TERTIARY),
            ),
        ]);
        frame.render_widget(Paragraph::new(header), chunks[1]);
    }

    // ── Separator ───────────────────────────────────────────────────────────
    render_separator(frame, chunks[2]);

    // ── Status block ────────────────────────────────────────────────────────
    if state.detecting {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        let line = Line::from(vec![
            Span::styled(format!("{spinner} "), Style::default().fg(theme::YELLOW)),
            Span::styled("Checking for daemon\u{2026}", theme::dim_style()),
        ]);
        frame.render_widget(Paragraph::new(line), chunks[3]);
    } else {
        let mut lines: Vec<Line> = Vec::new();

        // Daemon status
        if let Some(ref url) = state.daemon_url {
            let agent_suffix = if state.daemon_agents > 0 {
                format!(
                    " ({} agent{})",
                    state.daemon_agents,
                    if state.daemon_agents == 1 { "" } else { "s" }
                )
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    "\u{25cf} ",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("Daemon running at {url}"),
                    Style::default().fg(theme::TEXT_PRIMARY),
                ),
                Span::styled(agent_suffix, Style::default().fg(theme::GREEN)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("\u{25cb} ", theme::dim_style()),
                Span::styled("No daemon running", theme::dim_style()),
            ]));
        }

        // Provider status
        if let Some((provider, env_var)) = detect_provider() {
            lines.push(Line::from(vec![
                Span::styled(
                    "\u{2714} ",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("Provider: {provider}"),
                    Style::default().fg(theme::TEXT_PRIMARY),
                ),
                Span::styled(format!(" ({env_var})"), theme::dim_style()),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("\u{25cb} ", Style::default().fg(theme::YELLOW)),
                Span::styled("No API keys detected", Style::default().fg(theme::YELLOW)),
            ]));
            if !state.first_run {
                lines.push(Line::from(vec![Span::styled(
                    "  Run 'Re-run setup' to configure a provider",
                    theme::hint_style(),
                )]));
            } else {
                lines.push(Line::from(vec![Span::styled(
                    "  Select 'Get started' to configure",
                    theme::hint_style(),
                )]));
            }
        }

        frame.render_widget(Paragraph::new(lines), chunks[3]);
    }

    // ── Separator 2 ─────────────────────────────────────────────────────────
    render_separator(frame, chunks[4]);

    // ── Menu ────────────────────────────────────────────────────────────────
    let items: Vec<ListItem> = menu
        .iter()
        .enumerate()
        .map(|(i, item)| {
            // Highlight "Get started" for first-run users
            let is_primary = state.first_run && i == 0;
            let label_style = if is_primary {
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT_PRIMARY)
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<26}", item.label), label_style),
                Span::styled(item.hint, theme::dim_style()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .bg(theme::BG_HOVER)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{25b8} ");

    frame.render_stateful_widget(list, chunks[5], &mut state.list);

    // ── Migration hint ────────────────────────────────────────────────────────
    if state.first_run && (state.openclaw_detected || state.openfang_detected) {
        let source = match (state.openclaw_detected, state.openfang_detected) {
            (true, true) => "OpenClaw / OpenFang",
            (true, false) => "OpenClaw",
            (false, true) => "OpenFang",
            _ => unreachable!(),
        };
        let hint_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("\u{2192} ", Style::default().fg(theme::BLUE)),
                Span::styled(
                    format!("Coming from {source}? "),
                    Style::default().fg(theme::BLUE),
                ),
                Span::styled(
                    "'Get started' includes automatic migration.",
                    theme::hint_style(),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(hint_lines), chunks[6]);
    }

    // ── Keybind hints ───────────────────────────────────────────────────────
    let hints = Line::from(vec![Span::styled(
        "\u{2191}\u{2193}/jk navigate  1-9 quick select  enter confirm  q quit",
        theme::hint_style(),
    )]);
    frame.render_widget(Paragraph::new(hints), chunks[7]);
}

fn render_separator(frame: &mut ratatui::Frame, area: Rect) {
    let w = (area.width as usize).min(60);
    let line = Line::from(vec![Span::styled(
        "\u{2500}".repeat(w),
        Style::default().fg(theme::BORDER),
    )]);
    frame.render_widget(Paragraph::new(line), area);
}

// ── Help screen ─────────────────────────────────────────────────────────────

/// Renders the help screen and returns the viewport height (in lines) so the
/// caller can update `Screen::Help::viewport_height` for scroll-bound clamping.
fn draw_help(frame: &mut ratatui::Frame, lines: &[String], scroll: usize) -> usize {
    use ratatui::widgets::Block;

    let area = frame.area();

    frame.render_widget(
        Block::default().style(Style::default().bg(theme::BG_PRIMARY)),
        area,
    );

    // Title bar (1 line)
    let title_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let content_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width.saturating_sub(1), // leave 1 col for scrollbar
        height: area.height.saturating_sub(2),
    };
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let scrollbar_area = Rect {
        x: area.x + area.width.saturating_sub(1),
        y: area.y + 1,
        width: 1,
        height: area.height.saturating_sub(2),
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "All commands",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  — q/Esc to go back", theme::dim_style()),
        ])),
        title_area,
    );

    let visible_h = content_area.height as usize;
    let display_lines: Vec<Line> = lines
        .iter()
        .skip(scroll)
        .take(visible_h)
        .map(|l| {
            Line::from(Span::styled(
                l.as_str(),
                Style::default().fg(theme::TEXT_PRIMARY),
            ))
        })
        .collect();

    frame.render_widget(Paragraph::new(display_lines), content_area);

    // Scrollbar — content_length is the total number of lines; viewport_content_length
    // is the number of visible lines so ratatui sizes the thumb correctly.
    let total = lines.len();
    let mut sb_state = ScrollbarState::new(total)
        .viewport_content_length(visible_h)
        .position(scroll);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        scrollbar_area,
        &mut sb_state,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "\u{2191}\u{2193}/jk scroll  PgUp/PgDn  g/G top/bottom  q back",
            theme::hint_style(),
        ))),
        hint_area,
    );

    visible_h
}

// ── Desktop app launcher ────────────────────────────────────────────────────

pub fn launch_desktop_app() {
    use crate::desktop_install;

    if let Some(path) = desktop_install::find_desktop_binary() {
        desktop_install::launch(&path);
        return;
    }

    // Not installed — offer to download
    if let Some(installed) = desktop_install::prompt_and_install() {
        desktop_install::launch(&installed);
    }
}
