//! Goals screen: browse, create, manage, and run goals.

use crate::tui::{theme, widgets};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, ListItem, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct GoalInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub progress: u8,
    pub agent_id: Option<String>,
    pub loop_engineering: bool,
    pub verify_agent_id: Option<String>,
    pub run_phase: Option<String>,
    pub run_iteration: Option<u32>,
    pub run_max_iterations: Option<u32>,
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
    pub create_loop_engineering: bool,
    pub create_verify_agent_id: String,
    pub status_msg: String,
    pub confirm_delete: bool,
}

#[allow(dead_code)]
pub enum GoalsAction {
    Continue,
    Refresh,
    CreateGoal {
        title: String,
        description: String,
        agent_id: String,
        loop_engineering: bool,
        verify_agent_id: String,
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
    ShowDetail {
        goal_id: String,
    },
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
            create_loop_engineering: false,
            create_verify_agent_id: String::new(),
            status_msg: String::new(),
            confirm_delete: false,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
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
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
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
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_delete = false;
                    if let Some(sel) = self.list_state.selected() {
                        if let Some(&idx) = self.filtered.get(sel) {
                            let id = self.goals[idx].id.clone();
                            return GoalsAction::DeleteGoal { goal_id: id };
                        }
                    }
                }
                _ => {
                    self.confirm_delete = false;
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
                self.create_loop_engineering = false;
                self.create_verify_agent_id.clear();
            }
            KeyCode::Char('d') if self.list_state.selected().is_some() => {
                self.confirm_delete = true;
            }
            KeyCode::Char('s') => {
                if let Some(sel) = self.list_state.selected() {
                    if let Some(&idx) = self.filtered.get(sel) {
                        let g = &self.goals[idx];
                        if g.run_phase.as_deref() == Some("running") {
                            return GoalsAction::StopRun {
                                goal_id: g.id.clone(),
                            };
                        } else {
                            return GoalsAction::StartRun {
                                goal_id: g.id.clone(),
                            };
                        }
                    }
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
                    if idx < self.goals.len() {
                        let g = &self.goals[idx];
                        if g.run_phase.as_deref() == Some("running") {
                            return GoalsAction::StopRun {
                                goal_id: g.id.clone(),
                            };
                        } else {
                            return GoalsAction::StartRun {
                                goal_id: g.id.clone(),
                            };
                        }
                    }
                }
            }
            KeyCode::Char('r') => return GoalsAction::Refresh,
            _ => {}
        }
        GoalsAction::Continue
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
                if self.create_step >= 4 {
                    let action = GoalsAction::CreateGoal {
                        title: self.create_title.clone(),
                        description: self.create_desc.clone(),
                        agent_id: self.create_agent_id.clone(),
                        loop_engineering: self.create_loop_engineering,
                        verify_agent_id: self.create_verify_agent_id.clone(),
                    };
                    self.create_open = false;
                    return action;
                }
                self.create_step += 1;
            }
            KeyCode::Tab | KeyCode::Char(' ') if self.create_step == 3 => {
                self.create_loop_engineering = !self.create_loop_engineering;
            }
            KeyCode::Char(c) => match self.create_step {
                0 => self.create_title.push(c),
                1 => self.create_desc.push(c),
                2 => self.create_agent_id.push(c),
                4 => self.create_verify_agent_id.push(c),
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
                4 => {
                    self.create_verify_agent_id.pop();
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
    let inner = widgets::render_screen_block(f, area, &format!("\u{2316} Goals"));

    if state.create_open {
        draw_create(f, inner, state);
    } else if state.detail_open {
        draw_split(f, inner, state);
    } else {
        draw_list(f, inner, state);
    }
}

fn draw_split(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let chunks = Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(area);

    draw_list_panel(f, chunks[0], state);
    draw_detail(f, chunks[1], state);
}

fn draw_list_panel(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    // Header with count
    if state.search_mode {
        f.render_widget(widgets::search_input(&state.search_buf), chunks[0]);
    } else {
        let search_hint = if state.search_buf.is_empty() {
            String::new()
        } else {
            format!("  filter: {}", state.search_buf)
        };
        f.render_widget(
            Paragraph::new(vec![Line::from(vec![
                Span::styled(
                    format!("  {} goals", state.filtered.len()),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
                Span::styled(search_hint, theme::dim_style()),
            ])]),
            chunks[0],
        );
    }

    // List
    if state.loading {
        f.render_widget(widgets::spinner(state.tick, "Loading goals..."), chunks[1]);
    } else if state.filtered.is_empty() {
        f.render_widget(widgets::empty_state("No goals found."), chunks[1]);
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

    // Hints
    f.render_widget(
        widgets::status_or_hint(
            &state.status_msg,
            "  / search | n new | d delete | s start/stop | Enter detail | r refresh",
        ),
        chunks[2],
    );
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    draw_list_panel(f, area, state);
}

fn draw_detail(f: &mut Frame, area: Rect, state: &mut GoalsState) {
    let idx = match state.selected_goal {
        Some(i) if i < state.goals.len() => i,
        _ => {
            f.render_widget(widgets::empty_state("No goal selected."), area);
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

    // Body: description, agent, status, progress, run info
    let (badge, badge_style) = goal_status_badge(&g.status);
    let agent = g.agent_id.as_deref().unwrap_or("(none)");
    let loop_eng = if g.loop_engineering { "yes" } else { "no" };
    let verify_agent = g.verify_agent_id.as_deref().unwrap_or("(none)");

    let mut lines = vec![
        Line::from(vec![
            Span::styled("  Status: ", theme::dim_style()),
            Span::styled(badge, badge_style),
        ]),
        Line::from(vec![Span::styled("  Progress: ", theme::dim_style())]),
    ];

    // Progress bar
    let pct = g.progress.min(100);
    let _gauge = Gauge::default()
        .gauge_style(Style::default().fg(theme::ACCENT))
        .percent(pct as u16)
        .label(format!(" {pct}%"));
    // gauge doesn't fit as a Line, render after the text block
    // We'll render it below the Paragraph using a separate area

    lines.push(Line::from(vec![
        Span::styled("  Description: ", theme::dim_style()),
        Span::styled(
            widgets::truncate(&g.description, 40),
            Style::default().fg(theme::TEXT_SECONDARY),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Agent: ", theme::dim_style()),
        Span::styled(agent, Style::default().fg(theme::CYAN)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Loop Engineering: ", theme::dim_style()),
        Span::styled(loop_eng, Style::default().fg(theme::YELLOW)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Verify Agent: ", theme::dim_style()),
        Span::styled(verify_agent, Style::default().fg(theme::TEXT_SECONDARY)),
    ]));

    if let Some(ref phase) = g.run_phase {
        let iter = g.run_iteration.unwrap_or(0);
        let max_iter = g
            .run_max_iterations
            .map(|m| m.to_string())
            .unwrap_or_default();
        let phase_style = if phase == "running" {
            Style::default().fg(theme::GREEN)
        } else if phase == "finished" {
            Style::default().fg(theme::ACCENT)
        } else {
            theme::dim_style()
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Run Phase: ", theme::dim_style()),
            Span::styled(phase, phase_style),
        ]));
        if !max_iter.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Iteration: ", theme::dim_style()),
                Span::styled(
                    format!("{iter}/{max_iter}"),
                    Style::default().fg(theme::TEXT_SECONDARY),
                ),
            ]));
        }
    }

    let text_area = chunks[2];
    let (text_top, gauge_area) = {
        let ch = Layout::vertical([
            Constraint::Length(lines.len() as u16 + 1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(text_area);
        (ch[0], ch[1])
    };

    f.render_widget(Paragraph::new(lines), text_top);

    // Render progress gauge separately
    let pct = g.progress.min(100);
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(theme::ACCENT))
            .percent(pct as u16)
            .label(format!(" {}% ", pct)),
        gauge_area,
    );

    let run_hint = if g.run_phase.as_deref() == Some("running") {
        "s stop run"
    } else {
        "s start run"
    };
    let hint = format!("  q/Esc close | {run_hint} | r refresh");
    f.render_widget(widgets::hint_bar(&hint), chunks[3]);
}

fn draw_create(f: &mut Frame, area: Rect, state: &GoalsState) {
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
            Span::styled("  \u{2316} ", Style::default().fg(theme::ACCENT)),
            Span::styled(
                "New Goal",
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    // Step progress indicator
    let progress: Vec<Span> = (0..5)
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
        format!("  step {}/5", state.create_step + 1),
        Style::default().fg(theme::TEXT_SECONDARY),
    ));
    f.render_widget(Paragraph::new(Line::from(step_line)), chunks[2]);

    let loop_label = if state.create_loop_engineering {
        "enabled"
    } else {
        "disabled"
    };
    let (label, value): (&str, &str) = match state.create_step {
        0 => ("Title:", &state.create_title),
        1 => ("Description:", &state.create_desc),
        2 => ("Agent ID:", &state.create_agent_id),
        3 => ("Loop Engineering:", loop_label),
        _ => ("Verify Agent ID:", &state.create_verify_agent_id),
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {label}"),
            Style::default().fg(theme::TEXT_PRIMARY),
        )])),
        chunks[4],
    );

    if state.create_step == 3 {
        // Checkbox / toggle for loop_engineering
        let toggle = if state.create_loop_engineering {
            "\u{25a3} enabled"
        } else {
            "\u{25a1} disabled"
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{276f} ", Style::default().fg(theme::ACCENT)),
                Span::styled(toggle, theme::input_style()),
            ])),
            chunks[6],
        );
    } else {
        let display = if value.is_empty() {
            match state.create_step {
                0 => "e.g. Improve code quality",
                1 => "e.g. Run linters and tests on every PR",
                2 => "e.g. 550e8400-e29b-41d4-a716-446655440000",
                _ => "e.g. 550e8400-e29b-41d4-a716-446655440000",
            }
        } else {
            value
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
    }

    let hint_text = if state.create_step >= 4 {
        "Esc back | Enter submit"
    } else if state.create_step == 3 {
        "Esc back | Space/Tab toggle | Enter next"
    } else {
        "Esc back | Enter next"
    };
    f.render_widget(widgets::hint_bar(hint_text), chunks[8]);
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Map a goal status string to a (badge_text, style) pair.
fn goal_status_badge(status: &str) -> (&'static str, Style) {
    let lower = status.to_lowercase();
    if lower.contains("in_progress") || lower.contains("running") || lower.contains("active") {
        (
            "\u{25cf} ACTV",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )
    } else if lower.contains("completed") || lower.contains("done") {
        ("\u{25cb} DONE", Style::default().fg(theme::ACCENT_DIM))
    } else if lower.contains("cancelled") || lower.contains("cancel") {
        ("\u{25cb} CANC", Style::default().fg(theme::TEXT_TERTIARY))
    } else if lower.contains("failed") || lower.contains("error") {
        (
            "\u{25cf} FAIL",
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        )
    } else {
        // pending / default
        ("\u{25cb} PEND", Style::default().fg(theme::YELLOW))
    }
}
