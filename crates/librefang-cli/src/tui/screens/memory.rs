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

/// What `GET /api/memory/config` reports, in the shape the screen shows it.
///
/// `extraction_model` is deliberately not stored alone: unset means "inherit
/// the kernel default", so on its own it answers "nobody chose one" rather
/// than "this one is doing the work". The resolved name and its provenance
/// are what an operator actually needs — a slow model inherited here runs
/// after every reply and delays every answer.
#[derive(Clone, Default)]
pub struct MemoryConfigView {
    pub embedding_provider: String,
    pub embedding_model: String,
    pub auto_memorize: bool,
    pub auto_retrieve: bool,
    /// The model extraction actually runs on, chosen or inherited.
    pub effective_extraction_model: String,
    /// True when nobody picked it and it fell through to `[default_model]`.
    pub extraction_model_inherited: bool,
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
    Config,
}

/// The rows of the config panel that can be changed.
///
/// Kept as an explicit enum rather than an index so adding a row cannot
/// silently shift what a keypress edits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    AutoMemorize,
    AutoRetrieve,
    ExtractionModel,
}

impl ConfigField {
    const ORDER: [ConfigField; 3] = [
        ConfigField::AutoMemorize,
        ConfigField::AutoRetrieve,
        ConfigField::ExtractionModel,
    ];

    fn next(self) -> Self {
        let i = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }

    fn prev(self) -> Self {
        let i = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }
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
    pub config: Option<MemoryConfigView>,
    pub config_field: ConfigField,
    /// Draft of the extraction model while it is being typed.
    pub config_model_buf: String,
    pub config_editing_model: bool,
    /// Set once anything is changed, cleared on save — so the panel can say
    /// there is unsaved work instead of losing it silently on Esc.
    pub config_dirty: bool,
}

#[derive(Debug)]
pub enum MemoryUIAction {
    Continue,
    LoadAgents,
    LoadConfig,
    SaveConfig {
        auto_memorize: bool,
        auto_retrieve: bool,
        extraction_model: String,
    },
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
            config: None,
            config_field: ConfigField::AutoMemorize,
            config_model_buf: String::new(),
            config_editing_model: false,
            config_dirty: false,
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
            MemorySub::Config => self.handle_config(key),
        }
    }

    fn handle_config(&mut self, key: KeyEvent) -> MemoryUIAction {
        // Typing the model name swallows the navigation keys, or every `r` in
        // a model id would trigger a refresh and every `j` would move a row.
        if self.config_editing_model {
            match key.code {
                KeyCode::Esc => {
                    self.config_editing_model = false;
                    if let Some(cfg) = &self.config {
                        self.config_model_buf = cfg.effective_extraction_model.clone();
                    }
                }
                KeyCode::Enter => {
                    self.config_editing_model = false;
                    if let Some(cfg) = &mut self.config {
                        cfg.effective_extraction_model = self.config_model_buf.clone();
                        cfg.extraction_model_inherited = false;
                    }
                    self.config_dirty = true;
                }
                KeyCode::Backspace => {
                    self.config_model_buf.pop();
                }
                KeyCode::Char(c) => self.config_model_buf.push(c),
                _ => {}
            }
            return MemoryUIAction::Continue;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.config_field = self.config_field.prev(),
            KeyCode::Down | KeyCode::Char('j') => self.config_field = self.config_field.next(),
            KeyCode::Char(' ') | KeyCode::Enter => match self.config_field {
                ConfigField::AutoMemorize => {
                    if let Some(cfg) = &mut self.config {
                        cfg.auto_memorize = !cfg.auto_memorize;
                        self.config_dirty = true;
                    }
                }
                ConfigField::AutoRetrieve => {
                    if let Some(cfg) = &mut self.config {
                        cfg.auto_retrieve = !cfg.auto_retrieve;
                        self.config_dirty = true;
                    }
                }
                ConfigField::ExtractionModel => {
                    if let Some(cfg) = &self.config {
                        self.config_model_buf = cfg.effective_extraction_model.clone();
                        self.config_editing_model = true;
                    }
                }
            },
            KeyCode::Char('s') => {
                if let Some(cfg) = &self.config {
                    let action = MemoryUIAction::SaveConfig {
                        auto_memorize: cfg.auto_memorize,
                        auto_retrieve: cfg.auto_retrieve,
                        extraction_model: cfg.effective_extraction_model.clone(),
                    };
                    // `config_dirty` stays set until the daemon confirms the
                    // write — see `apply_save_result`.
                    self.status_msg = crate::i18n::t("tui-memory-config-saving");
                    return action;
                }
            }
            KeyCode::Char('r') => return MemoryUIAction::LoadConfig,
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = MemorySub::AgentSelect;
                self.config_dirty = false;
            }
            _ => {}
        }
        MemoryUIAction::Continue
    }

    /// Fold the outcome of a config save back into the panel.
    ///
    /// The unsaved-changes marker is cleared here and nowhere else: it is the
    /// only thing telling the operator their edits are still pending, so
    /// dropping it on the keypress turns a failed PATCH into "everything is
    /// saved" with a transient error beside it.
    pub fn apply_save_result(&mut self, result: Result<(), crate::tui::event::FetchFailure>) {
        self.status_msg = match result {
            Ok(()) => {
                self.config_dirty = false;
                crate::i18n::t("tui-memory-config-saved")
            }
            Err(crate::tui::event::FetchFailure::RequiresDaemon) => {
                crate::i18n::t("tui-memory-config-requires-daemon")
            }
            Err(crate::tui::event::FetchFailure::Error(reason)) => {
                crate::i18n::t_args("tui-memory-config-save-failed", &[("error", &reason)])
            }
        };
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
            KeyCode::Char('c') => {
                self.sub = MemorySub::Config;
                self.loading = self.config.is_none();
                return MemoryUIAction::LoadConfig;
            }
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
        MemorySub::Config => draw_config(f, inner, state),
    }
}

