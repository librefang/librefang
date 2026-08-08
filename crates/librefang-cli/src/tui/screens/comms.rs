//! Comms screen: Agent communication topology + live event feed.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct CommsNode {
    pub id: String,
    pub name: String,
    pub state: String,
    pub model: String,
}

#[derive(Clone, Default)]
pub struct CommsEdge {
    pub from: String,
    pub to: String,
    pub kind: String, // "parent_child" or "peer"
}

#[derive(Clone, Default)]
pub struct CommsEventItem {
    /// Event ID — used by the dashboard for dedup, kept for wire compat.
    #[allow(dead_code)]
    pub id: String,
    pub timestamp: String,
    pub kind: String,
    pub source_name: String,
    pub target_name: String,
    pub detail: String,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommsFocus {
    Topology,
    EventList,
    Channels,
    ChannelAdd,
}

#[derive(Clone, Default)]
pub struct ChannelInfo {
    pub adapter: String,
    pub default_agent: String,
    pub token_set: bool,
    pub enabled: bool,
    #[allow(dead_code)]
    pub index: usize, // position in config array
}

pub struct CommsState {
    pub nodes: Vec<CommsNode>,
    pub edges: Vec<CommsEdge>,
    pub events: Vec<CommsEventItem>,
    pub event_list_state: ListState,
    pub focus: CommsFocus,
    pub loading: bool,
    pub tick: usize,
    pub poll_tick: usize,
    // Send modal
    pub show_send_modal: bool,
    pub send_from: String,
    pub send_to: String,
    pub send_msg: String,
    pub send_field: usize,
    // Channels management
    pub channels: Vec<ChannelInfo>,
    pub channel_list_state: ListState,
    // New channel form
    pub new_adapter: String,
    pub new_default_agent: String,
    pub new_token: String,
    pub new_field: usize, // 0=adapter, 1=token, 2=agent
    // Task modal
    pub show_task_modal: bool,
    pub task_title: String,
    pub task_desc: String,
    pub task_assign: String,
    pub task_field: usize,
    // Status
    pub status_msg: String,
}

pub enum CommsAction {
    Continue,
    Refresh,
    SendMessage {
        from: String,
        to: String,
        msg: String,
    },
    PostTask {
        title: String,
        desc: String,
        assign: String,
    },
}

impl CommsState {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            events: Vec::new(),
            event_list_state: ListState::default(),
            channels: Vec::new(),
            channel_list_state: ListState::default(),
            new_adapter: String::new(),
            new_default_agent: String::new(),
            new_token: String::new(),
            new_field: 0,
            status_msg: String::new(),
            focus: CommsFocus::Topology,
            loading: false,
            tick: 0,
            poll_tick: 0,
            show_send_modal: false,
            send_from: String::new(),
            send_to: String::new(),
            send_msg: String::new(),
            send_field: 0,
            show_task_modal: false,
            task_title: String::new(),
            task_desc: String::new(),
            task_assign: String::new(),
            task_field: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.poll_tick = self.poll_tick.wrapping_add(1);
    }

    /// Auto-refresh every ~5s at 20fps tick rate.
    pub fn should_poll(&self) -> bool {
        self.poll_tick > 0 && self.poll_tick.is_multiple_of(100)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> CommsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return CommsAction::Continue;
        }

        // Modal key handling
        if self.show_send_modal {
            return self.handle_send_modal_key(key);
        }
        if self.show_task_modal {
            return self.handle_task_modal_key(key);
        }

        // Channel creation form
        if self.focus == CommsFocus::ChannelAdd {
            return self.handle_channel_add_key(key);
        }

