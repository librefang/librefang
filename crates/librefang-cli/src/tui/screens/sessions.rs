//! Sessions screen: browse agent sessions, open in chat, delete.

use crate::tui::{theme, widgets};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct SessionInfo {
    pub id: String,
    pub agent_name: String,
    pub agent_id: String,
    pub message_count: u64,
    pub created: String,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct SessionsState {
    pub sessions: Vec<SessionInfo>,
    pub filtered: Vec<usize>,
    pub list_state: ratatui::widgets::ListState,
    pub search_buf: String,
    pub search_mode: bool,
    pub loading: bool,
    pub tick: usize,
    pub confirm_delete: bool,
    pub status_msg: String,
}

pub enum SessionsAction {
    Continue,
    Refresh,
    OpenInChat {
        agent_id: String,
        agent_name: String,
    },
    DeleteSession(String),
}

impl SessionsState {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            filtered: Vec::new(),
            list_state: ratatui::widgets::ListState::default(),
            search_buf: String::new(),
            search_mode: false,
            loading: false,
            tick: 0,
            confirm_delete: false,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn refilter(&mut self) {
        if self.search_buf.is_empty() {
            self.filtered = (0..self.sessions.len()).collect();
        } else {
            let q = self.search_buf.to_lowercase();
            self.filtered = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| s.agent_name.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SessionsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SessionsAction::Continue;
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
            return SessionsAction::Continue;
        }

        if self.confirm_delete {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_delete = false;
                    if let Some(sel) = self.list_state.selected() {
                        if let Some(&idx) = self.filtered.get(sel) {
                            let id = self.sessions[idx].id.clone();
                            return SessionsAction::DeleteSession(id);
                        }
                    }
                }
                _ => {
                    self.confirm_delete = false;
                }
            }
            return SessionsAction::Continue;
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
            KeyCode::Enter => {
                if let Some(sel) = self.list_state.selected() {
                    if let Some(&idx) = self.filtered.get(sel) {
                        let s = &self.sessions[idx];
                        return SessionsAction::OpenInChat {
                            agent_id: s.agent_id.clone(),
                            agent_name: s.agent_name.clone(),
                        };
                    }
                }
            }
            KeyCode::Char('d') => {
                if self.list_state.selected().is_some() {
                    self.confirm_delete = true;
                }
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_buf.clear();
            }
            KeyCode::Char('r') => return SessionsAction::Refresh,
            _ => {}
        }
        SessionsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SessionsState) {
    let inner = widgets::render_screen_block(f, area, "Sessions");

    let (header, content, hints) = widgets::layout_hch(inner, 2);

    // ── Header / search bar ──
    if state.search_mode {
        f.render_widget(widgets::search_input(&state.search_buf), header);
    } else {
        let search_hint = if state.search_buf.is_empty() {
            String::new()
        } else {
            format!("  (filter: \"{}\")", state.search_buf)
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(
                        "  {:<20} {:<16} {:<8} {}",
                        "Agent", "Session ID", "Msgs", "Created"
                    ),
                    theme::table_header(),
                ),
                Span::styled(search_hint, theme::dim_style()),
            ])),
            header,
        );
    }

    // ── List ──
    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, "Loading sessions\u{2026}"),
            content,
        );
    } else if state.filtered.is_empty() {
        f.render_widget(widgets::empty_state("No sessions found."), content);
    } else {
        let items: Vec<ListItem> = state
            .filtered
            .iter()
            .map(|&idx| {
                let s = &state.sessions[idx];
                let id_short = if s.id.len() > 12 {
                    format!("{}\u{2026}", librefang_types::truncate_str(&s.id, 12))
                } else {
                    s.id.clone()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<20}", widgets::truncate(&s.agent_name, 19)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(format!(" {:<16}", id_short), theme::dim_style()),
                    Span::styled(
                        format!(" {:<8}", s.message_count),
                        Style::default().fg(theme::GREEN),
                    ),
                    Span::styled(format!(" {}", s.created), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, content, &mut state.list_state);
    }

    // ── Hints / status ──
    f.render_widget(
        widgets::confirm_or_status_or_hint(
            state.confirm_delete,
            "  Delete this session? [y] Yes  [any] Cancel",
            &state.status_msg,
            "  [\u{2191}\u{2193}] Navigate  [Enter] Open in Chat  [d] Delete  [/] Search  [r] Refresh",
        ),
        hints,
    );
}
