//! Standalone ratatui mini-wizard: guides users to pick a free LLM provider,
//! opens the registration page, and prompts for API key paste.
//!
//! Launched from `detect_best_provider()` when no API keys are found.

use ratatui::crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::time::{Duration, Instant};

use crate::i18n;
use crate::tui::theme;

// ── Provider metadata ──────────────────────────────────────────────────────

struct FreeProvider {
    name: &'static str,
    display: &'static str,
    env_var: &'static str,
    hint: &'static str,
    register_url: &'static str,
}

const FREE_PROVIDERS: &[FreeProvider] = &[
    FreeProvider {
        name: "groq",
        display: "Groq",
        env_var: "GROQ_API_KEY",
        hint: "free tier, blazing fast inference",
        register_url: "https://console.groq.com/keys",
    },
    FreeProvider {
        name: "gemini",
        display: "Gemini",
        env_var: "GEMINI_API_KEY",
        hint: "free tier, generous quota (Google account)",
        register_url: "https://aistudio.google.com/apikey",
    },
    FreeProvider {
        name: "deepseek",
        display: "DeepSeek",
        env_var: "DEEPSEEK_API_KEY",
        hint: "5M free tokens for new accounts",
        register_url: "https://platform.deepseek.com/api_keys",
    },
];

// ── Result type ────────────────────────────────────────────────────────────

pub enum GuideResult {
    /// User completed setup: (provider, env_var).
    /// The API key is already saved to .env and set in the process environment.
    Completed { provider: String, env_var: String },
    /// User chose to skip / cancelled.
    Skipped,
}

// ── Internal state ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Pick a free provider from the list.
    Select,
    /// Browser opened — waiting for user to paste API key.
    PasteKey,
    /// Testing the key.
    Testing,
    /// Key verified (or unverifiable), auto-advancing.
    Done,
}

struct State {
    phase: Phase,
    list: ListState,
    selected: usize,
    key_input: String,
    key_ok: Option<bool>,
    status_msg: String,
    save_warn: Option<String>,
    test_started: Option<Instant>,
    done_at: Option<Instant>,
}

impl State {
    fn new() -> Self {
        let mut list = ListState::default();
        list.select(Some(0));
        Self {
            phase: Phase::Select,
            list,
            selected: 0,
            key_input: String::new(),
            key_ok: None,
            status_msg: String::new(),
            save_warn: None,
            test_started: None,
            done_at: None,
        }
    }
}

// ── Entry point ────────────────────────────────────────────────────────────

