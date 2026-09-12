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
    /// RFC3339 start time, straight from the payload. Sort key for the run
    /// history; not rendered.
    pub started_at: String,
    /// Derived from `started_at` / `completed_at` — the payload carries no
    /// duration of its own. Empty while the run is still in flight.
    pub duration: String,
    /// Steps that have recorded a result (`steps_completed` in the run list).
    pub steps_completed: usize,
    /// 0-based index of the step the run is executing right now.
    /// The daemon gates this on the run's state (`WorkflowRun::live_step_index`),
    /// so it is `None` for anything that is not actually running.
    pub current_step_index: Option<usize>,
    /// Steps the workflow declares. `0` means the daemon has no figure to give:
    /// a run persisted before the column existed, or a workflow with no steps.
    pub total_steps: usize,
}

/// Cells in the inline progress bar drawn beside the `n/m` count.
const PROGRESS_BAR_CELLS: usize = 5;

/// Width of the `Progress` column, in terminal columns.
/// `▰▰▰▰▰ 999/999` is 13, which covers any workflow anyone hand-writes.
const PROGRESS_CELL: usize = 13;

/// Fit a label to exactly [`PROGRESS_CELL`] columns, trimming or padding.
///
/// Both the trim and the pad live here rather than in a `{:<width$}` on the
/// format string, so the column's width is stated once and the header and the
/// rows cannot drift apart.
///
/// `widgets::truncate` is the wrong tool for this cell: it measures bytes, and
/// `▰` is three of them, so a width of 13 would cut every bar after four
/// glyphs. Counting chars is also what `{:<n}` pads by, which is how the
/// neighbouring columns stay aligned.
fn fit_progress_cell(label: &str) -> String {
    let width = label.chars().count();
    if width > PROGRESS_CELL {
        return label
            .chars()
            .take(PROGRESS_CELL - 1)
            .chain(['\u{2026}'])
            .collect();
    }
    format!("{label}{}", " ".repeat(PROGRESS_CELL - width))
}

