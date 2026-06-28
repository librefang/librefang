//! Triggers screen: CRUD with pattern type picker.

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
pub struct TriggerInfo {
    pub id: String,
    pub agent_id: String,
    pub pattern: String,
    pub fires: u64,
    pub enabled: bool,
}

const PATTERN_TYPES: &[&str] = &[
    "Lifecycle",
    "AgentSpawned",
    "ContentMatch",
    "Schedule",
    "Webhook",
    "ChannelMessage",
];

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
pub enum TriggerSubScreen {
    List,
    Create,
}

pub struct TriggerState {
    pub sub: TriggerSubScreen,
    pub triggers: Vec<TriggerInfo>,
    pub list_state: ListState,
    // Create wizard
    pub create_step: usize, // 0=agent, 1=pattern_type, 2=param, 3=prompt, 4=max_fires, 5=review
    pub create_agent_id: String,
    pub create_pattern_type: usize,
    pub create_pattern_param: String,
    pub create_prompt: String,
    pub create_max_fires: String,
    pub pattern_type_list: ListState,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum TriggerAction {
    Continue,
    Refresh,
    CreateTrigger {
        agent_id: String,
        pattern_type: String,
        pattern_param: String,
        prompt: String,
        max_fires: u64,
    },
    DeleteTrigger(String),
}

impl TriggerState {
    pub fn new() -> Self {
        Self {
            sub: TriggerSubScreen::List,
            triggers: Vec::new(),
            list_state: ListState::default(),
            create_step: 0,
            create_agent_id: String::new(),
            create_pattern_type: 0,
            create_pattern_param: String::new(),
            create_prompt: String::new(),
            create_max_fires: String::new(),
            pattern_type_list: ListState::default(),
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> TriggerAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return TriggerAction::Continue;
        }
        match self.sub {
            TriggerSubScreen::List => self.handle_list(key),
            TriggerSubScreen::Create => self.handle_create(key),
        }
    }

    fn handle_list(&mut self, key: KeyEvent) -> TriggerAction {
        let total = self.triggers.len() + 1; // +1 for "Create new"
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.list_state.select(Some(next));
            }
            KeyCode::Char('d') => {
                if let Some(idx) = self.list_state.selected() {
                    if idx < self.triggers.len() {
                        let id = self.triggers[idx].id.clone();
                        return TriggerAction::DeleteTrigger(id);
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(idx) = self.list_state.selected() {
                    if idx >= self.triggers.len() {
                        // "Create new"
                        self.create_step = 0;
                        self.create_agent_id.clear();
                        self.create_pattern_type = 0;
                        self.create_pattern_param.clear();
                        self.create_prompt.clear();
                        self.create_max_fires.clear();
                        self.pattern_type_list.select(Some(0));
                        self.sub = TriggerSubScreen::Create;
                    }
                }
            }
            KeyCode::Char('r') => return TriggerAction::Refresh,
            _ => {}
        }
        TriggerAction::Continue
    }

    fn handle_create(&mut self, key: KeyEvent) -> TriggerAction {
        match self.create_step {
            1 => return self.handle_pattern_picker(key),
            5 => return self.handle_review(key),
            _ => {}
        }

        match key.code {
            KeyCode::Esc => {
                if self.create_step == 0 {
                    self.sub = TriggerSubScreen::List;
                } else {
                    self.create_step -= 1;
                }
            }
            KeyCode::Enter if self.create_step < 5 => {
                self.create_step += 1;
            }
            KeyCode::Char(c) => match self.create_step {
                0 => self.create_agent_id.push(c),
                2 => self.create_pattern_param.push(c),
                3 => self.create_prompt.push(c),
                4 if c.is_ascii_digit() => {
                    self.create_max_fires.push(c);
                }
                _ => {}
            },
            KeyCode::Backspace => match self.create_step {
                0 => {
                    self.create_agent_id.pop();
                }
                2 => {
                    self.create_pattern_param.pop();
                }
                3 => {
                    self.create_prompt.pop();
                }
                4 => {
                    self.create_max_fires.pop();
                }
                _ => {}
            },
            _ => {}
        }
        TriggerAction::Continue
    }

    fn handle_pattern_picker(&mut self, key: KeyEvent) -> TriggerAction {
        match key.code {
            KeyCode::Esc => {
                self.create_step = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.pattern_type_list.selected().unwrap_or(0);
                let next = if i == 0 {
                    PATTERN_TYPES.len() - 1
                } else {
                    i - 1
                };
                self.pattern_type_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.pattern_type_list.selected().unwrap_or(0);
                let next = (i + 1) % PATTERN_TYPES.len();
                self.pattern_type_list.select(Some(next));
            }
            KeyCode::Enter => {
                if let Some(idx) = self.pattern_type_list.selected() {
                    self.create_pattern_type = idx;
                    self.create_step = 2;
                }
            }
            _ => {}
        }
        TriggerAction::Continue
    }