pub fn run() -> GuideResult {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        return GuideResult::Skipped;
    }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::DisableBracketedPaste
        );
        ratatui::restore();
        original_hook(info);
    }));

    // Enable bracketed paste so terminal paste events arrive as Event::Paste
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableBracketedPaste
    );

    let mut terminal = ratatui::init();
    let mut state = State::new();

    let (test_tx, test_rx) = std::sync::mpsc::channel::<bool>();

    let result = loop {
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw failed");

        // Check background key-test result
        if state.phase == Phase::Testing {
            if let Ok(ok) = test_rx.try_recv() {
                state.key_ok = Some(ok);
                state.status_msg = if ok {
                    i18n::t("guide-key-verified")
                } else {
                    i18n::t("guide-test-key-unverified")
                };
                state.phase = Phase::Done;
                state.done_at = Some(Instant::now());
            }
            // Timeout: if test_api_key takes >15s, treat as unverified
            if let Some(done_at) = state.test_started {
                if done_at.elapsed() >= Duration::from_secs(15) && state.phase == Phase::Testing {
                    state.key_ok = Some(false);
                    state.status_msg = i18n::t("guide-test-key-unverified");
                    state.phase = Phase::Done;
                    state.done_at = Some(Instant::now());
                }
            }
        }

        // Auto-advance from Done after 800ms
        if state.phase == Phase::Done {
            if let Some(done_at) = state.done_at {
                if done_at.elapsed() >= Duration::from_millis(800) {
                    let p = &FREE_PROVIDERS[state.selected];
                    break GuideResult::Completed {
                        provider: p.name.to_string(),
                        env_var: p.env_var.to_string(),
                    };
                }
            }
        }

        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read() {
                // Handle bracketed paste (terminal paste event)
                Ok(CtEvent::Paste(text)) => {
                    if state.phase == Phase::PasteKey {
                        // Trim whitespace/newlines that terminals often include
                        state.key_input.push_str(text.trim());
                    }
                }
                Ok(CtEvent::Key(key)) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // Ctrl+C always quits
                    if key.code == KeyCode::Char('c')
                        && key
                            .modifiers
                            .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
                    {
                        break GuideResult::Skipped;
                    }

                    match state.phase {
                        Phase::Select => match key.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('s') => {
                                break GuideResult::Skipped;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = state.list.selected().unwrap_or(0);
                                let next = if i == 0 {
                                    FREE_PROVIDERS.len() - 1
                                } else {
                                    i - 1
                                };
                                state.list.select(Some(next));
                                state.selected = next;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = state.list.selected().unwrap_or(0);
                                let next = (i + 1) % FREE_PROVIDERS.len();
                                state.list.select(Some(next));
                                state.selected = next;
                            }
                            KeyCode::Enter => {
                                let p = &FREE_PROVIDERS[state.selected];
                                crate::open_in_browser(p.register_url);
                                state.phase = Phase::PasteKey;
                            }
                            _ => {}
                        },

                        Phase::PasteKey => match key.code {
                            KeyCode::Esc => {
                                state.key_input.clear();
                                state.phase = Phase::Select;
                            }
                            KeyCode::Enter => {
                                if !state.key_input.is_empty() {
                                    submit_key(&mut state, &test_tx);
                                }
                            }
                            KeyCode::Char(c) => {
                                state.key_input.push(c);
                            }
                            KeyCode::Backspace => {
                                state.key_input.pop();
                            }
                            _ => {}
                        },

                        Phase::Testing | Phase::Done => {
                            // Ignore input while testing/done
                        }
                    }
                }
                _ => {}
            }
        }
    };

    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableBracketedPaste
    );
    ratatui::restore();
    result
}

/// Save the API key and kick off a background verification.
fn submit_key(state: &mut State, test_tx: &std::sync::mpsc::Sender<bool>) {
    let p = &FREE_PROVIDERS[state.selected];
    let save_warn = crate::dotenv::save_env_key(p.env_var, &state.key_input).err();
    std::env::set_var(p.env_var, &state.key_input);
    state.save_warn = save_warn.map(|e| e.to_string());
    state.status_msg = i18n::t("guide-testing-key");
    state.phase = Phase::Testing;
    state.test_started = Some(Instant::now());

    let provider_name = p.name.to_string();
    let env_var = p.env_var.to_string();
    let tx = test_tx.clone();
    std::thread::spawn(move || {
        let ok = crate::test_api_key(&provider_name, &env_var);
        let _ = tx.send(ok);
    });
}

// ── Drawing ────────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, area: Rect, state: &State) {
    // Fill background
    f.render_widget(
        Block::default().style(Style::default().bg(theme::BG_PRIMARY)),
        area,
    );

    let outer = Layout::vertical([
        Constraint::Length(1), // top margin
        Constraint::Min(0),    // content
        Constraint::Length(1), // bottom bar
    ])
    .split(area);

    // Title
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " LibreFang ",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("— {}", i18n::t("guide-title")),
            Style::default().fg(theme::TEXT_SECONDARY),
        ),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(title, outer[0]);

    match state.phase {
        Phase::Select => draw_select(f, outer[1], state),
        Phase::PasteKey => draw_paste_key(f, outer[1], state),
        Phase::Testing | Phase::Done => draw_testing(f, outer[1], state),
    }

    // Bottom help bar
    let help_text = match state.phase {
        Phase::Select => i18n::t("guide-help-select"),
        Phase::PasteKey => i18n::t("guide-help-paste"),
        Phase::Testing | Phase::Done => i18n::t("guide-help-wait"),
    };
    let help = Paragraph::new(Span::styled(
        help_text,
        Style::default().fg(theme::TEXT_TERTIARY),
    ))
    .alignment(Alignment::Center);
    f.render_widget(help, outer[2]);
}