        match key.code {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    CommsFocus::Topology => CommsFocus::EventList,
                    CommsFocus::EventList => CommsFocus::Channels,
                    CommsFocus::Channels => CommsFocus::Topology,
                    _ => CommsFocus::Topology,
                };
            }
            KeyCode::Char('s') => {
                self.show_send_modal = true;
                self.send_from.clear();
                self.send_to.clear();
                self.send_msg.clear();
                self.send_field = 0;
            }
            KeyCode::Char('t') => {
                self.show_task_modal = true;
                self.task_title.clear();
                self.task_desc.clear();
                self.task_assign.clear();
                self.task_field = 0;
            }
            KeyCode::Char('r') => return CommsAction::Refresh,
            // Channel list navigation
            KeyCode::Up | KeyCode::Char('k')
                if self.focus == CommsFocus::Channels && !self.channels.is_empty() =>
            {
                let i = self.channel_list_state.selected().unwrap_or(0);
                let next = if i == 0 {
                    self.channels.len() - 1
                } else {
                    i - 1
                };
                self.channel_list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.focus == CommsFocus::Channels && !self.channels.is_empty() =>
            {
                let i = self.channel_list_state.selected().unwrap_or(0);
                let next = (i + 1) % self.channels.len();
                self.channel_list_state.select(Some(next));
            }
            KeyCode::Char('a') if self.focus == CommsFocus::Channels => {
                self.focus = CommsFocus::ChannelAdd;
                self.new_adapter = "telegram".to_string();
                self.new_default_agent.clear();
                self.new_token.clear();
                self.new_field = 0;
            }
            KeyCode::Up | KeyCode::Char('k')
                if self.focus == CommsFocus::EventList && !self.events.is_empty() =>
            {
                let i = self.event_list_state.selected().unwrap_or(0);
                let next = if i == 0 { self.events.len() - 1 } else { i - 1 };
                self.event_list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.focus == CommsFocus::EventList && !self.events.is_empty() =>
            {
                let i = self.event_list_state.selected().unwrap_or(0);
                let next = (i + 1) % self.events.len();
                self.event_list_state.select(Some(next));
            }
            _ => {}
        }
        CommsAction::Continue
    }

    fn handle_channel_add_key(&mut self, key: KeyEvent) -> CommsAction {
        match key.code {
            KeyCode::Esc => {
                self.focus = CommsFocus::Channels;
            }
            KeyCode::Tab => {
                self.new_field = (self.new_field + 1) % 3;
            }
            KeyCode::Char(c) => match self.new_field {
                0 => self.new_adapter.push(c),
                1 => self.new_token.push(c),
                2 => self.new_default_agent.push(c),
                _ => {}
            },
            KeyCode::Backspace => match self.new_field {
                0 => {
                    self.new_adapter.pop();
                }
                1 => {
                    self.new_token.pop();
                }
                2 => {
                    self.new_default_agent.pop();
                }
                _ => {}
            },
            KeyCode::Enter => {
                let info = ChannelInfo {
                    adapter: self.new_adapter.clone(),
                    default_agent: self.new_default_agent.clone(),
                    token_set: !self.new_token.is_empty(),
                    enabled: true,
                    index: self.channels.len(),
                };
                self.channels.push(info);
                self.channel_list_state
                    .select(Some(self.channels.len() - 1));
                self.status_msg = format!("Channel added. Reload config to apply.");
                self.focus = CommsFocus::Channels;
                return CommsAction::Refresh;
            }
            _ => {}
        }
        CommsAction::Continue
    }

    fn handle_send_modal_key(&mut self, key: KeyEvent) -> CommsAction {
        match key.code {
            KeyCode::Esc => {
                self.show_send_modal = false;
            }
            KeyCode::Tab => {
                self.send_field = (self.send_field + 1) % 3;
            }
            KeyCode::BackTab => {
                self.send_field = if self.send_field == 0 {
                    2
                } else {
                    self.send_field - 1
                };
            }
            KeyCode::Enter
                if !self.send_from.is_empty()
                    && !self.send_to.is_empty()
                    && !self.send_msg.is_empty() =>
            {
                self.show_send_modal = false;
                return CommsAction::SendMessage {
                    from: self.send_from.clone(),
                    to: self.send_to.clone(),
                    msg: self.send_msg.clone(),
                };
            }
            KeyCode::Char(c) => match self.send_field {
                0 => self.send_from.push(c),
                1 => self.send_to.push(c),
                _ => self.send_msg.push(c),
            },
            KeyCode::Backspace => match self.send_field {
                0 => {
                    self.send_from.pop();
                }
                1 => {
                    self.send_to.pop();
                }
                _ => {
                    self.send_msg.pop();
                }
            },
            _ => {}
        }
        CommsAction::Continue
    }

    fn handle_task_modal_key(&mut self, key: KeyEvent) -> CommsAction {
        match key.code {
            KeyCode::Esc => {
                self.show_task_modal = false;
            }
            KeyCode::Tab => {
                self.task_field = (self.task_field + 1) % 3;
            }
            KeyCode::BackTab => {
                self.task_field = if self.task_field == 0 {
                    2
                } else {
                    self.task_field - 1
                };
            }
            KeyCode::Enter if !self.task_title.is_empty() => {
                self.show_task_modal = false;
                return CommsAction::PostTask {
                    title: self.task_title.clone(),
                    desc: self.task_desc.clone(),
                    assign: self.task_assign.clone(),
                };
            }
            KeyCode::Char(c) => match self.task_field {
                0 => self.task_title.push(c),
                1 => self.task_desc.push(c),
                _ => self.task_assign.push(c),
            },
            KeyCode::Backspace => match self.task_field {
                0 => {
                    self.task_title.pop();
                }
                1 => {
                    self.task_desc.pop();
                }
                _ => {
                    self.task_assign.pop();
                }
            },
            _ => {}
        }
        CommsAction::Continue
    }

    // ── Topology helpers ─────────────────────────────────────────────────────

    fn root_nodes(&self) -> Vec<&CommsNode> {
        let child_ids: std::collections::HashSet<&str> = self
            .edges
            .iter()
            .filter(|e| e.kind == "parent_child")
            .map(|e| e.to.as_str())
            .collect();
        self.nodes
            .iter()
            .filter(|n| !child_ids.contains(n.id.as_str()))
            .collect()
    }

    fn children_of(&self, id: &str) -> Vec<&CommsNode> {
        let child_ids: Vec<&str> = self
            .edges
            .iter()
            .filter(|e| e.kind == "parent_child" && e.from == id)
            .map(|e| e.to.as_str())
            .collect();
        self.nodes
            .iter()
            .filter(|n| child_ids.contains(&n.id.as_str()))
            .collect()
    }

    fn peers_of(&self, id: &str) -> Vec<&CommsNode> {
        let peer_ids: std::collections::HashSet<&str> = self
            .edges
            .iter()
            .filter(|e| e.kind == "peer")
            .filter_map(|e| {
                if e.from == id {
                    Some(e.to.as_str())
                } else if e.to == id {
                    Some(e.from.as_str())
                } else {
                    None
                }
            })
            .collect();
        self.nodes
            .iter()
            .filter(|n| peer_ids.contains(n.id.as_str()))
            .collect()
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut CommsState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("{} {}", "○", crate::i18n::t("tui-comms-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(1),      // focus tabs
        Constraint::Length(1),      // separator
        Constraint::Percentage(35), // topology
        Constraint::Length(1),      // separator
        Constraint::Min(4),         // event list
        Constraint::Length(1),      // hints
    ])
    .split(inner);

    // Focus tab indicator
    if state.focus == CommsFocus::Channels {
        draw_channel_list(f, inner, state);
        if state.focus == CommsFocus::ChannelAdd {
            draw_channel_add(f, inner, state);
        }
        f.render_widget(
            widgets::hint_bar("a add channel | j/k navigate | Tab switch focus"),
            chunks[5],
        );
        return;
    }
    let topo_style = if state.focus == CommsFocus::Topology {
        theme::tab_active()
    } else {
        theme::tab_inactive()
    };
    let event_style = if state.focus == CommsFocus::EventList {
        theme::tab_active()
    } else {
        theme::tab_inactive()
    };

    let _chan_style = if state.focus == CommsFocus::Channels {
        theme::tab_active()
    } else {
        theme::tab_inactive()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(
                    " {} ",
                    crate::i18n::t_args(
                        "tui-comms-tab-topology",
                        &[
                            ("agents", &state.nodes.len().to_string()),
                            ("edges", &state.edges.len().to_string()),
                        ]
                    )
                ),
                topo_style,
            ),
            Span::raw("  "),
            Span::styled(
                format!(
                    " {} ",
                    crate::i18n::t_args(
                        "tui-comms-tab-events",
                        &[("count", &state.events.len().to_string())]
                    )
                ),
                event_style,
            ),
        ])),
        chunks[0],
    );

    // Separator
    f.render_widget(widgets::separator(inner.width), chunks[1]);

    // Topology tree
    draw_topology(f, chunks[2], state);

    // Separator
    f.render_widget(widgets::separator(inner.width), chunks[3]);

    // Event list
    draw_event_list(f, chunks[4], state);

    // Status message or hints
    let hint_text = if !state.status_msg.is_empty() {
        format!(
            "  {} | {}",
            state.status_msg,
            crate::i18n::t("tui-comms-hints")
        )
    } else {
        crate::i18n::t("tui-comms-hints")
    };
    f.render_widget(widgets::hint_bar(&hint_text), chunks[5]);

    // Modal overlays
    if state.show_send_modal {
        draw_send_modal(f, area, state);
    }
    if state.show_task_modal {
        draw_task_modal(f, area, state);
    }
}