    fn handle_review(&mut self, key: KeyEvent) -> TriggerAction {
        match key.code {
            KeyCode::Esc => {
                self.create_step = 4;
            }
            KeyCode::Enter => {
                let max_fires = self.create_max_fires.parse::<u64>().unwrap_or(0);
                let pattern_type = PATTERN_TYPES
                    .get(self.create_pattern_type)
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                self.sub = TriggerSubScreen::List;
                return TriggerAction::CreateTrigger {
                    agent_id: self.create_agent_id.clone(),
                    pattern_type,
                    pattern_param: self.create_pattern_param.clone(),
                    prompt: self.create_prompt.clone(),
                    max_fires,
                };
            }
            _ => {}
        }
        TriggerAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut TriggerState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("⊙ {}", crate::i18n::t("tui-triggers-title-screen")),
    );

    match state.sub {
        TriggerSubScreen::List => draw_list(f, inner, state),
        TriggerSubScreen::Create => draw_create(f, inner, state),
    }
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut TriggerState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // separator
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<14} {:<20} {:<8} {}",
                crate::i18n::t("tui-triggers-header-agent"),
                crate::i18n::t("tui-triggers-header-pattern"),
                crate::i18n::t("tui-triggers-header-fires"),
                crate::i18n::t("tui-triggers-header-status")
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-triggers-loading")),
            chunks[2],
        );
    } else if state.triggers.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-triggers-empty-state")),
            chunks[2],
        );
    } else {
        let mut items: Vec<ListItem> = state
            .triggers
            .iter()
            .map(|tr| {
                let (enabled_text, enabled_style) = if tr.enabled {
                    (
                        crate::i18n::t("tui-triggers-status-active"),
                        Style::default().fg(theme::GREEN),
                    )
                } else {
                    (
                        crate::i18n::t("tui-triggers-status-off"),
                        Style::default().fg(theme::RED),
                    )
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<14}", widgets::truncate(&tr.agent_id, 13)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {:<20}", widgets::truncate(&tr.pattern, 19)),
                        Style::default().fg(theme::YELLOW),
                    ),
                    Span::styled(
                        format!(" {:<8}", tr.fires),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                    Span::styled(format!(" {enabled_text}"), enabled_style),
                ]))
            })
            .collect();

        items.push(ListItem::new(Line::from(vec![Span::styled(
            crate::i18n::t("tui-triggers-create-new-option"),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )])));

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[2], &mut state.list_state);
    }

    f.render_widget(
        widgets::status_or_hint(
            &state.status_msg,
            &crate::i18n::t("tui-triggers-hints-list"),
        ),
        chunks[3],
    );
}

fn draw_create(f: &mut Frame, area: Rect, state: &mut TriggerState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // separator
        Constraint::Length(1), // step progress
        Constraint::Length(1), // spacer
        Constraint::Min(6),    // content
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ⊙ ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-triggers-title-create"),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // Step progress indicator with filled/hollow circles
    let progress: Vec<Span> = (0..6)
        .map(|i| {
            if i < state.create_step {
                Span::styled("● ", Style::default().fg(theme::GREEN))
            } else if i == state.create_step {
                Span::styled("● ", Style::default().fg(theme::ACCENT))
            } else {
                Span::styled("○ ", Style::default().fg(theme::TEXT_TERTIARY))
            }
        })
        .collect();
    let mut step_line = vec![Span::raw("  ")];
    step_line.extend(progress);
    step_line.push(Span::styled(
        crate::i18n::t_args(
            "tui-triggers-create-step",
            &[
                ("current", &(state.create_step + 1).to_string()),
                ("total", "6"),
            ],
        ),
        Style::default().fg(theme::TEXT_SECONDARY),
    ));
    f.render_widget(Paragraph::new(Line::from(step_line)), chunks[2]);

    let label_agent_id = crate::i18n::t("tui-triggers-label-agent-id");
    let placeholder_agent_id = crate::i18n::t("tui-triggers-placeholder-agent-id");
    let label_prompt = crate::i18n::t("tui-triggers-label-prompt");
    let placeholder_prompt = crate::i18n::t("tui-triggers-placeholder-prompt");
    let label_max_fires = crate::i18n::t("tui-triggers-label-max-fires");
    let placeholder_max_fires = crate::i18n::t("tui-triggers-placeholder-max-fires");

    let pattern_tech_name = PATTERN_TYPES
        .get(state.create_pattern_type)
        .copied()
        .unwrap_or("?");
    let pattern_type_loc = crate::i18n::t(&format!(
        "tui-triggers-type-{}-name",
        pattern_tech_name.to_lowercase()
    ));
    let label_pattern_param = crate::i18n::t_args(
        "tui-triggers-prompt-param",
        &[("type", pattern_type_loc.as_str())],
    );
    let placeholder_pattern_param = crate::i18n::t("tui-triggers-placeholder-pattern-param");

    match state.create_step {
        0 => draw_text_field(
            f,
            chunks[4],
            label_agent_id.as_str(),
            &state.create_agent_id,
            placeholder_agent_id.as_str(),
        ),
        1 => draw_pattern_picker(f, chunks[4], state),
        2 => draw_text_field(
            f,
            chunks[4],
            label_pattern_param.as_str(),
            &state.create_pattern_param,
            placeholder_pattern_param.as_str(),
        ),
        3 => draw_text_field(
            f,
            chunks[4],
            label_prompt.as_str(),
            &state.create_prompt,
            placeholder_prompt.as_str(),
        ),
        4 => draw_text_field(
            f,
            chunks[4],
            label_max_fires.as_str(),
            &state.create_max_fires,
            placeholder_max_fires.as_str(),
        ),
        _ => draw_trigger_review(f, chunks[4], state),
    }

    let hint_string = if state.create_step == 5 {
        crate::i18n::t("tui-triggers-hints-create-submit")
    } else if state.create_step == 1 {
        crate::i18n::t("tui-triggers-hints-create-select")
    } else {
        crate::i18n::t("tui-triggers-hints-create-next")
    };
    f.render_widget(widgets::hint_bar(&hint_string), chunks[5]);
}