fn draw_select(f: &mut Frame, area: Rect, state: &State) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // top padding
        Constraint::Length(3), // message
        Constraint::Length(1), // gap
        Constraint::Min(0),    // list
    ])
    .split(area);

    // No API keys message
    let msg = Paragraph::new(vec![
        Line::from(Span::styled(
            format!("  {}", i18n::t("hint-no-api-keys")),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  {}", i18n::t("guide-free-providers-title")),
            Style::default().fg(theme::TEXT_SECONDARY),
        )),
    ]);
    f.render_widget(msg, chunks[1]);

    // Provider list
    let items: Vec<ListItem> = FREE_PROVIDERS
        .iter()
        .map(|p| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {}  ", p.display),
                    Style::default()
                        .fg(theme::TEXT_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("— {}", p.hint),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
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
        .highlight_symbol("▸ ");

    let mut list_state = state.list.clone();
    f.render_stateful_widget(list, chunks[3], &mut list_state);
}

fn draw_paste_key(f: &mut Frame, area: Rect, state: &State) {
    let p = &FREE_PROVIDERS[state.selected];

    let chunks = Layout::vertical([
        Constraint::Length(2), // top padding
        Constraint::Length(5), // instructions
        Constraint::Length(1), // gap
        Constraint::Length(3), // input box
        Constraint::Min(0),    // rest
    ])
    .split(area);

    let instructions = Paragraph::new(vec![
        Line::from(Span::styled(
            format!("  {} — {}", p.display, i18n::t("guide-get-free-key")),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("  {}", p.register_url),
            Style::default().fg(theme::BLUE),
        )),
        Line::from(Span::styled(
            format!("  {}", i18n::t("guide-paste-key-hint")),
            Style::default().fg(theme::TEXT_SECONDARY),
        )),
    ]);
    f.render_widget(instructions, chunks[1]);

    // Key input box
    let display_key = if state.key_input.is_empty() {
        format!("  ({})", i18n::t("guide-paste-key-placeholder"))
    } else {
        let chars: Vec<char> = state.key_input.chars().collect();
        let len = chars.len();
        if len <= 8 {
            format!("  {}", "*".repeat(len))
        } else {
            let prefix: String = chars[..4].iter().collect();
            let suffix: String = chars[len - 4..].iter().collect();
            format!("  {}...{}", prefix, suffix)
        }
    };

    let input_style = if state.key_input.is_empty() {
        Style::default().fg(theme::TEXT_TERTIARY)
    } else {
        Style::default().fg(theme::GREEN)
    };

    let input = Paragraph::new(Span::styled(display_key, input_style)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(
                " API Key ",
                Style::default().fg(theme::TEXT_SECONDARY),
            )),
    );
    f.render_widget(input, chunks[3]);
}

fn draw_testing(f: &mut Frame, area: Rect, state: &State) {
    let p = &FREE_PROVIDERS[state.selected];

    let chunks = Layout::vertical([
        Constraint::Length(2), // top padding
        Constraint::Length(6), // status (4 lines + possible warn line)
        Constraint::Min(0),    // rest
    ])
    .split(area);

    let status_color = match state.key_ok {
        Some(true) => theme::GREEN,
        Some(false) => theme::YELLOW,
        None => theme::BLUE,
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!("  {} — {}...", p.display, i18n::t("guide-setting-up")),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("  {}", state.status_msg),
            Style::default().fg(status_color),
        )),
    ];
    if let Some(warn) = &state.save_warn {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ .env save failed: {warn}"),
            Style::default().fg(theme::YELLOW),
        )));
    }
    let status = Paragraph::new(lines);
    f.render_widget(status, chunks[1]);
}
