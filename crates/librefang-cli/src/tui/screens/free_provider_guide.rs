//! Standalone ratatui mini-wizard: guides users to pick a free LLM provider,
//! opens the registration page, and prompts for API key paste.
//!
//! Launched from `detect_best_provider()` when no API keys are found.
//! Visual style matches `init_wizard` (left-aligned content, separator, hints).

use ratatui::crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::time::{Duration, Instant};

use crate::i18n;
use crate::tui::theme;
use crate::tui::widgets;
use librefang_extensions::dotenv;

// ── Constants ─────────────────────────────────────────────────────────────

const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const DONE_DELAY: Duration = Duration::from_millis(800);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const CONTENT_MAX_WIDTH: u16 = 72;
const CONTENT_MARGIN: u16 = 3;

// ── Provider metadata ─────────────────────────────────────────────────────

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

// ── Public types ──────────────────────────────────────────────────────────

pub enum GuideResult {
    /// User completed setup. Key is already saved to `.env` and `std::env`.
    Completed { provider: String, env_var: String },
    /// User skipped or cancelled.
    Skipped,
}

// ── Internal state ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Select,
    PasteKey,
    Testing,
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

    fn provider(&self) -> &'static FreeProvider {
        &FREE_PROVIDERS[self.selected]
    }

    fn move_selection(&mut self, delta: isize) {
        let len = FREE_PROVIDERS.len();
        let cur = self.list.selected().unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(len as isize) as usize;
        self.list.select(Some(next));
        self.selected = next;
    }
}

// ── Terminal lifecycle helpers ─────────────────────────────────────────────

fn enable_paste() {
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableBracketedPaste
    );
}

fn disable_paste() {
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableBracketedPaste
    );
}

// ── Entry point ───────────────────────────────────────────────────────────

pub fn run() -> GuideResult {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        return GuideResult::Skipped;
    }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_paste();
        ratatui::restore();
        original_hook(info);
    }));

    enable_paste();
    let mut terminal = ratatui::init();
    let mut state = State::new();
    let (test_tx, test_rx) = std::sync::mpsc::channel::<bool>();

    let result = loop {
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw failed");

        // ── Background task polling ──
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
            if let Some(t) = state.test_started {
                if t.elapsed() >= TEST_TIMEOUT && state.phase == Phase::Testing {
                    state.key_ok = Some(false);
                    state.status_msg = i18n::t("guide-test-key-unverified");
                    state.phase = Phase::Done;
                    state.done_at = Some(Instant::now());
                }
            }
        }

        // ── Auto-advance after result display ──
        if state.phase == Phase::Done {
            if let Some(t) = state.done_at {
                if t.elapsed() >= DONE_DELAY {
                    let p = state.provider();
                    // Ensure the env var is set in-process so the daemon
                    // picks it up on boot (covers both verified and unverified).
                    std::env::set_var(p.env_var, &state.key_input);
                    break GuideResult::Completed {
                        provider: p.name.to_string(),
                        env_var: p.env_var.to_string(),
                    };
                }
            }
        }

        // ── Event handling ──
        if !event::poll(POLL_INTERVAL).unwrap_or(false) {
            continue;
        }
        match event::read() {
            Ok(CtEvent::Paste(text)) if state.phase == Phase::PasteKey => {
                state.key_input.push_str(text.trim());
            }
            Ok(CtEvent::Key(key)) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break GuideResult::Skipped;
                }
                if handle_key(&mut state, key.code, &test_tx) {
                    break GuideResult::Skipped;
                }
            }
            _ => {}
        }
    };

    disable_paste();
    ratatui::restore();
    result
}

// ── Key handling ──────────────────────────────────────────────────────────

/// Returns `true` if the user wants to quit.
fn handle_key(state: &mut State, code: KeyCode, test_tx: &std::sync::mpsc::Sender<bool>) -> bool {
    match state.phase {
        Phase::Select => match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('s') => return true,
            KeyCode::Up | KeyCode::Char('k') => state.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => state.move_selection(1),
            KeyCode::Enter => {
                crate::open_in_browser(state.provider().register_url);
                state.phase = Phase::PasteKey;
            }
            _ => {}
        },
        Phase::PasteKey => match code {
            KeyCode::Esc => {
                state.key_input.clear();
                state.phase = Phase::Select;
            }
            KeyCode::Enter if !state.key_input.is_empty() => {
                submit_key(state, test_tx);
            }
            KeyCode::Char(c) => state.key_input.push(c),
            KeyCode::Backspace => {
                state.key_input.pop();
            }
            _ => {}
        },
        Phase::Testing | Phase::Done => {}
    }
    false
}

/// Save the key to `.env`, set it in-process, and kick off background verification.
fn submit_key(state: &mut State, test_tx: &std::sync::mpsc::Sender<bool>) {
    let p = state.provider();
    state.save_warn = dotenv::save_env_key(p.env_var, &state.key_input)
        .err()
        .map(|e| e.to_string());
    state.status_msg = i18n::t("guide-testing-key");
    state.phase = Phase::Testing;
    state.test_started = Some(Instant::now());

    let name = p.name.to_string();
    let key = state.key_input.clone();
    let var = p.env_var.to_string();
    let tx = test_tx.clone();
    std::thread::spawn(move || {
        let ok = crate::test_api_key(&name, &key);
        if ok {
            // Only set env var after successful verification so the daemon
            // picks it up on boot.  set_var is called from the spawned thread
            // after the test completes — no concurrent env access.
            std::env::set_var(&var, &key);
        }
        let _ = tx.send(ok);
    });
}