fn draw_text_field(f: &mut Frame, area: Rect, label: &str, value: &str, placeholder: &str) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {label}"),
            Style::default().fg(theme::TEXT_PRIMARY),
        )])),
        chunks[0],
    );

    let display = if value.is_empty() { placeholder } else { value };
    let style = if value.is_empty() {
        theme::dim_style()
    } else {
        theme::input_style()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ❯ ", Style::default().fg(theme::ACCENT)),
            Span::styled(display, style),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])),
        chunks[2],
    );
}

fn draw_pattern_picker(f: &mut Frame, area: Rect, state: &mut TriggerState) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            crate::i18n::t("tui-triggers-label-pattern-picker"),
            Style::default().fg(theme::TEXT_PRIMARY),
        )])),
        chunks[0],
    );

    let items: Vec<ListItem> = PATTERN_TYPES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let indicator = if Some(i) == state.pattern_type_list.selected() {
                "●"
            } else {
                "○"
            };

            let name_key = format!("tui-triggers-type-{}-name", name.to_lowercase());
            let desc_key = format!("tui-triggers-type-{}-desc", name.to_lowercase());
            let loc_name = crate::i18n::t(&name_key);
            let loc_desc = crate::i18n::t(&desc_key);

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {indicator} "),
                    Style::default().fg(theme::ACCENT),
                ),
                Span::styled(
                    format!("{:<18}", loc_name),
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(loc_desc, Style::default().fg(theme::TEXT_SECONDARY)),
            ]))
        })
        .collect();

    let list = widgets::themed_list(items);
    f.render_stateful_widget(list, chunks[1], &mut state.pattern_type_list);
}

fn draw_trigger_review(f: &mut Frame, area: Rect, state: &TriggerState) {
    let pattern_tech_name = PATTERN_TYPES
        .get(state.create_pattern_type)
        .copied()
        .unwrap_or("?");
    let pattern_name = crate::i18n::t(&format!(
        "tui-triggers-type-{}-name",
        pattern_tech_name.to_lowercase()
    ));
    let unlimited_str = crate::i18n::t("tui-triggers-review-unlimited");
    let max_fires = if state.create_max_fires.is_empty() {
        unlimited_str.as_str()
    } else {
        &state.create_max_fires
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                crate::i18n::t("tui-triggers-review-agent"),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
            Span::styled(&state.create_agent_id, Style::default().fg(theme::CYAN)),
        ]),
        Line::from(vec![
            Span::styled(
                crate::i18n::t("tui-triggers-review-pattern"),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
            Span::styled(pattern_name, Style::default().fg(theme::YELLOW)),
            Span::styled(
                format!(" ({})", state.create_pattern_param),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                crate::i18n::t("tui-triggers-review-prompt"),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
            Span::styled(
                &state.create_prompt,
                Style::default().fg(theme::TEXT_PRIMARY),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                crate::i18n::t("tui-triggers-review-max"),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
            Span::styled(max_fires, Style::default().fg(theme::GREEN)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-triggers-review-confirm"),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), area);
}