fn draw_topology(f: &mut Frame, area: Rect, state: &CommsState) {
    if state.loading && state.nodes.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-comms-loading")),
            area,
        );
        return;
    }

    if state.nodes.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-comms-empty")),
            area,
        );
        return;
    }

    let focus_highlight = state.focus == CommsFocus::Topology;
    let mut lines = Vec::new();

    for root in state.root_nodes() {
        let (indicator, indicator_style) = state_indicator(&root.state);
        let mut spans = vec![
            Span::styled("  ", Style::default()),
            Span::styled(format!("{indicator} "), indicator_style),
            Span::styled(
                format!("{} ", root.name),
                Style::default()
                    .fg(if focus_highlight {
                        theme::ACCENT
                    } else {
                        theme::TEXT_PRIMARY
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("({}) ", root.model), theme::dim_style()),
            Span::styled(root.state.clone(), state_color(&root.state)),
        ];
        // Peer annotations
        for peer in state.peers_of(&root.id) {
            spans.push(Span::styled(
                format!("  ↔ {}", peer.name),
                Style::default().fg(theme::PURPLE),
            ));
        }
        lines.push(Line::from(spans));

        // Children
        let children = state.children_of(&root.id);
        for (i, child) in children.iter().enumerate() {
            let branch = if i < children.len() - 1 {
                "├── "
            } else {
                "└── "
            };
            let (child_ind, child_ind_style) = state_indicator(&child.state);
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(branch, Style::default().fg(theme::BORDER)),
                Span::styled(format!("{child_ind} "), child_ind_style),
                Span::styled(
                    format!("{} ", child.name),
                    Style::default()
                        .fg(theme::TEXT_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("({}) ", child.model), theme::dim_style()),
                Span::styled(child.state.clone(), state_color(&child.state)),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_event_list(f: &mut Frame, area: Rect, state: &mut CommsState) {
    if state.events.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-comms-events-empty")),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = state
        .events
        .iter()
        .map(|ev| {
            let kind_style = kind_color(&ev.kind);
            let kind_label = kind_short(&ev.kind);
            let kind_indicator = kind_indicator(&ev.kind);
            let target_part = if ev.target_name.is_empty() {
                String::new()
            } else {
                format!(" → {}", ev.target_name)
            };
            let detail = widgets::truncate(&ev.detail, 50);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {:<8}", short_time(&ev.timestamp)),
                    Style::default().fg(theme::TEXT_TERTIARY),
                ),
                Span::styled(format!(" {kind_indicator}"), kind_style),
                Span::styled(format!(" {:<10}", kind_label), kind_style),
                Span::styled(
                    ev.source_name.to_string(),
                    Style::default()
                        .fg(theme::TEXT_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(target_part, Style::default().fg(theme::PURPLE)),
                Span::styled(format!("  {detail}"), theme::dim_style()),
            ]))
        })
        .collect();

    let list = widgets::themed_list(items);
    f.render_stateful_widget(list, area, &mut state.event_list_state);
}

fn draw_send_modal(f: &mut Frame, area: Rect, state: &CommsState) {
    let modal = centered_rect(50, 12, area);
    f.render_widget(Clear, modal);

    let block = Block::default()
        .title(Span::styled(
            crate::i18n::t("tui-comms-modal-send-title"),
            theme::title_style(),
        ))
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));
    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let field_style = |idx: usize| {
        if state.send_field == idx {
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD)
        } else {
            theme::dim_style()
        }
    };

    f.render_widget(
        Paragraph::new(Span::styled(
            crate::i18n::t("tui-comms-modal-send-from"),
            field_style(0),
        )),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {}█", state.send_from),
            Style::default().fg(theme::TEXT),
        )),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            crate::i18n::t("tui-comms-modal-send-to"),
            field_style(1),
        )),
        rows[2],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {}█", state.send_to),
            Style::default().fg(theme::TEXT),
        )),
        rows[3],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            crate::i18n::t("tui-comms-modal-send-msg"),
            field_style(2),
        )),
        rows[4],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {}█", state.send_msg),
            Style::default().fg(theme::TEXT),
        )),
        rows[5],
    );
    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-comms-modal-send-hints")),
        rows[6],
    );
}

