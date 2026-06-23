//! Memory screen: per-agent KV store browser and editor.

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
pub struct KvPair {
    pub key: String,
    pub value: String,
}

#[derive(Clone)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
pub enum MemorySub {
    AgentSelect,
    KvBrowser,
    EditKey,
    AddKey,
}

#[derive(Clone, PartialEq, Eq)]
pub enum EditField {
    Key,
    Value,
}

pub struct MemoryState {
    pub sub: MemorySub,
    pub agents: Vec<AgentEntry>,
    pub selected_agent: Option<AgentEntry>,
    pub kv_pairs: Vec<KvPair>,
    pub agent_list_state: ListState,
    pub kv_list_state: ListState,
    pub key_buf: String,
    pub value_buf: String,
    pub edit_field: EditField,
    pub loading: bool,
    pub tick: usize,
    pub confirm_delete: bool,
    pub status_msg: String,
}

pub enum MemoryUIAction {
    Continue,
    LoadAgents,
    LoadKv(String),
    SaveKv {
        agent_id: String,
        key: String,
        value: String,
    },
    DeleteKv {
        agent_id: String,
        key: String,
    },
}

impl MemoryState {
    pub fn new() -> Self {
        Self {
            sub: MemorySub::AgentSelect,
            agents: Vec::new(),
            selected_agent: None,
            kv_pairs: Vec::new(),
            agent_list_state: ListState::default(),
            kv_list_state: ListState::default(),
            key_buf: String::new(),
            value_buf: String::new(),
            edit_field: EditField::Key,
            loading: false,
            tick: 0,
            confirm_delete: false,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> MemoryUIAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return MemoryUIAction::Continue;
        }
        match self.sub {
            MemorySub::AgentSelect => self.handle_agent_select(key),
            MemorySub::KvBrowser => self.handle_kv_browser(key),
            MemorySub::EditKey | MemorySub::AddKey => self.handle_edit(key),
        }
    }

    fn handle_agent_select(&mut self, key: KeyEvent) -> MemoryUIAction {
        let total = self.agents.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.agent_list_state.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.agent_list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.agent_list_state.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.agent_list_state.select(Some(next));
            }
            KeyCode::Enter => {
                if let Some(sel) = self.agent_list_state.selected() {
                    if sel < self.agents.len() {
                        let agent = self.agents[sel].clone();
                        let id = agent.id.clone();
                        self.selected_agent = Some(agent);
                        self.sub = MemorySub::KvBrowser;
                        self.loading = true;
                        return MemoryUIAction::LoadKv(id);
                    }
                }
            }
            KeyCode::Char('r') => return MemoryUIAction::LoadAgents,
            _ => {}
        }
        MemoryUIAction::Continue
    }

    fn handle_kv_browser(&mut self, key: KeyEvent) -> MemoryUIAction {
        if self.confirm_delete {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_delete = false;
                    if let (Some(agent), Some(sel)) =
                        (&self.selected_agent, self.kv_list_state.selected())
                    {
                        if sel < self.kv_pairs.len() {
                            return MemoryUIAction::DeleteKv {
                                agent_id: agent.id.clone(),
                                key: self.kv_pairs[sel].key.clone(),
                            };
                        }
                    }
                }
                _ => self.confirm_delete = false,
            }
            return MemoryUIAction::Continue;
        }

        let total = self.kv_pairs.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = MemorySub::AgentSelect;
                self.kv_pairs.clear();
                self.selected_agent = None;
            }
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.kv_list_state.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.kv_list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.kv_list_state.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.kv_list_state.select(Some(next));
            }
            KeyCode::Char('a') => {
                self.sub = MemorySub::AddKey;
                self.key_buf.clear();
                self.value_buf.clear();
                self.edit_field = EditField::Key;
            }
            KeyCode::Char('e') => {
                if let Some(sel) = self.kv_list_state.selected() {
                    if sel < self.kv_pairs.len() {
                        self.key_buf = self.kv_pairs[sel].key.clone();
                        self.value_buf = self.kv_pairs[sel].value.clone();
                        self.edit_field = EditField::Value;
                        self.sub = MemorySub::EditKey;
                    }
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if self.kv_list_state.selected().is_some() => {
                self.confirm_delete = true;
            }
            KeyCode::Char('r') if self.selected_agent.is_some() => {
                if let Some(agent) = &self.selected_agent {
                    self.loading = true;
                    return MemoryUIAction::LoadKv(agent.id.clone());
                }
            }
            _ => {}
        }
        MemoryUIAction::Continue
    }

    fn handle_edit(&mut self, key: KeyEvent) -> MemoryUIAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = MemorySub::KvBrowser;
            }
            KeyCode::Tab => {
                self.edit_field = match self.edit_field {
                    EditField::Key => EditField::Value,
                    EditField::Value => EditField::Key,
                };
            }
            KeyCode::Enter => {
                if !self.key_buf.is_empty() {
                    if let Some(agent) = &self.selected_agent {
                        let action = MemoryUIAction::SaveKv {
                            agent_id: agent.id.clone(),
                            key: self.key_buf.clone(),
                            value: self.value_buf.clone(),
                        };
                        self.sub = MemorySub::KvBrowser;
                        return action;
                    }
                }
                self.sub = MemorySub::KvBrowser;
            }
            KeyCode::Backspace => match self.edit_field {
                EditField::Key if self.sub == MemorySub::AddKey => {
                    self.key_buf.pop();
                }
                EditField::Value => {
                    self.value_buf.pop();
                }
                _ => {}
            },
            KeyCode::Char(c) => match self.edit_field {
                EditField::Key if self.sub == MemorySub::AddKey => self.key_buf.push(c),
                EditField::Value => self.value_buf.push(c),
                _ => {}
            },
            _ => {}
        }
        MemoryUIAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut MemoryState) {
    let title = format!("□ {}", crate::i18n::t("tui-memory-title-screen"));
    let inner = widgets::render_screen_block(f, area, &title);

    match state.sub {
        MemorySub::AgentSelect => draw_agent_select(f, inner, state),
        MemorySub::KvBrowser => draw_kv_browser(f, inner, state),
        MemorySub::EditKey | MemorySub::AddKey => draw_edit(f, inner, state),
    }
}

