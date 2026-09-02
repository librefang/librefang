//! Workflows screen: CRUD, run input, run history.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct WorkflowInfo {
    pub id: String,
    pub name: String,
    pub steps: usize,
    pub created: String,
}

#[derive(Clone, Default)]
pub struct WorkflowRun {
    pub id: String,
    pub state: String,
    pub duration: String,
    pub output_preview: String,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
pub enum WorkflowSubScreen {
    List,
    Runs,
    Create,
    RunInput,
    RunResult,
}

/// One declared parameter, with whatever the operator has typed so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowParamField {
    pub name: String,
    pub param_type: String,
    pub required: bool,
    pub description: String,
    pub value: String,
}

/// What the run-input form knows about the workflow's declared parameters.
///
/// Every fetch outcome is one of these — success, "declares none" and
/// "could not load" are three different states, because each one needs a
/// different next step from the operator and a silent fallback to the
/// bare-string box hides all three behind the same screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowParamsFetch {
    /// The workflow declared these parameters — one editable row per param.
    Loaded(Vec<WorkflowParamField>),
    /// The workflow answered and declares no parameters — bare-string input.
    None,
    /// The schema could not be consulted (in-process mode, the daemon was
    /// unreachable, or the answer was not a success). Bare-string input,
    /// with the status line saying so.
    Failed,
}

pub struct WorkflowState {
    pub sub: WorkflowSubScreen,
    pub workflows: Vec<WorkflowInfo>,
    pub list_state: ListState,
    pub selected_workflow: Option<usize>,
    // Run history
    pub runs: Vec<WorkflowRun>,
    pub runs_list_state: ListState,
    // Create wizard
    pub create_step: usize, // 0=name, 1=desc, 2=steps_json, 3=review
    pub create_name: String,
    pub create_desc: String,
    pub create_steps: String,
    // Run — declared parameters fetched from the workflow's `input_schema`
    pub run_params: Vec<WorkflowParamField>,
    pub param_cursor: usize,
    pub run_input: String,
    pub run_result: Option<String>,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum WorkflowAction {
    Continue,
    Refresh,
    LoadRuns(String),
    FetchWorkflowParams(String),
    CreateWorkflow {
        name: String,
        description: String,
        steps_json: String,
    },
    RunWorkflow {
        id: String,
        input: String,
    },
}

impl WorkflowState {
    pub fn new() -> Self {
        Self {
            sub: WorkflowSubScreen::List,
            workflows: Vec::new(),
            list_state: ListState::default(),
            selected_workflow: None,
            runs: Vec::new(),
            runs_list_state: ListState::default(),
            create_step: 0,
            create_name: String::new(),
            create_desc: String::new(),
            create_steps: String::new(),
            run_params: Vec::new(),
            param_cursor: 0,
            run_input: String::new(),
            run_result: None,
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> WorkflowAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return WorkflowAction::Continue;
        }
        match self.sub {
            WorkflowSubScreen::List => self.handle_list(key),
            WorkflowSubScreen::Runs => self.handle_runs(key),
            WorkflowSubScreen::Create => self.handle_create(key),
            WorkflowSubScreen::RunInput => self.handle_run_input(key),
            WorkflowSubScreen::RunResult => self.handle_run_result(key),
        }
    }

    fn handle_list(&mut self, key: KeyEvent) -> WorkflowAction {
        let total = self.workflows.len() + 1; // +1 for "Create new"
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
            KeyCode::Enter => {
                if let Some(idx) = self.list_state.selected() {
                    if idx < self.workflows.len() {
                        self.selected_workflow = Some(idx);
                        let wf_id = self.workflows[idx].id.clone();
                        self.runs_list_state.select(Some(0));
                        self.sub = WorkflowSubScreen::Runs;
                        return WorkflowAction::LoadRuns(wf_id);
                    } else {
                        // "Create new"
                        self.create_step = 0;
                        self.create_name.clear();
                        self.create_desc.clear();
                        self.create_steps.clear();
                        self.sub = WorkflowSubScreen::Create;
                    }
                }
            }
            KeyCode::Char('x') => {
                if let Some(idx) = self.list_state.selected() {
                    if idx < self.workflows.len() {
                        self.selected_workflow = Some(idx);
                        self.run_params.clear();
                        self.param_cursor = 0;
                        self.run_input.clear();
                        self.run_result = None;
                        self.status_msg.clear();
                        self.sub = WorkflowSubScreen::RunInput;
                        return WorkflowAction::FetchWorkflowParams(self.workflows[idx].id.clone());
                    }
                }
            }
            KeyCode::Char('r') => return WorkflowAction::Refresh,
            _ => {}
        }
        WorkflowAction::Continue
    }