fn draw_config(f: &mut Frame, area: Rect, state: &MemoryState) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);

    let mut lines: Vec<Line> = Vec::new();
    match &state.config {
        None => {
            lines.push(Line::from(Span::styled(
                crate::i18n::t("tui-memory-config-loading"),
                Style::default().fg(theme::TEXT_SECONDARY),
            )));
        }
        Some(cfg) => {
            // The selected row carries a marker AND a colour: a colour-only
            // cue vanishes on a monochrome terminal, which is exactly where
            // people run this.
            let sel = state.config_field;
            let label = |k: &str, mine: bool| {
                Span::styled(
                    format!("{} {k:<26}", if mine { ">" } else { " " }),
                    if mine {
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::TEXT_SECONDARY)
                    },
                )
            };
            let value = |v: &str| {
                Span::styled(v.to_string(), Style::default().add_modifier(Modifier::BOLD))
            };
            let onoff = |b: bool| {
                if b {
                    crate::i18n::t("tui-memory-config-on")
                } else {
                    crate::i18n::t("tui-memory-config-off")
                }
            };

            lines.push(Line::from(Span::styled(
                crate::i18n::t("tui-memory-config-remembering"),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(vec![
                label(
                    &crate::i18n::t("tui-memory-config-auto-memorize"),
                    sel == ConfigField::AutoMemorize,
                ),
                value(&onoff(cfg.auto_memorize)),
            ]));
            lines.push(Line::from(vec![
                label(
                    &crate::i18n::t("tui-memory-config-auto-retrieve"),
                    sel == ConfigField::AutoRetrieve,
                ),
                value(&onoff(cfg.auto_retrieve)),
            ]));

            // The point of this screen: name the model doing the extraction,
            // and say whether anyone chose it. An inherited slow model runs
            // after every reply and delays every answer, and nothing else in
            // the terminal reported it.
            let mut spans = vec![label(
                &crate::i18n::t("tui-memory-config-extraction-model"),
                sel == ConfigField::ExtractionModel,
            )];
            if state.config_editing_model {
                spans.push(Span::styled(
                    format!("{}_", state.config_model_buf),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(value(&cfg.effective_extraction_model));
            }
            if cfg.extraction_model_inherited {
                spans.push(Span::styled(
                    format!("  {}", crate::i18n::t("tui-memory-config-inherited")),
                    Style::default().fg(theme::YELLOW),
                ));
            }
            lines.push(Line::from(spans));

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                crate::i18n::t("tui-memory-config-searching"),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(vec![
                label(
                    &crate::i18n::t("tui-memory-config-embedding-provider"),
                    false,
                ),
                value(&cfg.embedding_provider),
            ]));
            lines.push(Line::from(vec![
                label(&crate::i18n::t("tui-memory-config-embedding-model"), false),
                value(&cfg.embedding_model),
            ]));
        }
    }

    if state.config_dirty {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            crate::i18n::t("tui-memory-config-unsaved"),
            Style::default().fg(theme::YELLOW),
        )));
    }
    if !state.status_msg.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status_msg.clone(),
            Style::default().fg(theme::TEXT_SECONDARY),
        )));
    }

    f.render_widget(Paragraph::new(lines), chunks[0]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            crate::i18n::t("tui-memory-config-hint"),
            Style::default().fg(theme::TEXT_SECONDARY),
        ))),
        chunks[1],
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// The terminal had no way at all to ask which model writes memories: the
    /// screen was a key/value browser and nothing else. On a live deployment
    /// that model was inherited from the system default, ran after every
    /// reply, and could not finish inside its own ceiling — so every answer
    /// was held for over two minutes and no surface named the cause.
    #[test]
    fn config_view_is_reachable_and_asks_for_its_data() {
        let mut state = MemoryState::new();
        assert!(matches!(state.sub, MemorySub::AgentSelect));

        let action = state.handle_key(key(KeyCode::Char('c')));

        assert!(matches!(state.sub, MemorySub::Config));
        assert!(
            matches!(action, MemoryUIAction::LoadConfig),
            "opening the panel must fetch, or it shows a blank as if that were the configuration"
        );
    }

    #[test]
    fn config_view_returns_to_the_agent_list() {
        let mut state = MemoryState::new();
        state.handle_key(key(KeyCode::Char('c')));

        state.handle_key(key(KeyCode::Esc));
        assert!(matches!(state.sub, MemorySub::AgentSelect));
    }

    /// Reopening with data already loaded must not blank the panel behind a
    /// spinner: the previous answer stays on screen while the refresh runs.
    #[test]
    fn reopening_with_data_does_not_show_a_spinner() {
        let mut state = MemoryState::new();
        state.config = Some(MemoryConfigView {
            effective_extraction_model: "sensor-model-generic".to_string(),
            ..Default::default()
        });

        state.handle_key(key(KeyCode::Char('c')));

        assert!(
            !state.loading,
            "a cached config must not be hidden behind a load"
        );
    }

    #[test]
    fn refresh_from_inside_the_panel_refetches() {
        let mut state = MemoryState::new();
        state.handle_key(key(KeyCode::Char('c')));

        let action = state.handle_key(key(KeyCode::Char('r')));
        assert!(matches!(action, MemoryUIAction::LoadConfig));
        assert!(
            matches!(state.sub, MemorySub::Config),
            "refresh must not leave the panel"
        );
    }

    /// `c` on the agent list is the panel, not a stray key on the KV browser —
    /// the browser uses its own bindings and must keep them.
    fn loaded(model: &str) -> MemoryState {
        let mut state = MemoryState::new();
        state.config = Some(MemoryConfigView {
            auto_memorize: true,
            auto_retrieve: true,
            effective_extraction_model: model.to_string(),
            extraction_model_inherited: true,
            ..Default::default()
        });
        state.sub = MemorySub::Config;
        state
    }

    #[test]
    fn toggling_a_row_marks_the_panel_unsaved() {
        let mut state = loaded("litellm:x");

        state.handle_key(key(KeyCode::Char(' ')));

        assert!(!state.config.as_ref().unwrap().auto_memorize);
        assert!(
            state.config_dirty,
            "an edit must be visible as unsaved work"
        );
    }

    #[test]
    fn moving_between_rows_edits_the_row_it_shows() {
        let mut state = loaded("litellm:x");

        state.handle_key(key(KeyCode::Down));
        state.handle_key(key(KeyCode::Char(' ')));

        let cfg = state.config.as_ref().unwrap();
        assert!(cfg.auto_memorize, "the first row must be untouched");
        assert!(!cfg.auto_retrieve, "space must act on the selected row");
    }

    /// Typing a model name must not be read as navigation, or every `r` in an
    /// id would refresh and every `j` would move a row.
    #[test]
    fn typing_a_model_name_does_not_trigger_shortcuts() {
        let mut state = loaded("old");
        state.handle_key(key(KeyCode::Down));
        state.handle_key(key(KeyCode::Down));
        let action = state.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, MemoryUIAction::Continue));
        assert!(state.config_editing_model);

        for c in "jrsq".chars() {
            let action = state.handle_key(key(KeyCode::Char(c)));
            assert!(
                matches!(action, MemoryUIAction::Continue),
                "'{c}' must be typed, not obeyed"
            );
        }
        assert_eq!(state.config_model_buf, "oldjrsq");
        assert!(
            matches!(state.sub, MemorySub::Config),
            "'q' must not have exited"
        );
    }

    #[test]
    fn accepting_a_typed_model_stops_it_being_inherited() {
        let mut state = loaded("inherited-one");
        state.config_field = ConfigField::ExtractionModel;
        state.handle_key(key(KeyCode::Enter));
        for c in "-fast".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Enter));

        let cfg = state.config.as_ref().unwrap();
        assert_eq!(cfg.effective_extraction_model, "inherited-one-fast");
        assert!(
            !cfg.extraction_model_inherited,
            "choosing a model is not inheriting one; the panel must stop saying it is"
        );
    }

    #[test]
    fn abandoning_a_typed_model_restores_what_was_there() {
        let mut state = loaded("keep-me");
        state.config_field = ConfigField::ExtractionModel;
        state.handle_key(key(KeyCode::Enter));
        for c in "zzz".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Esc));

        assert!(!state.config_editing_model);
        assert_eq!(
            state.config.as_ref().unwrap().effective_extraction_model,
            "keep-me"
        );
    }

    #[test]
    fn saving_sends_exactly_what_the_panel_shows() {
        let mut state = loaded("litellm:sensor-model-generic");
        state.handle_key(key(KeyCode::Char(' ')));

        let action = state.handle_key(key(KeyCode::Char('s')));

        match action {
            MemoryUIAction::SaveConfig {
                auto_memorize,
                auto_retrieve,
                extraction_model,
            } => {
                assert!(
                    !auto_memorize,
                    "the toggle the operator flipped must be sent"
                );
                assert!(auto_retrieve);
                assert_eq!(extraction_model, "litellm:sensor-model-generic");
            }
            other => panic!("expected a save, got {other:?}"),
        }
        assert!(
            state.config_dirty,
            "the keypress only asks for a save; the marker is what says the edits are still pending"
        );
    }

    #[test]
    fn a_successful_save_clears_the_unsaved_marker() {
        let mut state = loaded("litellm:x");
        state.handle_key(key(KeyCode::Char(' ')));
        let _ = state.handle_key(key(KeyCode::Char('s')));

        state.apply_save_result(Ok(()));

        assert!(
            !state.config_dirty,
            "a successful save clears the unsaved marker"
        );
        assert_eq!(
            state.status_msg,
            crate::i18n::t("tui-memory-config-saved"),
            "a saved panel must say it saved, not repeat a row label"
        );
    }

    /// A failed PATCH used to read as "everything is saved" with a transient
    /// error beside it, because the marker was gone before the daemon answered.
    #[test]
    fn a_failed_save_keeps_the_unsaved_marker() {
        let mut state = loaded("litellm:x");
        state.handle_key(key(KeyCode::Char(' ')));
        let _ = state.handle_key(key(KeyCode::Char('s')));

        state.apply_save_result(Err(crate::tui::event::FetchFailure::Error(
            "500 Internal Server Error".to_string(),
        )));

        assert!(
            state.config_dirty,
            "edits the daemon refused are still unsaved"
        );
        assert!(
            state.status_msg.contains("500"),
            "the panel must carry why it failed, got {:?}",
            state.status_msg
        );
    }

    /// In-process there is no HTTP surface to PATCH, so nothing ever answered
    /// and the panel sat on "Saving..." with no way out.
    #[test]
    fn a_save_without_a_daemon_says_so_instead_of_hanging() {
        let mut state = loaded("litellm:x");
        state.handle_key(key(KeyCode::Char(' ')));
        let _ = state.handle_key(key(KeyCode::Char('s')));

        state.apply_save_result(Err(crate::tui::event::FetchFailure::RequiresDaemon));

        assert_eq!(
            state.status_msg,
            crate::i18n::t("tui-memory-config-requires-daemon")
        );
        assert!(
            state.config_dirty,
            "nothing was written, so nothing is safe"
        );
    }

    #[test]
    fn saving_without_data_does_nothing() {
        let mut state = MemoryState::new();
        state.sub = MemorySub::Config;

        let action = state.handle_key(key(KeyCode::Char('s')));
        assert!(
            matches!(action, MemoryUIAction::Continue),
            "an empty panel must not PATCH blanks over the stored configuration"
        );
    }

    #[test]
    fn the_kv_browser_does_not_lose_its_own_bindings() {
        let mut state = MemoryState::new();
        state.sub = MemorySub::KvBrowser;

        state.handle_key(key(KeyCode::Char('c')));
        assert!(
            !matches!(state.sub, MemorySub::Config),
            "the config panel opens from the agent list, not from inside the browser"
        );
    }
}