fn draw_agent_select(f: &mut Frame, area: Rect, state: &mut MemoryState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![Span::styled(
                crate::i18n::t("tui-memory-label-select-agent"),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  ", theme::table_header()),
                Span::styled(
                    format!("{:<20}", crate::i18n::t("tui-memory-header-agent-name")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    crate::i18n::t("tui-memory-header-id"),
                    theme::table_header(),
                ),
            ]),
        ]),
        chunks[0],
    );

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-memory-loading-agents")),
            chunks[1],
        );
    } else if state.agents.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-memory-empty-agents")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .agents
            .iter()
            .map(|a| {
                let id_short = if a.id.len() > 12 {
                    format!("{}…", librefang_types::truncate_str(&a.id, 12))
                } else {
                    a.id.clone()
                };
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        format!("{:<20}", widgets::truncate(&a.name, 19)),
                        Style::default().fg(theme::TEXT_PRIMARY),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(id_short, Style::default().fg(theme::TEXT_SECONDARY)),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.agent_list_state);
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-memory-hints-agent-select")),
        chunks[2],
    );
}

fn draw_kv_browser(f: &mut Frame, area: Rect, state: &mut MemoryState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    let agent_name = state
        .selected_agent
        .as_ref()
        .map(|a| a.name.as_str())
        .unwrap_or("?");

    let count_str = state.kv_pairs.len().to_string();

    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!("  {agent_name}"),
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    crate::i18n::t_args("tui-memory-pairs-count", &[("count", &count_str)]),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ]),
            Line::from(vec![
                Span::styled("  ", theme::table_header()),
                Span::styled(
                    format!("{:<24}", crate::i18n::t("tui-memory-header-key")),
                    theme::table_header(),
                ),
                Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                Span::styled(
                    crate::i18n::t("tui-memory-header-value"),
                    theme::table_header(),
                ),
            ]),
        ]),
        chunks[0],
    );

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-memory-loading")),
            chunks[1],
        );
    } else if state.kv_pairs.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-memory-empty-kv")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .kv_pairs
            .iter()
            .map(|kv| {
                let val_display = if kv.value.len() > 40 {
                    format!("{}…", librefang_types::truncate_str(&kv.value, 39))
                } else {
                    kv.value.clone()
                };
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        format!("{:<24}", widgets::truncate(&kv.key, 23)),
                        Style::default().fg(theme::YELLOW),
                    ),
                    Span::styled(" │ ", Style::default().fg(theme::BORDER)),
                    Span::styled(val_display, Style::default().fg(theme::TEXT_SECONDARY)),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.kv_list_state);
    }

    f.render_widget(
        widgets::confirm_or_status_or_hint(
            state.confirm_delete,
            &crate::i18n::t("tui-memory-confirm-delete"),
            &state.status_msg,
            &crate::i18n::t("tui-memory-hints-kv-browser"),
        ),
        chunks[2],
    );
}

fn draw_edit(f: &mut Frame, area: Rect, state: &MemoryState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    let title = if state.sub == MemorySub::AddKey {
        crate::i18n::t("tui-memory-title-add")
    } else {
        crate::i18n::t("tui-memory-title-edit")
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {title}"),
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        )])),
        chunks[0],
    );

    // Key field
    let key_active = state.edit_field == EditField::Key && state.sub == MemorySub::AddKey;
    let key_label_style = if key_active {
        Style::default().fg(theme::ACCENT)
    } else {
        theme::dim_style()
    };
    let key_indicator = if key_active { "●" } else { "○" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {key_indicator} "), key_label_style),
            Span::styled(crate::i18n::t("tui-memory-field-key"), key_label_style),
        ])),
        chunks[2],
    );
    let key_display = if state.key_buf.is_empty() {
        crate::i18n::t("tui-memory-placeholder-key")
    } else {
        state.key_buf.clone()
    };
    let key_style = if state.key_buf.is_empty() {
        theme::dim_style()
    } else {
        theme::input_style()
    };
    let mut key_spans = vec![Span::raw("    "), Span::styled(key_display, key_style)];
    if key_active {
        key_spans.push(Span::styled(
            "█",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(key_spans)), chunks[3]);

    // Value field
    let val_active = state.edit_field == EditField::Value;
    let val_label_style = if val_active {
        Style::default().fg(theme::ACCENT)
    } else {
        theme::dim_style()
    };
    let val_indicator = if val_active { "●" } else { "○" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {val_indicator} "), val_label_style),
            Span::styled(crate::i18n::t("tui-memory-field-value"), val_label_style),
        ])),
        chunks[4],
    );
    let val_display = if state.value_buf.is_empty() {
        crate::i18n::t("tui-memory-placeholder-value")
    } else {
        state.value_buf.clone()
    };
    let val_style = if state.value_buf.is_empty() {
        theme::dim_style()
    } else {
        theme::input_style()
    };
    let mut val_spans = vec![Span::raw("    "), Span::styled(val_display, val_style)];
    if val_active {
        val_spans.push(Span::styled(
            "█",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(val_spans)), chunks[5]);

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-memory-hints-edit")),
        chunks[6],
    );
}