    fn handle_runs(&mut self, key: KeyEvent) -> WorkflowAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = WorkflowSubScreen::List;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.runs_list_state.selected().unwrap_or(0);
                let next = if i == 0 {
                    self.runs.len().saturating_sub(1)
                } else {
                    i - 1
                };
                self.runs_list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.runs_list_state.selected().unwrap_or(0);
                let total = self.runs.len().max(1);
                let next = (i + 1) % total;
                self.runs_list_state.select(Some(next));
            }
            KeyCode::Char('r') => {
                if let Some(idx) = self.selected_workflow {
                    if idx < self.workflows.len() {
                        let wf_id = self.workflows[idx].id.clone();
                        return WorkflowAction::LoadRuns(wf_id);
                    }
                }
            }
            _ => {}
        }
        WorkflowAction::Continue
    }

    fn handle_create(&mut self, key: KeyEvent) -> WorkflowAction {
        match key.code {
            KeyCode::Esc => {
                if self.create_step == 0 {
                    self.sub = WorkflowSubScreen::List;
                } else {
                    self.create_step -= 1;
                }
            }
            KeyCode::Enter => {
                if self.create_step < 3 {
                    self.create_step += 1;
                } else {
                    // Submit
                    let action = WorkflowAction::CreateWorkflow {
                        name: self.create_name.clone(),
                        description: self.create_desc.clone(),
                        steps_json: self.create_steps.clone(),
                    };
                    self.sub = WorkflowSubScreen::List;
                    return action;
                }
            }
            KeyCode::Char(c) => match self.create_step {
                0 => self.create_name.push(c),
                1 => self.create_desc.push(c),
                2 => self.create_steps.push(c),
                _ => {}
            },
            KeyCode::Backspace => match self.create_step {
                0 => {
                    self.create_name.pop();
                }
                1 => {
                    self.create_desc.pop();
                }
                2 => {
                    self.create_steps.pop();
                }
                _ => {}
            },
            _ => {}
        }
        WorkflowAction::Continue
    }

    fn handle_run_input(&mut self, key: KeyEvent) -> WorkflowAction {
        let field_count = self.run_params.len() + 1;
        match key.code {
            KeyCode::Esc => {
                self.sub = WorkflowSubScreen::List;
            }
            KeyCode::Tab | KeyCode::Down => {
                self.param_cursor = (self.param_cursor + 1) % field_count;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.param_cursor = if self.param_cursor == 0 {
                    field_count - 1
                } else {
                    self.param_cursor - 1
                };
            }
            KeyCode::Enter => {
                if let Some(missing) = self
                    .run_params
                    .iter()
                    .find(|p| p.required && p.value.trim().is_empty())
                {
                    self.status_msg = crate::i18n::t_args(
                        "tui-workflows-param-required",
                        &[("name", missing.name.as_str())],
                    );
                    return WorkflowAction::Continue;
                }
                if let Some(idx) = self.selected_workflow {
                    if idx < self.workflows.len() {
                        let wf_id = self.workflows[idx].id.clone();
                        let input = self.build_run_input();
                        self.loading = true;
                        self.sub = WorkflowSubScreen::RunResult;
                        return WorkflowAction::RunWorkflow { id: wf_id, input };
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(field) = self.run_params.get_mut(self.param_cursor) {
                    field.value.push(c);
                } else {
                    self.run_input.push(c);
                }
            }
            KeyCode::Backspace => {
                if let Some(field) = self.run_params.get_mut(self.param_cursor) {
                    field.value.pop();
                } else {
                    self.run_input.pop();
                }
            }
            _ => {}
        }
        WorkflowAction::Continue
    }

    /// Build the payload sent as `input`.
    ///
    /// With declared parameters this is a JSON object keyed by parameter name;
    /// with none declared it stays the bare string every pre-schema workflow expects.
    pub fn build_run_input(&self) -> String {
        if self.run_params.is_empty() {
            return self.run_input.clone();
        }
        let mut obj = serde_json::Map::new();
        for p in &self.run_params {
            if p.value.trim().is_empty() {
                continue;
            }
            let value = match p.param_type.as_str() {
                "number" => p
                    .value
                    .trim()
                    .parse::<f64>()
                    .map(|n| serde_json::json!(n))
                    .unwrap_or_else(|_| serde_json::json!(p.value)),
                "boolean" => serde_json::json!(matches!(
                    p.value.trim().to_ascii_lowercase().as_str(),
                    "true" | "1" | "yes"
                )),
                _ => serde_json::json!(p.value),
            };
            obj.insert(p.name.clone(), value);
        }
        // A declared parameter named `input` wins over the free-text box: the
        // loop above bound it by name, and the free-text line has no declared
        // parameter to be.
        if !self.run_input.trim().is_empty() && !obj.contains_key("input") {
            obj.insert("input".to_string(), serde_json::json!(self.run_input));
        }
        serde_json::to_string(&serde_json::Value::Object(obj))
            .unwrap_or_else(|_| self.run_input.clone())
    }

    fn handle_run_result(&mut self, key: KeyEvent) -> WorkflowAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.sub = WorkflowSubScreen::List;
                self.loading = false;
            }
            _ => {}
        }
        WorkflowAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut WorkflowState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("▷ {}", crate::i18n::t("tui-workflows-title-screen")),
    );

    match state.sub {
        WorkflowSubScreen::List => draw_list(f, inner, state),
        WorkflowSubScreen::Runs => draw_runs(f, inner, state),
        WorkflowSubScreen::Create => draw_create(f, inner, state),
        WorkflowSubScreen::RunInput => draw_run_input(f, inner, state),
        WorkflowSubScreen::RunResult => draw_run_result(f, inner, state),
    }
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut WorkflowState) {
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
                "  {:<12} {:<24} {:<8} {}",
                crate::i18n::t("tui-workflows-header-id"),
                crate::i18n::t("tui-workflows-header-name"),
                crate::i18n::t("tui-workflows-header-steps"),
                crate::i18n::t("tui-workflows-header-created")
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-workflows-loading")),
            chunks[2],
        );
    } else if state.workflows.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-workflows-empty-state")),
            chunks[2],
        );
    } else {
        let mut items: Vec<ListItem> = state
            .workflows
            .iter()
            .map(|wf| {
                let step_icon = if wf.steps > 0 { "\u{25cf}" } else { "\u{25cb}" };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<12}", widgets::truncate(&wf.id, 11)),
                        theme::dim_style(),
                    ),
                    Span::styled(
                        format!(" {:<24}", widgets::truncate(&wf.name, 23)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {} {:<6}", step_icon, wf.steps),
                        Style::default().fg(theme::YELLOW),
                    ),
                    Span::styled(
                        format!(" {}", wf.created),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                ]))
            })
            .collect();

        items.push(ListItem::new(Line::from(vec![Span::styled(
            crate::i18n::t("tui-workflows-create-new-option"),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )])));

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[2], &mut state.list_state);
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-workflows-hints-list")),
        chunks[3],
    );
}

