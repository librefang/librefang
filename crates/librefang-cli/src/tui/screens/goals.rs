//! Goals screen: browse, create, run, and delete autonomous goals.
//!
//! Mirrors the `/api/goals` surface the daemon exposes.
//! A goal document carries only its own fields; the live run state (phase, iteration, cap) lives in the kernel's run registry and is served separately by `GET /api/goals/{id}/run`, so the detail pane fetches it on open rather than expecting it inside the list payload.

use crate::tui::{theme, widgets};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, ListItem, Paragraph};
use ratatui::Frame;

/// Number of fields in the create wizard: title, description, agent.
pub const CREATE_STEPS: usize = 3;

// ── Data types ──────────────────────────────────────────────────────────────

/// One goal as shown in the list, plus the run state once the detail pane has fetched it.
#[derive(Clone, Default)]
pub struct GoalInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub progress: u8,
    pub agent_id: Option<String>,
    /// Live run phase, populated by [`GoalsAction::ShowDetail`]; `None` until then.
    pub run_phase: Option<String>,
    pub run_iteration: Option<u32>,
    pub run_max_iterations: Option<u32>,
}

impl GoalInfo {
    /// Whether the kernel reports an actively running loop for this goal.
    ///
    /// Drives the start/stop toggle, so it keys off the run registry's phase rather than the goal document's `status` — `status` is flipped to `in_progress` when a run starts but is never flipped back when the loop ends, so it would keep offering "stop" for a finished run.
    pub fn is_running(&self) -> bool {
        self.run_phase.as_deref() == Some("running")
    }
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct GoalsState {
    pub goals: Vec<GoalInfo>,
    pub filtered: Vec<usize>,
    pub list_state: ratatui::widgets::ListState,
    pub search_buf: String,
    pub search_mode: bool,
    pub loading: bool,
    pub tick: usize,
    pub detail_open: bool,
    pub selected_goal: Option<usize>,
    pub create_open: bool,
    pub create_step: usize,
    pub create_title: String,
    pub create_desc: String,
    pub create_agent_id: String,
    pub status_msg: String,
    pub confirm_delete: bool,
}

/// What the surrounding app should do after a key press.
pub enum GoalsAction {
    Continue,
    Refresh,
    CreateGoal {
        title: String,
        description: String,
        agent_id: String,
    },
    StartRun {
        goal_id: String,
    },
    StopRun {
        goal_id: String,
    },
    DeleteGoal {
        goal_id: String,
    },
    /// Detail pane opened — fetch this goal's live run state.
    ShowDetail {
        goal_id: String,
    },
}

impl Default for GoalsState {
    fn default() -> Self {
        Self::new()
    }
}

impl GoalsState {
    pub fn new() -> Self {
        Self {
            goals: Vec::new(),
            filtered: Vec::new(),
            list_state: ratatui::widgets::ListState::default(),
            search_buf: String::new(),
            search_mode: false,
            loading: false,
            tick: 0,
            detail_open: false,
            selected_goal: None,
            create_open: false,
            create_step: 0,
            create_title: String::new(),
            create_desc: String::new(),
            create_agent_id: String::new(),
            status_msg: String::new(),
            confirm_delete: false,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Merge a freshly fetched run state into the matching goal.
    pub fn apply_run_state(
        &mut self,
        goal_id: &str,
        phase: Option<String>,
        iteration: Option<u32>,
        max_iterations: Option<u32>,
    ) {
        if let Some(g) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            g.run_phase = phase;
            g.run_iteration = iteration;
            g.run_max_iterations = max_iterations;
        }
    }

    pub fn refilter(&mut self) {
        if self.search_buf.is_empty() {
            self.filtered = (0..self.goals.len()).collect();
        } else {
            let q = self.search_buf.to_lowercase();
            self.filtered = self
                .goals
                .iter()
                .enumerate()
                .filter(|(_, g)| {
                    g.title.to_lowercase().contains(&q)
                        || g.agent_id
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    /// The goal currently highlighted in the list, if any.
    fn selected_in_list(&self) -> Option<&GoalInfo> {
        let sel = self.list_state.selected()?;
        let idx = *self.filtered.get(sel)?;
        self.goals.get(idx)
    }

    /// Start or stop `goal`, whichever its live phase calls for.
    fn toggle_run(goal: &GoalInfo) -> GoalsAction {
        if goal.is_running() {
            GoalsAction::StopRun {
                goal_id: goal.id.clone(),
            }
        } else {
            GoalsAction::StartRun {
                goal_id: goal.id.clone(),
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> GoalsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return GoalsAction::Continue;
        }

        if self.create_open {
            return self.handle_create_key(key);
        }

        if self.detail_open {
            return self.handle_detail_key(key);
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
            return GoalsAction::Continue;
        }

        if self.confirm_delete {
            self.confirm_delete = false;
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                if let Some(g) = self.selected_in_list() {
                    return GoalsAction::DeleteGoal {
                        goal_id: g.id.clone(),
                    };
                }
            }
            return GoalsAction::Continue;
        }

        let total = self.filtered.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.list_state.select(Some(next));
            }
            KeyCode::Enter => {
                if let Some(sel) = self.list_state.selected() {
                    if let Some(&idx) = self.filtered.get(sel) {
                        self.selected_goal = Some(idx);
                        self.detail_open = true;
                        return GoalsAction::ShowDetail {
                            goal_id: self.goals[idx].id.clone(),
                        };
                    }
                }
            }
            KeyCode::Char('n') => {
                self.create_open = true;
                self.create_step = 0;
                self.create_title.clear();
                self.create_desc.clear();
                self.create_agent_id.clear();
            }
            KeyCode::Char('d') if self.list_state.selected().is_some() => {
                self.confirm_delete = true;
            }
            KeyCode::Char('s') => {
                if let Some(g) = self.selected_in_list() {
                    return Self::toggle_run(g);
                }
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_buf.clear();
            }
            KeyCode::Char('r') => return GoalsAction::Refresh,
            _ => {}
        }
        GoalsAction::Continue
    }

    fn handle_detail_key(&mut self, key: KeyEvent) -> GoalsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.detail_open = false;
            }
            KeyCode::Char('s') => {
                if let Some(idx) = self.selected_goal {
                    if let Some(g) = self.goals.get(idx) {
                        return Self::toggle_run(g);
                    }
                }
            }
            KeyCode::Char('r') => return GoalsAction::Refresh,
            _ => {}
        }
        GoalsAction::Continue
    }