fn draw_task_modal(f: &mut Frame, area: Rect, state: &CommsState) {
    let modal = centered_rect(50, 12, area);
    f.render_widget(Clear, modal);

    let block = Block::default()
        .title(Span::styled(
            crate::i18n::t("tui-comms-modal-task-title"),
            theme::title_style(),
        ))
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));
    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let field_style = |idx: usize| {
        if state.task_field == idx {
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD)
        } else {
            theme::dim_style()
        }
    };

    f.render_widget(
        Paragraph::new(Span::styled(
            crate::i18n::t("tui-comms-modal-task-title-field"),
            field_style(0),
        )),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {}█", state.task_title),
            Style::default().fg(theme::TEXT),
        )),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            crate::i18n::t("tui-comms-modal-task-desc"),
            field_style(1),
        )),
        rows[2],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {}█", state.task_desc),
            Style::default().fg(theme::TEXT),
        )),
        rows[3],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            crate::i18n::t("tui-comms-modal-task-assign"),
            field_style(2),
        )),
        rows[4],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  {}█", state.task_assign),
            Style::default().fg(theme::TEXT),
        )),
        rows[5],
    );
    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-comms-modal-task-hints")),
        rows[6],
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn state_color(state: &str) -> Style {
    match state {
        "Running" => Style::default().fg(theme::GREEN),
        "Suspended" => Style::default().fg(theme::YELLOW),
        "Terminated" | "Crashed" => Style::default().fg(theme::RED),
        _ => theme::dim_style(),
    }
}