fn draw_runs(f: &mut Frame, area: Rect, state: &mut WorkflowState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // header
        Constraint::Length(1), // separator
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    let wf_name = state
        .selected_workflow
        .and_then(|i| state.workflows.get(i))
        .map(|w| w.name.as_str())
        .unwrap_or("?");

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{25b7} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t_args("tui-workflows-title-runs", &[("name", wf_name)]),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<12} {:<12} {:<12} {}",
                crate::i18n::t("tui-workflows-header-run-id"),
                crate::i18n::t("tui-workflows-header-state"),
                crate::i18n::t("tui-workflows-header-duration"),
                crate::i18n::t("tui-workflows-header-output")
            ),
            theme::table_header(),
        )])),
        chunks[1],
    );

    f.render_widget(widgets::separator(chunks[2].width), chunks[2]);

    if state.runs.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-workflows-runs-empty")),
            chunks[3],
        );
    } else {
        let items: Vec<ListItem> = state
            .runs
            .iter()
            .map(|run| {
                let (badge, badge_style) = theme::state_badge(&run.state);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<12}", widgets::truncate(&run.id, 11)),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                    Span::styled(format!(" {:<12}", badge), badge_style),
                    Span::styled(
                        format!(" {:<12}", run.duration),
                        Style::default().fg(theme::YELLOW),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&run.output_preview, 30)),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[3], &mut state.runs_list_state);
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-workflows-hints-runs")),
        chunks[4],
    );
}