    /// Whether the create wizard can be submitted: title and agent are both required.
    ///
    /// The daemon refuses to start a run on a goal with no agent assigned, so submitting without one would create a goal that can never run.
    pub fn create_is_submittable(&self) -> bool {
        !self.create_title.trim().is_empty() && !self.create_agent_id.trim().is_empty()
    }

    fn handle_create_key(&mut self, key: KeyEvent) -> GoalsAction {
        match key.code {
            KeyCode::Esc => {
                if self.create_step == 0 {
                    self.create_open = false;
                } else {
                    self.create_step -= 1;
                }
            }
            KeyCode::Enter => {
                if self.create_step + 1 < CREATE_STEPS {
                    self.create_step += 1;
                } else if self.create_is_submittable() {
                    let action = GoalsAction::CreateGoal {
                        title: self.create_title.trim().to_string(),
                        description: self.create_desc.trim().to_string(),
                        agent_id: self.create_agent_id.trim().to_string(),
                    };
                    self.create_open = false;
                    return action;
                } else {
                    self.status_msg = crate::i18n::t("tui-goals-create-incomplete");
                }
            }
            KeyCode::Char(c) => match self.create_step {
                0 => self.create_title.push(c),
                1 => self.create_desc.push(c),
                2 => self.create_agent_id.push(c),
                _ => {}
            },
            KeyCode::Backspace => match self.create_step {
                0 => {
                    self.create_title.pop();
                }
                1 => {
                    self.create_desc.pop();
                }
                2 => {
                    self.create_agent_id.pop();
                }
                _ => {}
            },
            _ => {}
        }
        GoalsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let inner = widgets::render_screen_block(f, area, &crate::i18n::t("tui-goals-title"));

    if state.create_open {
        draw_create(f, inner, state);
    } else if state.detail_open {
        draw_split(f, inner, state);
    } else {
        draw_list_panel(f, inner, state);
    }
}

fn draw_split(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let chunks = Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(area);

    draw_list_panel(f, chunks[0], state);
    draw_detail(f, chunks[1], state);
}

/// A dimmed `label:` prefix. The colon lives inside the translated value so a
/// locale can punctuate it its own way; only the indent is added here.
fn label_span(key: &str) -> Span<'static> {
    Span::styled(format!("  {} ", crate::i18n::t(key)), theme::dim_style())
}

fn draw_list_panel(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    if state.search_mode {
        f.render_widget(widgets::search_input(&state.search_buf), chunks[0]);
    } else {
        let search_hint = if state.search_buf.is_empty() {
            String::new()
        } else {
            crate::i18n::t_args("tui-goals-filter", &[("query", &state.search_buf)])
        };
        f.render_widget(
            Paragraph::new(vec![Line::from(vec![
                Span::styled(
                    format!(
                        "  {}",
                        crate::i18n::t_args(
                            "tui-goals-count",
                            &[("count", &state.filtered.len().to_string())],
                        )
                    ),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
                Span::styled(search_hint, theme::dim_style()),
            ])]),
            chunks[0],
        );
    }

    if state.loading {
        let loading_text = crate::i18n::t("tui-goals-loading");
        f.render_widget(widgets::spinner(state.tick, &loading_text), chunks[1]);
    } else if state.filtered.is_empty() {
        let empty_text = crate::i18n::t("tui-goals-empty");
        f.render_widget(widgets::empty_state(&empty_text), chunks[1]);
    } else {
        let items: Vec<ListItem> = state
            .filtered
            .iter()
            .map(|&idx| {
                let g = &state.goals[idx];
                let (badge, badge_style) = goal_status_badge(&g.status);
                let title_display = widgets::truncate(&g.title, 22);
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(format!("{:<8}", badge), badge_style),
                    Span::styled(" ", Style::default()),
                    Span::styled(
                        format!("{:<22}", title_display),
                        Style::default().fg(theme::TEXT_PRIMARY),
                    ),
                    Span::styled(" ", Style::default()),
                    Span::styled(
                        format!("{:>3}%", g.progress),
                        Style::default().fg(theme::ACCENT_DIM),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    let hint = if state.confirm_delete {
        crate::i18n::t("tui-goals-confirm-delete")
    } else {
        crate::i18n::t("tui-goals-hints")
    };
    f.render_widget(widgets::status_or_hint(&state.status_msg, &hint), chunks[2]);
}

fn draw_detail(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let idx = match state.selected_goal {
        Some(i) if i < state.goals.len() => i,
        _ => {
            f.render_widget(
                widgets::empty_state(&crate::i18n::t("tui-goals-none-selected")),
                area,
            );
            return;
        }
    };
    let g = &state.goals[idx];

    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // separator
        Constraint::Min(3),    // body
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{2316} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                widgets::truncate(&g.title, 36),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    let (badge, badge_style) = goal_status_badge(&g.status);
    let unassigned = crate::i18n::t("tui-goals-agent-none");
    let agent = g.agent_id.as_deref().unwrap_or(&unassigned);

    let mut lines = vec![
        Line::from(vec![
            label_span("tui-goals-label-status"),
            Span::styled(badge, badge_style),
        ]),
        Line::from(vec![
            label_span("tui-goals-label-description"),
            Span::styled(
                widgets::truncate(&g.description, 40),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
        ]),
        Line::from(vec![
            label_span("tui-goals-label-agent"),
            Span::styled(agent.to_string(), Style::default().fg(theme::CYAN)),
        ]),
        Line::from(vec![label_span("tui-goals-label-progress")]),
    ];

    if let Some(ref phase) = g.run_phase {
        let phase_style = match phase.as_str() {
            "running" => Style::default().fg(theme::GREEN),
            "paused" => Style::default().fg(theme::YELLOW),
            "finished" => Style::default().fg(theme::ACCENT),
            "max_iterations_reached" => Style::default().fg(theme::YELLOW),
            "rate_limited" => Style::default().fg(theme::RED),
            "stopped" => Style::default().fg(theme::TEXT_TERTIARY),
            _ => theme::dim_style(),
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            label_span("tui-goals-label-phase"),
            Span::styled(translate_phase(phase), phase_style),
        ]));
        if let Some(max_iter) = g.run_max_iterations {
            let iter = g.run_iteration.unwrap_or(0);
            lines.push(Line::from(vec![
                label_span("tui-goals-label-iteration"),
                Span::styled(
                    format!("{iter}/{max_iter}"),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ]));
        }
    }

    let ch = Layout::vertical([
        Constraint::Length(lines.len() as u16 + 1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(chunks[2]);

    f.render_widget(Paragraph::new(lines), ch[0]);

    let pct = g.progress.min(100);
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(theme::ACCENT))
            .percent(pct as u16)
            .label(format!(" {}% ", pct)),
        ch[1],
    );

    let run_hint = if g.is_running() {
        crate::i18n::t("tui-goals-hint-stop")
    } else {
        crate::i18n::t("tui-goals-hint-start")
    };
    let hint = crate::i18n::t_args("tui-goals-detail-hints", &[("run_hint", &run_hint)]);
    f.render_widget(widgets::hint_bar(&hint), chunks[3]);
}

fn draw_create(f: &mut Frame, area: Rect, state: &GoalsState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // separator
        Constraint::Length(1), // step progress
        Constraint::Length(1), // spacer
        Constraint::Length(1), // field label
        Constraint::Length(1), // input
        Constraint::Length(1), // info hint
        Constraint::Min(0),
        Constraint::Length(1), // nav hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{2316} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t("tui-goals-new-title"),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    let dots: Vec<Span> = (0..CREATE_STEPS)
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
    step_line.extend(dots);
    step_line.push(Span::styled(
        crate::i18n::t_args(
            "tui-goals-step",
            &[
                ("n", &(state.create_step + 1).to_string()),
                ("total", &CREATE_STEPS.to_string()),
            ],
        ),
        Style::default().fg(theme::TEXT_SECONDARY),
    ));
    f.render_widget(Paragraph::new(Line::from(step_line)), chunks[2]);

    let (label_key, value, hint_key) = match state.create_step {
        0 => (
            "tui-goals-label-title",
            &state.create_title,
            "tui-goals-title-hint",
        ),
        1 => (
            "tui-goals-label-description",
            &state.create_desc,
            "tui-goals-description-hint",
        ),
        _ => (
            "tui-goals-label-agent",
            &state.create_agent_id,
            "tui-goals-agent-hint",
        ),
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {}", crate::i18n::t(label_key)),
            Style::default().fg(theme::TEXT_PRIMARY),
        )])),
        chunks[4],
    );

    let display = if value.is_empty() {
        Span::styled("  \u{258c}", Style::default().fg(theme::TEXT_TERTIARY))
    } else {
        Span::styled(
            format!("  {value}"),
            Style::default().fg(theme::TEXT_PRIMARY),
        )
    };
    f.render_widget(Paragraph::new(Line::from(vec![display])), chunks[5]);

    // Info hint: the variable-shaped detail lives here, never in the label.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  \u{24d8} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                crate::i18n::t(hint_key),
                Style::default().fg(theme::TEXT_SECONDARY),
            ),
        ])),
        chunks[6],
    );

    let nav_key = if state.create_step + 1 < CREATE_STEPS {
        "tui-goals-nav-next"
    } else {
        "tui-goals-nav-submit"
    };
    f.render_widget(widgets::hint_bar(&crate::i18n::t(nav_key)), chunks[8]);
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Map a goal status string to a (badge_text, style) pair.
pub fn goal_status_badge(status: &str) -> (String, Style) {
    let lower = status.to_lowercase();
    if lower.contains("in_progress") || lower.contains("running") || lower.contains("active") {
        (
            crate::i18n::t("tui-goals-phase-actv"),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )
    } else if lower.contains("completed") || lower.contains("done") {
        (
            crate::i18n::t("tui-goals-phase-done"),
            Style::default().fg(theme::ACCENT_DIM),
        )
    } else if lower.contains("cancelled") || lower.contains("cancel") {
        (
            crate::i18n::t("tui-goals-phase-canc"),
            Style::default().fg(theme::TEXT_TERTIARY),
        )
    } else if lower.contains("paused") || lower.contains("stopped") {
        (
            crate::i18n::t("tui-goals-phase-paused"),
            Style::default().fg(theme::YELLOW),
        )
    } else if lower.contains("rate_limited") {
        (
            crate::i18n::t("tui-goals-phase-rate-limited"),
            Style::default().fg(theme::RED),
        )
    } else if lower.contains("max_iterations") {
        (
            crate::i18n::t("tui-goals-phase-max-iterations"),
            Style::default().fg(theme::YELLOW),
        )
    } else if lower.contains("failed") || lower.contains("error") {
        (
            crate::i18n::t("tui-goals-phase-fail"),
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        )
    } else {
        (
            crate::i18n::t("tui-goals-phase-pend"),
            Style::default().fg(theme::YELLOW),
        )
    }
}