fn state_indicator(state: &str) -> (&'static str, Style) {
    match state {
        "Running" => (
            "●",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        "Suspended" => ("●", Style::default().fg(theme::YELLOW)),
        "Terminated" | "Crashed" => ("○", Style::default().fg(theme::RED)),
        _ => ("○", theme::dim_style()),
    }
}

fn kind_color(kind: &str) -> Style {
    match kind {
        "agent_message" => Style::default().fg(theme::CYAN),
        "agent_spawned" => Style::default().fg(theme::GREEN),
        "agent_terminated" => Style::default().fg(theme::RED),
        "task_posted" => Style::default().fg(theme::YELLOW),
        "task_claimed" => Style::default().fg(theme::CYAN),
        "task_completed" => Style::default().fg(theme::GREEN),
        _ => theme::dim_style(),
    }
}

fn kind_short(kind: &str) -> &str {
    match kind {
        "agent_message" => "MSG",
        "agent_spawned" => "SPAWNED",
        "agent_terminated" => "KILLED",
        "task_posted" => "TASK+",
        "task_claimed" => "CLAIM",
        "task_completed" => "DONE",
        _ => kind,
    }
}

fn kind_indicator(kind: &str) -> &'static str {
    match kind {
        "agent_spawned" | "task_completed" => "●", // filled green-ish
        "agent_message" | "task_claimed" => "●",   // filled
        "agent_terminated" => "○",                 // hollow
        "task_posted" => "●",                      // filled yellow-ish
        _ => "○",
    }
}