fn draw_create(f: &mut Frame, area: Rect, state: &WorkflowState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // separator
        Constraint::Length(1), // step progress
        Constraint::Length(1), // spacer
        Constraint::Length(1), // field label
        Constraint::Length(1), // spacer
        Constraint::Length(1), // input
        Constraint::Min(0),
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{25b7} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-workflows-title-create"),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // Step progress indicator with filled/hollow circles
    let progress: Vec<Span> = (0..4)
        .map(|i| {
            if i < state.create_step {
                Span::styled("\u{25cf} ", Style::default().fg(theme::GREEN))
            } else if i == state.create_step {
                Span::styled("\u{25cf} ", Style::default().fg(theme::ACCENT))
            } else {
                Span::styled("\u{25cb} ", Style::default().fg(theme::TEXT_TERTIARY))
            }
        })
        .collect();
    let mut step_line = vec![Span::raw("  ")];
    step_line.extend(progress);
    step_line.push(Span::styled(
        crate::i18n::t_args(
            "tui-workflows-create-step",
            &[
                ("current", &(state.create_step + 1).to_string()),
                ("total", "4"),
            ],
        ),
        Style::default().fg(theme::TEXT_SECONDARY),
    ));
    f.render_widget(Paragraph::new(Line::from(step_line)), chunks[2]);

    let label_name = crate::i18n::t("tui-workflows-label-name");
    let placeholder_name = crate::i18n::t("tui-workflows-placeholder-name");
    let label_desc = crate::i18n::t("tui-workflows-label-desc");
    let placeholder_desc = crate::i18n::t("tui-workflows-placeholder-desc");
    let label_steps = crate::i18n::t("tui-workflows-label-steps");
    let placeholder_steps = crate::i18n::t("tui-workflows-placeholder-steps");
    let label_review = crate::i18n::t("tui-workflows-label-review");

    let (label, value, placeholder) = match state.create_step {
        0 => (
            label_name.as_str(),
            &state.create_name,
            placeholder_name.as_str(),
        ),
        1 => (
            label_desc.as_str(),
            &state.create_desc,
            placeholder_desc.as_str(),
        ),
        2 => (
            label_steps.as_str(),
            &state.create_steps,
            placeholder_steps.as_str(),
        ),
        _ => (label_review.as_str(), &state.create_name, ""),
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {label}"),
            Style::default().fg(theme::TEXT_PRIMARY),
        )])),
        chunks[4],
    );

    if state.create_step < 3 {
        let display = if value.is_empty() {
            placeholder
        } else {
            value.as_str()
        };
        let style = if value.is_empty() {
            theme::dim_style()
        } else {
            theme::input_style()
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{276f} ", Style::default().fg(theme::ACCENT)),
                Span::styled(display, style),
                Span::styled(
                    "\u{2588}",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ])),
            chunks[6],
        );
    } else {
        // Review
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(
                        crate::i18n::t("tui-workflows-review-name"),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                    Span::styled(&state.create_name, Style::default().fg(theme::CYAN)),
                ]),
                Line::from(vec![
                    Span::styled(
                        crate::i18n::t("tui-workflows-review-desc"),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                    Span::styled(&state.create_desc, Style::default().fg(theme::TEXT_PRIMARY)),
                ]),
            ]),
            chunks[6],
        );
    }

    // The steps field is authored as one raw JSON blob, so the routing keys a
    // step may carry are not discoverable from any control (#7724).
    // `agent_type` in particular has no other mention on this screen: the
    // placeholder can only show one binding at a time, and it shows
    // `agent_name`.
    if state.create_step == 2 {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    crate::i18n::t("tui-workflows-hint-steps"),
                    Style::default().fg(theme::TEXT_TERTIARY),
                ),
            ]))
            .wrap(Wrap { trim: true }),
            chunks[7],
        );
    }

    let hint_text = if state.create_step == 3 {
        crate::i18n::t("tui-workflows-hints-create-submit")
    } else {
        crate::i18n::t("tui-workflows-hints-create-next")
    };
    f.render_widget(widgets::hint_bar(&hint_text), chunks[8]);
}