/// Locale key for a `GoalRunPhase` wire value, or `None` if unrecognised.
fn phase_message_key(phase: &str) -> Option<&'static str> {
    match phase {
        "running" => Some("tui-goals-run-running"),
        "paused" => Some("tui-goals-run-paused"),
        "finished" => Some("tui-goals-run-finished"),
        "max_iterations_reached" => Some("tui-goals-run-max-iterations"),
        "rate_limited" => Some("tui-goals-run-rate-limited"),
        "stopped" => Some("tui-goals-run-stopped"),
        _ => None,
    }
}

/// Human-readable name for a `GoalRunPhase` wire value.
///
/// The wire values are the kernel's `snake_case` enum names; an operator should
/// see prose, so each maps to its own message and anything unrecognised falls
/// back to the raw value rather than an empty cell.
pub fn translate_phase(phase: &str) -> String {
    match phase_message_key(phase) {
        Some(key) => crate::i18n::t(key),
        None => phase.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(id: &str, title: &str, agent: Option<&str>) -> GoalInfo {
        GoalInfo {
            id: id.to_string(),
            title: title.to_string(),
            agent_id: agent.map(|a| a.to_string()),
            ..Default::default()
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn state_with(goals: Vec<GoalInfo>) -> GoalsState {
        let mut s = GoalsState::new();
        s.goals = goals;
        s.refilter();
        s
    }

    #[test]
    fn refilter_matches_title_and_agent() {
        let mut s = state_with(vec![
            goal("1", "Fix login", Some("alpha")),
            goal("2", "Write docs", Some("beta")),
        ]);

        s.search_buf = "login".to_string();
        s.refilter();
        assert_eq!(s.filtered, vec![0]);

        s.search_buf = "beta".to_string();
        s.refilter();
        assert_eq!(s.filtered, vec![1]);

        s.search_buf.clear();
        s.refilter();
        assert_eq!(s.filtered, vec![0, 1]);
    }

    #[test]
    fn refilter_clears_selection_when_nothing_matches() {
        let mut s = state_with(vec![goal("1", "Fix login", None)]);
        s.search_buf = "nothing-matches-this".to_string();
        s.refilter();
        assert!(s.filtered.is_empty());
        assert_eq!(s.list_state.selected(), None);
    }

    #[test]
    fn navigation_wraps_both_ways() {
        let mut s = state_with(vec![
            goal("1", "a b", None),
            goal("2", "c d", None),
            goal("3", "e f", None),
        ]);
        assert_eq!(s.list_state.selected(), Some(0));

        s.handle_key(key(KeyCode::Up));
        assert_eq!(
            s.list_state.selected(),
            Some(2),
            "up from first wraps to last"
        );

        s.handle_key(key(KeyCode::Down));
        assert_eq!(
            s.list_state.selected(),
            Some(0),
            "down from last wraps to first"
        );
    }

    #[test]
    fn start_stop_toggle_follows_run_phase_not_status() {
        // `status` stays "in_progress" after a run ends, so only the live run
        // phase may decide which action the `s` key sends.
        let mut finished = goal("1", "a b", None);
        finished.status = "in_progress".to_string();
        finished.run_phase = Some("finished".to_string());
        let mut s = state_with(vec![finished]);

        assert!(
            matches!(
                s.handle_key(key(KeyCode::Char('s'))),
                GoalsAction::StartRun { .. }
            ),
            "a finished run must offer start, not stop"
        );

        s.goals[0].run_phase = Some("running".to_string());
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('s'))),
            GoalsAction::StopRun { .. }
        ));
    }

    #[test]
    fn enter_opens_detail_and_requests_run_state() {
        let mut s = state_with(vec![goal("goal-1", "a b", None)]);
        match s.handle_key(key(KeyCode::Enter)) {
            GoalsAction::ShowDetail { goal_id } => assert_eq!(goal_id, "goal-1"),
            _ => panic!("Enter must request the run state for the detail pane"),
        }
        assert!(s.detail_open);
    }

    #[test]
    fn apply_run_state_populates_only_the_matching_goal() {
        let mut s = state_with(vec![goal("1", "a b", None), goal("2", "c d", None)]);
        s.apply_run_state("2", Some("running".to_string()), Some(3), Some(25));

        assert!(s.goals[0].run_phase.is_none());
        assert!(s.goals[1].is_running());
        assert_eq!(s.goals[1].run_iteration, Some(3));
        assert_eq!(s.goals[1].run_max_iterations, Some(25));
    }

    #[test]
    fn delete_needs_confirmation() {
        let mut s = state_with(vec![goal("1", "a b", None)]);

        assert!(matches!(
            s.handle_key(key(KeyCode::Char('d'))),
            GoalsAction::Continue
        ));
        assert!(s.confirm_delete);

        assert!(matches!(
            s.handle_key(key(KeyCode::Char('n'))),
            GoalsAction::Continue
        ));
        assert!(!s.confirm_delete, "any key but y cancels");

        s.handle_key(key(KeyCode::Char('d')));
        match s.handle_key(key(KeyCode::Char('y'))) {
            GoalsAction::DeleteGoal { goal_id } => assert_eq!(goal_id, "1"),
            _ => panic!("y must confirm the delete"),
        }
    }

    #[test]
    fn create_wizard_walks_all_steps_then_submits() {
        let mut s = state_with(vec![]);
        s.handle_key(key(KeyCode::Char('n')));
        assert!(s.create_open);

        for c in "Ship".chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }
        s.handle_key(key(KeyCode::Enter));
        for c in "Do it".chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }
        s.handle_key(key(KeyCode::Enter));
        for c in "agent-7".chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }

        match s.handle_key(key(KeyCode::Enter)) {
            GoalsAction::CreateGoal {
                title,
                description,
                agent_id,
            } => {
                assert_eq!(title, "Ship");
                assert_eq!(description, "Do it");
                assert_eq!(agent_id, "agent-7");
            }
            _ => panic!("the final Enter must submit"),
        }
        assert!(!s.create_open);
    }

    #[test]
    fn create_wizard_refuses_to_submit_without_an_agent() {
        // The daemon rejects a run whose goal has no agent, so a goal created
        // without one could never start.
        let mut s = state_with(vec![]);
        s.handle_key(key(KeyCode::Char('n')));
        for c in "Ship".chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }
        s.create_step = CREATE_STEPS - 1;

        assert!(matches!(
            s.handle_key(key(KeyCode::Enter)),
            GoalsAction::Continue
        ));
        assert!(s.create_open, "the wizard stays open on an incomplete form");
        assert!(!s.status_msg.is_empty(), "and explains why");
    }

    #[test]
    fn esc_steps_back_through_the_wizard_then_closes_it() {
        let mut s = state_with(vec![]);
        s.handle_key(key(KeyCode::Char('n')));
        s.create_step = 2;

        s.handle_key(key(KeyCode::Esc));
        assert_eq!(s.create_step, 1);
        s.handle_key(key(KeyCode::Esc));
        assert_eq!(s.create_step, 0);
        s.handle_key(key(KeyCode::Esc));
        assert!(!s.create_open);
    }

    #[test]
    fn search_mode_captures_typing_and_esc_restores_the_full_list() {
        let mut s = state_with(vec![goal("1", "Fix login", None), goal("2", "Docs", None)]);
        s.handle_key(key(KeyCode::Char('/')));
        assert!(s.search_mode);

        for c in "Fix".chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(s.filtered, vec![0]);

        s.handle_key(key(KeyCode::Esc));
        assert!(!s.search_mode);
        assert_eq!(s.filtered, vec![0, 1]);
    }

    #[test]
    fn ctrl_c_is_left_for_the_global_handler() {
        let mut s = state_with(vec![goal("1", "a b", None)]);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(s.handle_key(ctrl_c), GoalsAction::Continue));
        assert!(!s.create_open, "Ctrl+C must not be read as a character");
    }

    #[test]
    fn every_run_phase_the_kernel_can_emit_is_mapped() {
        // Mirrors `GoalRunPhase` in librefang-types; an unmapped variant would
        // fall through and surface its raw snake_case name to the operator.
        //
        // This asserts the mapping rather than comparing the rendered text to
        // the wire value: `tui-goals-run-running` is legitimately "running" in
        // English, so a difference check would fail on a correct translation.
        for phase in [
            "running",
            "finished",
            "max_iterations_reached",
            "rate_limited",
            "stopped",
        ] {
            assert!(
                phase_message_key(phase).is_some(),
                "run phase '{phase}' is not mapped to a message"
            );
        }
    }

    #[test]
    fn unknown_run_phase_falls_back_to_the_raw_value() {
        assert!(phase_message_key("brand_new_phase").is_none());
        assert_eq!(translate_phase("brand_new_phase"), "brand_new_phase");
    }
}