// ── Drawing ───────────────────────────────────────────────────────────────

/// Compute a left-aligned content area matching init_wizard's layout.
fn content_area(area: Rect) -> Rect {
    if area.width < 10 || area.height < 5 {
        return area;
    }
    let margin = CONTENT_MARGIN.min(area.width.saturating_sub(10));
    let w = CONTENT_MAX_WIDTH.min(area.width.saturating_sub(margin));
    Rect {
        x: area.x.saturating_add(margin),
        y: area.y,
        width: w,
        height: area.height,
    }
}

fn draw(f: &mut Frame, area: Rect, state: &State) {
    f.render_widget(
        Block::default().style(Style::default().bg(theme::BG_PRIMARY)),
        area,
    );

    let content = content_area(area);
    let chunks = Layout::vertical([
        Constraint::Length(1), // top pad
        Constraint::Length(1), // header
        Constraint::Length(1), // separator
        Constraint::Min(1),    // step content
        Constraint::Length(1), // hint bar
    ])
    .split(content);

    // ── Header ──
    let header = Line::from(vec![
        Span::styled("LibreFang", theme::title_style()),
        Span::styled(format!(" — {}", i18n::t("guide-title")), theme::dim_style()),
    ]);
    f.render_widget(Paragraph::new(header), chunks[1]);

    // ── Separator ──
    f.render_widget(widgets::separator(content.width.min(60)), chunks[2]);

    // ── Phase content ──
    match state.phase {
        Phase::Select => draw_select(f, chunks[3], state),
        Phase::PasteKey => draw_paste_key(f, chunks[3], state),
        Phase::Testing | Phase::Done => draw_testing(f, chunks[3], state),
    }

    // ── Hint bar ──
    let hint = match state.phase {
        Phase::Select => i18n::t("guide-help-select"),
        Phase::PasteKey => i18n::t("guide-help-paste"),
        Phase::Testing | Phase::Done => i18n::t("guide-help-wait"),
    };
    f.render_widget(widgets::hint_bar(&hint), chunks[4]);
}

fn draw_select(f: &mut Frame, area: Rect, state: &State) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // blank
        Constraint::Length(2), // message
        Constraint::Length(1), // gap
        Constraint::Min(0),    // list
    ])
    .split(area);

    let msg = Paragraph::new(vec![
        Line::from(Span::styled(
            format!("  {}", i18n::t("hint-no-api-keys")),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  {}", i18n::t("guide-free-providers-title")),
            theme::dim_style(),
        )),
    ]);
    f.render_widget(msg, chunks[1]);

    let items: Vec<ListItem> = FREE_PROVIDERS
        .iter()
        .map(|p| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {}  ", p.display), theme::title_style()),
                Span::styled(format!("— {}", p.hint), theme::dim_style()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(theme::selected_style().add_modifier(Modifier::BOLD))
        .highlight_symbol("▸ ");

    let mut ls = state.list;
    f.render_stateful_widget(list, chunks[3], &mut ls);
}

fn draw_paste_key(f: &mut Frame, area: Rect, state: &State) {
    let p = state.provider();

    let chunks = Layout::vertical([
        Constraint::Length(1), // blank
        Constraint::Length(4), // instructions
        Constraint::Length(1), // gap
        Constraint::Length(3), // input
        Constraint::Min(0),
    ])
    .split(area);

    let instructions = Paragraph::new(vec![
        Line::from(Span::styled(
            format!("  {} — {}", p.display, i18n::t("guide-get-free-key")),
            theme::title_style(),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("  {}", p.register_url),
            Style::default().fg(theme::BLUE),
        )),
        Line::from(Span::styled(
            format!("  {}", i18n::t("guide-paste-key-hint")),
            theme::dim_style(),
        )),
    ]);
    f.render_widget(instructions, chunks[1]);

    let display_key = mask_key(&state.key_input);
    let style = if state.key_input.is_empty() {
        Style::default().fg(theme::TEXT_TERTIARY)
    } else {
        Style::default().fg(theme::GREEN)
    };
    let input = Paragraph::new(Span::styled(display_key, style)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_set(ratatui::symbols::border::ROUNDED)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(" API Key ", theme::dim_style())),
    );
    f.render_widget(input, chunks[3]);
}

fn draw_testing(f: &mut Frame, area: Rect, state: &State) {
    let p = state.provider();

    let chunks = Layout::vertical([
        Constraint::Length(1), // blank
        Constraint::Length(5), // status block
        Constraint::Min(0),
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
            theme::title_style(),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("  {}", state.status_msg),
            Style::default().fg(status_color),
        )),
    ];
    if let Some(warn) = &state.save_warn {
        lines.push(Line::from(Span::styled(
            format!("  \u{26a0} .env: {warn}"),
            Style::default().fg(theme::YELLOW),
        )));
    }
    f.render_widget(Paragraph::new(lines), chunks[1]);
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Show masked key: `sk-a...xyz9` for long keys, `****` for short ones.
fn mask_key(key: &str) -> String {
    if key.is_empty() {
        return format!("  ({})", i18n::t("guide-paste-key-placeholder"));
    }
    let chars: Vec<char> = key.chars().collect();
    let len = chars.len();
    if len <= 8 {
        format!("  {}", "*".repeat(len))
    } else {
        let prefix: String = chars[..4].iter().collect();
        let suffix: String = chars[len - 4..].iter().collect();
        format!("  {}...{}", prefix, suffix)
    }
}