fn draw_run_input(f: &mut Frame, area: Rect, state: &WorkflowState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        // One row per declared parameter, not one shared row — the old
        // `Length(1)` clipped everything past the first parameter while
        // Tab still moved the cursor onto the invisible rows.
        Constraint::Length(state.run_params.len() as u16),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let wf_name = state
        .selected_workflow
        .and_then(|i| state.workflows.get(i))
        .map(|w| w.name.as_str())
        .unwrap_or("?");

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{25b7} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t_args("tui-workflows-title-run-input", &[("name", wf_name)]),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // One line per declared parameter; the focused one shows a caret.
    let mut param_lines: Vec<Line> = Vec::new();
    for (i, p) in state.run_params.iter().enumerate() {
        let focused = state.param_cursor == i;
        let mark = if p.required { "*" } else { " " };
        param_lines.push(Line::from(vec![
            Span::styled(
                format!("  {}{:<16}", mark, p.name),
                if focused {
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::TEXT_SECONDARY)
                },
            ),
            Span::styled(
                format!("{}{}", p.value, if focused { "\u{2588}" } else { "" }),
                Style::default().fg(theme::TEXT_PRIMARY),
            ),
        ]));
    }
    if !param_lines.is_empty() {
        f.render_widget(Paragraph::new(param_lines), chunks[4]);
    }

    let hint = state
        .run_params
        .get(state.param_cursor)
        .filter(|p| !p.description.is_empty())
        .map(|p| format!("  {}", p.description));
    if !state.status_msg.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::YELLOW),
            )),
            chunks[3],
        );
    } else if let Some(hint) = hint {
        f.render_widget(
            Paragraph::new(Span::styled(
                hint,
                Style::default().fg(theme::TEXT_TERTIARY),
            )),
            chunks[3],
        );
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{25b7} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-workflows-label-run-input"),
                Style::default().fg(theme::TEXT_PRIMARY),
            ),
        ])),
        chunks[2],
    );

    let free_focused = state.param_cursor >= state.run_params.len();
    let placeholder = crate::i18n::t("tui-workflows-placeholder-run-input");
    let display = if state.run_input.is_empty() {
        placeholder.as_str()
    } else {
        &state.run_input
    };
    let style = if state.run_input.is_empty() {
        theme::dim_style()
    } else {
        theme::input_style()
    };

    if state.run_params.is_empty() || free_focused {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{276f} ", Style::default().fg(theme::ACCENT)),
                Span::styled(display, style),
                Span::styled(
                    "\u{2588}",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ])),
            chunks[5],
        );
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-workflows-hints-run-input")),
        chunks[6],
    );
}

fn draw_run_result(f: &mut Frame, area: Rect, state: &WorkflowState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{25b7} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-workflows-title-run-result"),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-workflows-running")),
            chunks[2],
        );
    } else if let Some(ref result) = state.run_result {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("  \u{25cf} ", Style::default().fg(theme::GREEN)),
                    Span::styled(
                        crate::i18n::t("tui-workflows-result-complete"),
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    format!("  {result}"),
                    Style::default().fg(theme::TEXT_PRIMARY),
                )]),
            ]),
            chunks[2],
        );
    } else {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-workflows-result-empty")),
            chunks[2],
        );
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-workflows-hints-run-result")),
        chunks[3],
    );
}