/// The `Progress` cell for one run — how far it has got, out of how many.
///
/// While a run is executing, `current_step_index` names the step in flight and
/// is rendered 1-based, so a run on its first step reads `1/4` rather than `0/4`.
/// Otherwise the count is the steps that actually produced a result, which stays
/// honest for every way a run can stop: not started reads `0/4`, completed reads
/// `4/4`, failed halfway reads `2/4`.
///
/// `total_steps == 0` covers both a run recovered from before the daemon recorded
/// the figure and a workflow that declares no steps.
/// Neither has a denominator, so both print `?` and no bar — there is no fraction
/// to compute, and computing one anyway is how this would divide by zero.
pub fn run_progress_label(run: &WorkflowRun) -> String {
    let done = match run.current_step_index {
        Some(i) => i.saturating_add(1),
        None => run.steps_completed,
    };
    if run.total_steps == 0 {
        return format!("{}/?", done);
    }
    // `steps_completed` counts step *executions* while `total_steps` counts
    // the steps the workflow declares, and a `StepMode::Loop` step pushes one
    // result per iteration. So `done` routinely runs past the denominator: a
    // 3-step workflow whose middle step loops five times and then fails has 6
    // results against 3 declared steps. Widening the denominator to match
    // would render that failed run as a full bar reading `6/6`. Clamp instead,
    // so the printed denominator stays the one the workflow declares and a run
    // that stopped early cannot claim the whole of it.
    let total = run.total_steps;
    let done = done.min(total);
    let filled = done * PROGRESS_BAR_CELLS / total;
    let bar = format!(
        "{}{}",
        "\u{25b0}".repeat(filled),
        "\u{25b1}".repeat(PROGRESS_BAR_CELLS - filled)
    );
    format!("{} {}/{}", bar, done, total)
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
    /// Re-poll the run list on a timer so step progress advances on screen
    /// without the operator pressing `r`. Toggled with `a`, as on the logs
    /// screen.
    pub auto_refresh: bool,
    pub poll_tick: usize,
    /// A poll is out and has not answered yet.
    ///
    /// `make_daemon_client` allows 5s per request against a 2s interval, so
    /// without this a slow daemon accumulates up to three in-flight requests.
    /// `WorkflowRunsLoaded` is last-write-wins, so a delayed answer overwrites
    /// a newer one and the step counter visibly walks backwards. Skipping the
    /// tick while one is outstanding also caps the cost: each poll makes the
    /// daemon clone every `WorkflowRun` it holds — `step_results`, prompts and
    /// outputs included — for a five-character progress cell.
    pub poll_in_flight: bool,
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
            auto_refresh: true,
            poll_tick: 0,
            poll_in_flight: false,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.poll_tick = self.poll_tick.wrapping_add(1);
    }

    /// The id of the workflow whose runs are on screen, if one is selected.
    pub fn selected_workflow_id(&self) -> Option<String> {
        self.workflows
            .get(self.selected_workflow?)
            .map(|w| w.id.clone())
    }

    /// True when it is time to re-fetch the run list (every ~2s at the 50ms
    /// tick rate). Only while the run history is the visible sub-screen — the
    /// list, the create wizard and the run form neither show progress nor want
    /// their state replaced underneath the operator.
    pub fn should_poll(&self) -> bool {
        self.sub == WorkflowSubScreen::Runs
            && self.auto_refresh
            && !self.poll_in_flight
            && self.poll_tick > 0
            && self.poll_tick.is_multiple_of(40)
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
            KeyCode::Char('a') => {
                self.auto_refresh = !self.auto_refresh;
            }
            KeyCode::Char('r') => {
                if let Some(wf_id) = self.selected_workflow_id() {
                    return WorkflowAction::LoadRuns(wf_id);
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

    let auto_badge = if state.auto_refresh {
        Span::styled(
            format!(
                " {} {}",
                "\u{25cf}",
                crate::i18n::t("tui-workflows-badge-auto")
            ),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(
                " {} {}",
                "\u{25cb}",
                crate::i18n::t("tui-workflows-badge-paused")
            ),
            theme::dim_style(),
        )
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{25b7} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t_args("tui-workflows-title-runs", &[("name", wf_name)]),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            auto_badge,
        ])),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            // No Output column: `GET /api/workflows/{id}/runs` emits no
            // `output` key, so it rendered an empty string on every row.
            // Narrowing it to make room for Progress would have been trading
            // width between a live column and a dead one.
            format!(
                "  {:<12} {:<12} {} {}",
                crate::i18n::t("tui-workflows-header-run-id"),
                crate::i18n::t("tui-workflows-header-state"),
                fit_progress_cell(&crate::i18n::t("tui-workflows-header-progress")),
                crate::i18n::t("tui-workflows-header-duration"),
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
                        // Trimmed like every other cell in the row: `{:<n}`
                        // pads but never trims, so a wide count would push
                        // Duration right and break alignment with the header.
                        format!(" {}", fit_progress_cell(&run_progress_label(run))),
                        // A run still in flight is the one the operator is
                        // watching, so its bar gets the accent; anything
                        // finished states its final count in the muted tone.
                        if run.current_step_index.is_some() {
                            Style::default().fg(theme::ACCENT)
                        } else {
                            Style::default().fg(theme::TEXT_SECONDARY)
                        },
                    ),
                    Span::styled(
                        format!(" {}", run.duration),
                        Style::default().fg(theme::YELLOW),
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

#[cfg(test)]
mod step_progress_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn run(current: Option<usize>, completed: usize, total: usize) -> WorkflowRun {
        WorkflowRun {
            id: "run-1".to_string(),
            state: "running".to_string(),
            started_at: "2026-09-09T10:00:00+00:00".to_string(),
            duration: String::new(),
            steps_completed: completed,
            current_step_index: current,
            total_steps: total,
        }
    }

    /// The run history sub-screen, with `runs` on it, ready to draw.
    fn runs_screen(runs: Vec<WorkflowRun>) -> WorkflowState {
        let mut s = WorkflowState::new();
        s.sub = WorkflowSubScreen::Runs;
        s.workflows.push(WorkflowInfo {
            id: "wf-1".to_string(),
            name: "nightly".to_string(),
            ..Default::default()
        });
        s.selected_workflow = Some(0);
        s.runs = runs;
        s.runs_list_state.select(Some(0));
        s
    }

    fn render(state: &mut WorkflowState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal
            .draw(|f| draw(f, f.area(), state))
            .expect("draw must not panic");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn a_running_run_counts_the_step_in_flight_one_based() {
        // Executing index 0 of 4 is "step 1 of 4", not "step 0 of 4".
        assert!(run_progress_label(&run(Some(0), 0, 4)).ends_with("1/4"));
        assert!(run_progress_label(&run(Some(2), 2, 4)).ends_with("3/4"));
    }

    #[test]
    fn progress_advances_when_the_poll_delivers_a_later_step() {
        let mut state = runs_screen(vec![run(Some(0), 0, 4)]);
        let before = render(&mut state);
        assert!(
            before.contains("1/4"),
            "first draw should show step 1: {before}"
        );

        // What the auto-poll does: the same run comes back further along.
        state.runs = vec![run(Some(2), 2, 4)];
        let after = render(&mut state);
        assert!(after.contains("3/4"), "redraw should show step 3: {after}");
        assert!(!after.contains("1/4"), "stale count must be gone: {after}");
    }

    #[test]
    fn a_finished_run_reports_the_steps_that_produced_a_result() {
        // The daemon clears the live index on every terminal transition, so a
        // completed run counts results, and a run that failed halfway says so
        // rather than claiming the whole workflow.
        assert!(run_progress_label(&run(None, 4, 4)).ends_with("4/4"));
        assert!(run_progress_label(&run(None, 2, 4)).ends_with("2/4"));
        assert!(run_progress_label(&run(None, 0, 4)).ends_with("0/4"));
    }

    #[test]
    fn an_unknown_total_prints_a_question_mark_instead_of_a_fraction() {
        // `total_steps == 0`: a workflow with no steps, or a run persisted
        // before the daemon recorded the figure.
        assert_eq!(run_progress_label(&run(None, 0, 0)), "0/?");
        assert_eq!(run_progress_label(&run(Some(1), 1, 0)), "2/?");
    }

    #[test]
    fn a_zero_step_run_renders_without_dividing_by_zero() {
        let mut state = runs_screen(vec![run(None, 0, 0)]);
        let out = render(&mut state);
        assert!(out.contains("0/?"), "{out}");
    }

    /// `steps_completed` counts executions and `total_steps` counts declared
    /// steps, so a loop step pushes the numerator past the denominator as a
    /// matter of course — a 3-step workflow whose middle step loops five times
    /// and then fails on step 3 has 6 results against 3 declared steps.
    /// Widening the denominator to match painted that failed run as a full bar
    /// reading `6/6`; the count has to be clamped to the declared total
    /// instead, so the run reads `3/3` with the bar full but the denominator
    /// still the workflow's own.
    #[test]
    fn a_loop_run_with_more_results_than_steps_keeps_the_declared_denominator() {
        let label = run_progress_label(&run(None, 6, 3));
        assert!(
            label.ends_with("3/3"),
            "denominator must stay the declared step count: {label}"
        );
        assert_eq!(label.matches('\u{25b0}').count(), PROGRESS_BAR_CELLS);

        // Same for a live index that outran the definition — the bar must not
        // subtract past zero either way.
        let label = run_progress_label(&run(Some(9), 0, 4));
        assert!(label.ends_with("4/4"), "{label}");
        assert_eq!(label.matches('\u{25b0}').count(), PROGRESS_BAR_CELLS);
    }

    /// `widgets::truncate` measures bytes and `▰` is three of them, so the
    /// shared helper would cut every bar after four glyphs. The cell needs a
    /// char-counted trim to stay the width `{:<n}` pads to.
    #[test]
    fn the_progress_cell_is_fitted_by_columns_not_bytes() {
        // 9 columns but 19 bytes, because the bar glyph is three bytes each.
        // A byte-measured fit would trim this; it must only be padded.
        let ordinary = run_progress_label(&run(Some(1), 0, 4));
        assert_eq!(ordinary.chars().count(), 9, "{ordinary}");
        let fitted = fit_progress_cell(&ordinary);
        assert!(
            fitted.starts_with(&ordinary),
            "a label inside the column must survive whole: {fitted:?}"
        );
        assert_eq!(fitted.chars().count(), PROGRESS_CELL);

        let wide = run_progress_label(&run(None, 1234, 1234));
        assert!(wide.chars().count() > PROGRESS_CELL, "{wide}");
        let fitted = fit_progress_cell(&wide);
        assert_eq!(
            fitted.chars().count(),
            PROGRESS_CELL,
            "a wide count must be trimmed to the column, not pushed into Duration"
        );
        assert!(fitted.ends_with('\u{2026}'), "{fitted:?}");
    }

    /// A slow daemon answers a 2s poll interval inside a 5s client timeout, so
    /// without a guard up to three requests are out at once. `WorkflowRunsLoaded`
    /// is last-write-wins, so a delayed answer overwrites a newer one and the
    /// counter walks backwards.
    #[test]
    fn no_second_poll_starts_while_one_is_still_out() {
        let mut state = runs_screen(vec![run(Some(0), 0, 4)]);
        for _ in 0..40 {
            state.tick();
        }
        assert!(state.should_poll(), "the first tick window must poll");

        state.poll_in_flight = true;
        for _ in 0..40 {
            state.tick();
        }
        assert!(
            !state.should_poll(),
            "a poll must not start while one is outstanding"
        );

        state.poll_in_flight = false;
        for _ in 0..40 {
            state.tick();
        }
        assert!(state.should_poll(), "polling resumes once the answer lands");
    }

    #[test]
    fn auto_refresh_polls_the_run_history_and_a_toggles_it_off() {
        let mut state = runs_screen(vec![run(Some(0), 0, 4)]);
        for _ in 0..40 {
            state.tick();
        }
        assert!(state.should_poll());
        assert_eq!(state.selected_workflow_id().as_deref(), Some("wf-1"));

        state.handle_key(key(KeyCode::Char('a')));
        for _ in 0..40 {
            state.tick();
        }
        assert!(!state.should_poll(), "[a] must stop the polling");
    }

    #[test]
    fn only_the_run_history_polls() {
        let mut state = runs_screen(vec![run(Some(0), 0, 4)]);
        state.sub = WorkflowSubScreen::List;
        for _ in 0..40 {
            state.tick();
        }
        assert!(!state.should_poll());
    }
}