fn short_time(ts: &str) -> String {
    // Extract HH:MM:SS from ISO-8601
    if let Some(t_pos) = ts.find('T') {
        let time_part = &ts[t_pos + 1..];
        if time_part.len() >= 8 {
            return time_part[..8].to_string();
        }
    }
    ts.chars().take(8).collect()
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, w, height.min(area.height))
}

fn draw_channel_list(f: &mut Frame, area: Rect, state: &mut CommsState) {
    let items: Vec<ListItem> = if state.channels.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  No channels configured. Press 'a' to add one.",
            Style::default().fg(theme::TEXT_TERTIARY),
        )))]
    } else {
        state
            .channels
            .iter()
            .map(|ch| {
                let adapter = &ch.adapter;
                let agent = if ch.default_agent.is_empty() {
                    "(any)"
                } else {
                    &ch.default_agent
                };
                let token = if ch.token_set {
                    "token ✓"
                } else {
                    "token ✗"
                };
                let enabled = if ch.enabled { "enabled" } else { "disabled" };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {adapter:12} "),
                        Style::default()
                            .fg(theme::CYAN)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("agent: {agent:20} "),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                    Span::styled(format!("{token} "), Style::default().fg(theme::GREEN)),
                    Span::styled(
                        format!("{enabled}"),
                        Style::default().fg(theme::TEXT_TERTIARY),
                    ),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(Style::default().fg(theme::ACCENT));
    let mut list_state = state.channel_list_state.clone();
    f.render_stateful_widget(list, area, &mut list_state);
    state.channel_list_state = list_state;
}

fn draw_channel_add(f: &mut Frame, area: Rect, state: &CommsState) {
    let popup_area = Rect::new(area.x + 2, area.y + 2, area.width.saturating_sub(4), 8);
    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .title(" New Channel ");
    f.render_widget(block, popup_area);
    let inner = Rect::new(
        popup_area.x + 1,
        popup_area.y + 1,
        popup_area.width - 2,
        popup_area.height - 2,
    );
    let lines = vec![
        Line::from(vec![
            Span::styled(
                if state.new_field == 0 { "▶ " } else { "  " },
                Style::default().fg(theme::ACCENT),
            ),
            Span::styled("Adapter: ", Style::default().fg(theme::TEXT_TERTIARY)),
            Span::styled(&state.new_adapter, Style::default().fg(theme::TEXT_PRIMARY)),
            Span::styled(
                " (telegram/slack/discord/whatsapp)",
                Style::default().fg(theme::TEXT_TERTIARY),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                if state.new_field == 1 { "▶ " } else { "  " },
                Style::default().fg(theme::ACCENT),
            ),
            Span::styled("Token: ", Style::default().fg(theme::TEXT_TERTIARY)),
            Span::styled(
                if state.new_token.is_empty() {
                    "(paste bot token)"
                } else {
                    "••••••••"
                },
                Style::default().fg(theme::TEXT_PRIMARY),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                if state.new_field == 2 { "▶ " } else { "  " },
                Style::default().fg(theme::ACCENT),
            ),
            Span::styled("Default Agent: ", Style::default().fg(theme::TEXT_TERTIARY)),
            Span::styled(
                if state.new_default_agent.is_empty() {
                    "(agent name or UUID)"
                } else {
                    &state.new_default_agent
                },
                Style::default().fg(theme::TEXT_PRIMARY),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Enter to add | Esc to cancel",
            Style::default().fg(theme::TEXT_TERTIARY),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
    if !state.status_msg.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &state.status_msg,
                Style::default().fg(theme::GREEN),
            ))),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        );
    }
}