#[cfg(test)]
mod run_param_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn field(name: &str, ty: &str, required: bool) -> WorkflowParamField {
        WorkflowParamField {
            name: name.to_string(),
            param_type: ty.to_string(),
            required,
            description: String::new(),
            value: String::new(),
        }
    }

    fn state_with(params: Vec<WorkflowParamField>) -> WorkflowState {
        let mut s = WorkflowState::new();
        s.sub = WorkflowSubScreen::RunInput;
        s.workflows.push(WorkflowInfo {
            id: "wf-1".to_string(),
            ..Default::default()
        });
        s.selected_workflow = Some(0);
        s.run_params = params;
        s
    }

    #[test]
    fn typing_goes_into_the_focused_parameter_not_the_free_text() {
        let mut s = state_with(vec![field("ciudad", "string", true)]);
        for c in "Vigo".chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(s.run_params[0].value, "Vigo");
        assert!(s.run_input.is_empty());
    }

    #[test]
    fn tab_moves_to_the_next_field_and_wraps_to_the_free_text_box() {
        let mut s = state_with(vec![field("a", "string", true)]);
        s.handle_key(key(KeyCode::Tab));
        s.handle_key(key(KeyCode::Char('x')));
        assert_eq!(s.run_input, "x");
        assert!(s.run_params[0].value.is_empty());
    }

    #[test]
    fn enter_refuses_while_a_required_parameter_is_empty() {
        let mut s = state_with(vec![field("ciudad", "string", true)]);
        let action = s.handle_key(key(KeyCode::Enter));

        assert!(matches!(action, WorkflowAction::Continue));
        assert!(s.status_msg.contains("ciudad"));
        assert!(matches!(s.sub, WorkflowSubScreen::RunInput));
    }

    #[test]
    fn the_payload_binds_by_name_and_types_numbers() {
        let mut s = state_with(vec![
            field("ciudad", "string", true),
            field("dias", "number", true),
        ]);
        s.run_params[0].value = "Vigo".to_string();
        s.run_params[1].value = "7".to_string();

        let payload: serde_json::Value = serde_json::from_str(&s.build_run_input()).unwrap();
        assert_eq!(payload["ciudad"], serde_json::json!("Vigo"));
        assert_eq!(payload["dias"], serde_json::json!(7.0));
    }

    #[test]
    fn a_non_numeric_number_parameter_falls_back_to_a_string() {
        let mut s = state_with(vec![field("dias", "number", true)]);
        s.run_params[0].value = "seven".to_string();

        let payload: serde_json::Value = serde_json::from_str(&s.build_run_input()).unwrap();
        assert_eq!(payload["dias"], serde_json::json!("seven"));
    }

    #[test]
    fn boolean_parameters_coerce_the_canonical_set_only() {
        let mut s = state_with(vec![field("reintentar", "boolean", false)]);
        s.run_params[0].value = "yes".to_string();
        let payload: serde_json::Value = serde_json::from_str(&s.build_run_input()).unwrap();
        assert_eq!(payload["reintentar"], serde_json::json!(true));

        s.run_params[0].value = "si".to_string();
        let payload: serde_json::Value = serde_json::from_str(&s.build_run_input()).unwrap();
        assert_eq!(payload["reintentar"], serde_json::json!(false));
        assert!(payload.get("input").is_none());
    }

    #[test]
    fn an_unknown_param_type_stays_a_string() {
        let mut s = state_with(vec![field("objetivo", "agent_id", true)]);
        s.run_params[0].value = "writer".to_string();

        let payload: serde_json::Value = serde_json::from_str(&s.build_run_input()).unwrap();
        assert_eq!(payload["objetivo"], serde_json::json!("writer"));
    }

    #[test]
    fn a_blank_optional_parameter_does_not_block_the_run() {
        let mut s = state_with(vec![field("nota", "string", false)]);
        let action = s.handle_key(key(KeyCode::Enter));

        assert!(matches!(action, WorkflowAction::RunWorkflow { .. }));
    }

    #[test]
    fn a_declared_input_parameter_is_not_clobbered_by_the_free_text_box() {
        let mut s = state_with(vec![field("input", "string", false)]);
        s.run_params[0].value = "declared wins".to_string();
        s.run_input = "free text".to_string();

        let payload: serde_json::Value = serde_json::from_str(&s.build_run_input()).unwrap();
        assert_eq!(payload["input"], serde_json::json!("declared wins"));
    }

    #[test]
    fn a_workflow_with_no_declared_parameters_still_sends_the_bare_string() {
        let mut s = state_with(vec![]);
        s.run_input = "texto libre".to_string();
        assert_eq!(s.build_run_input(), "texto libre");
    }
}
