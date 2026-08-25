//! Settings screen: provider key management, model catalog, tools list, backup archives.

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
pub struct ProviderInfo {
    pub name: String,
    pub configured: bool,
    pub env_var: String,
    /// Whether this is a local provider (ollama, vllm, lmstudio).
    pub is_local: bool,
    /// Whether the local provider is reachable (only set for local providers).
    pub reachable: Option<bool>,
    /// Probe latency in milliseconds (only set for local providers).
    pub latency_ms: Option<u64>,
}

#[derive(Clone, Default)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub tier: String,
    pub context_window: u64,
    pub cost_input: f64,
    pub cost_output: f64,
}

#[derive(Clone, Default)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Clone)]
pub struct TestResult {
    pub provider: String,
    pub success: bool,
    pub latency_ms: u64,
    pub message: String,
}

/// One archive as `GET /api/backups` reports it.
#[derive(Clone, Default)]
pub struct BackupInfo {
    pub filename: String,
    pub size_bytes: u64,
    /// Manifest `created_at`, falling back to the file's `modified_at`.
    pub created_at: String,
    /// Component names the archive's `manifest.json` declares.
    ///
    /// These are the only names the restore form offers, so the `components`
    /// list the TUI sends is always a subset of what the daemon itself wrote
    /// into the manifest — a name `POST /api/restore` cannot reject.
    pub components: Vec<String>,
}

/// The restore form opened over a selected archive.
///
/// Row `0` is the keep-config toggle and rows `1..=components.len()` are the
/// component toggles, so `cursor` indexes one flat list the arrow keys walk.
pub struct RestoreForm {
    pub filename: String,
    pub cursor: usize,
    /// Skip `config.toml` so this machine keeps its own key, port and paths.
    pub keep_config: bool,
    /// Every component in the archive, paired with whether it is selected.
    pub components: Vec<(String, bool)>,
}

impl RestoreForm {
    fn new(backup: &BackupInfo) -> Self {
        Self {
            filename: backup.filename.clone(),
            cursor: 0,
            keep_config: false,
            components: backup
                .components
                .iter()
                .map(|c| (c.clone(), true))
                .collect(),
        }
    }

    /// Number of navigable rows: the keep-config toggle plus one per component.
    fn row_count(&self) -> usize {
        self.components.len() + 1
    }

    fn toggle_current(&mut self) {
        if self.cursor == 0 {
            self.keep_config = !self.keep_config;
        } else if let Some(entry) = self.components.get_mut(self.cursor - 1) {
            entry.1 = !entry.1;
        }
    }
}

/// Build the `POST /api/restore` body for a filled-in restore form, or `None`
/// when the operator has deselected everything.
///
/// Three shapes, and the difference between them is destructive:
///
/// - every component selected (or an archive whose manifest listed none) omits
///   `components` entirely, which is the endpoint's own way of saying
///   "restore everything", including archive entries no component owns;
/// - a strict subset sends exactly those names;
/// - nothing selected returns `None`, because `components: []` is a `400` by
///   design — the API refuses to guess whether an empty list means "nothing"
///   or "everything", and the TUI must not make it guess.
pub fn restore_request_body(form: &RestoreForm) -> Option<serde_json::Value> {
    let selected: Vec<&str> = form
        .components
        .iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| name.as_str())
        .collect();
    if !form.components.is_empty() && selected.is_empty() {
        return None;
    }
    let mut body = serde_json::json!({
        "filename": form.filename,
        "keep_config": form.keep_config,
    });
    if selected.len() != form.components.len() {
        body["components"] = serde_json::json!(selected);
    }
    Some(body)
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsSub {
    Providers,
    Models,
    Tools,
    Backups,
}

pub struct SettingsState {
    pub sub: SettingsSub,
    pub providers: Vec<ProviderInfo>,
    pub models: Vec<ModelInfo>,
    pub tools: Vec<ToolInfo>,
    pub backups: Vec<BackupInfo>,
    pub provider_list: ListState,
    pub model_list: ListState,
    pub tool_list: ListState,
    pub backup_list: ListState,
    /// Open restore form, if any. While it is open the sub-tab number keys are
    /// inert, so `Esc` is the documented way back out.
    pub restore: Option<RestoreForm>,
    pub confirm_delete: bool,
    pub input_buf: String,
    pub input_mode: bool,
    pub editing_provider: Option<String>,
    pub test_result: Option<TestResult>,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum SettingsAction {
    Continue,
    RefreshProviders,
    RefreshModels,
    RefreshTools,
    RefreshBackups,
    SaveProviderKey { name: String, key: String },
    DeleteProviderKey(String),
    TestProvider(String),
    CreateBackup,
    DeleteBackup(String),
    RestoreBackup(serde_json::Value),
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            sub: SettingsSub::Providers,
            providers: Vec::new(),
            models: Vec::new(),
            tools: Vec::new(),
            backups: Vec::new(),
            provider_list: ListState::default(),
            model_list: ListState::default(),
            tool_list: ListState::default(),
            backup_list: ListState::default(),
            restore: None,
            confirm_delete: false,
            input_buf: String::new(),
            input_mode: false,
            editing_provider: None,
            test_result: None,
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Return the screen to its landing sub-tab and drop any modal state.
    ///
    /// Called every time the Settings tab is entered. `sub` is otherwise a
    /// plain field that survives leaving the tab, so a sub-tab holding a modal
    /// that swallows the `1`-`4` switch keys would stay in front of the
    /// operator for the rest of the session with no way past it. Re-entering
    /// the tab is the second escape, behind `Esc`.
    pub fn reset_sub(&mut self) {
        self.switch_sub(SettingsSub::Providers);
        self.input_mode = false;
        self.editing_provider = None;
        self.input_buf.clear();
    }

    /// Move to another sub-tab, dropping the state that belonged to the old
    /// one. `status_msg` is shared by the Providers and Backups panes, so
    /// carrying it across would show "Backup created" under the provider list.
    fn switch_sub(&mut self, sub: SettingsSub) {
        self.sub = sub;
        self.restore = None;
        self.confirm_delete = false;
        self.status_msg.clear();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SettingsAction::Continue;
        }

        if self.input_mode {
            return self.handle_input(key);
        }

        // Sub-tab switching. Suppressed only while a modal is up — the restore
        // form and the delete confirmation both bind `Esc` (and the
        // confirmation, any other key) to close, so the switch keys are never
        // unreachable for more than one keystroke.
        if self.restore.is_none() && !self.confirm_delete {
            match key.code {
                KeyCode::Char('1') => {
                    self.switch_sub(SettingsSub::Providers);
                    return SettingsAction::RefreshProviders;
                }
                KeyCode::Char('2') => {
                    self.switch_sub(SettingsSub::Models);
                    return SettingsAction::RefreshModels;
                }
                KeyCode::Char('3') => {
                    self.switch_sub(SettingsSub::Tools);
                    return SettingsAction::RefreshTools;
                }
                KeyCode::Char('4') => {
                    self.switch_sub(SettingsSub::Backups);
                    return SettingsAction::RefreshBackups;
                }
                _ => {}
            }
        }

        match self.sub {
            SettingsSub::Providers => self.handle_providers(key),
            SettingsSub::Models => self.handle_models(key),
            SettingsSub::Tools => self.handle_tools(key),
            SettingsSub::Backups => self.handle_backups(key),
        }
    }

    fn handle_input(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = false;
                self.editing_provider = None;
                self.input_buf.clear();
            }
            KeyCode::Enter => {
                self.input_mode = false;
                if let Some(name) = self.editing_provider.take() {
                    if !self.input_buf.is_empty() {
                        let api_key = self.input_buf.clone();
                        self.input_buf.clear();
                        return SettingsAction::SaveProviderKey { name, key: api_key };
                    }
                }
                self.input_buf.clear();
            }
            KeyCode::Backspace => {
                self.input_buf.pop();
            }
            KeyCode::Char(c) => {
                self.input_buf.push(c);
            }
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_providers(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.providers.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.provider_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.provider_list.select(Some(next));
                self.test_result = None;
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.provider_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.provider_list.select(Some(next));
                self.test_result = None;
            }
            KeyCode::Char('e') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        self.editing_provider = Some(self.providers[sel].name.clone());
                        self.input_mode = true;
                        self.input_buf.clear();
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        return SettingsAction::DeleteProviderKey(self.providers[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('t') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        self.test_result = None;
                        return SettingsAction::TestProvider(self.providers[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('r') => return SettingsAction::RefreshProviders,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_models(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.models.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.model_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.model_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.model_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.model_list.select(Some(next));
            }
            KeyCode::Char('r') => return SettingsAction::RefreshModels,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_tools(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.tools.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.tool_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.tool_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.tool_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.tool_list.select(Some(next));
            }
            KeyCode::Char('r') => return SettingsAction::RefreshTools,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_backups(&mut self, key: KeyEvent) -> SettingsAction {
        if self.restore.is_some() {
            return self.handle_restore_form(key);
        }

        if self.confirm_delete {
            self.confirm_delete = false;
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                if let Some(backup) = self.selected_backup() {
                    return SettingsAction::DeleteBackup(backup.filename.clone());
                }
            }
            return SettingsAction::Continue;
        }

        let total = self.backups.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.backup_list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.backup_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.backup_list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.backup_list.select(Some(next));
            }
            KeyCode::Char('c') => {
                self.status_msg = crate::i18n::t("tui-settings-backups-creating");
                return SettingsAction::CreateBackup;
            }
            KeyCode::Char('d') if self.selected_backup().is_some() => {
                self.confirm_delete = true;
            }
            KeyCode::Enter => {
                if let Some(form) = self.selected_backup().map(RestoreForm::new) {
                    self.status_msg.clear();
                    self.restore = Some(form);
                }
            }
            KeyCode::Char('r') => return SettingsAction::RefreshBackups,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_restore_form(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.restore = None;
                SettingsAction::Continue
            }
            KeyCode::Enter => match self.restore.as_ref().and_then(restore_request_body) {
                Some(body) => {
                    self.restore = None;
                    self.status_msg.clear();
                    SettingsAction::RestoreBackup(body)
                }
                // Deselecting everything is the one request the API answers
                // with a 400 by design, so it is answered here instead of
                // round-tripped into an error toast.
                None => {
                    self.status_msg = crate::i18n::t("tui-settings-backups-restore-none");
                    SettingsAction::Continue
                }
            },
            other => {
                if let Some(form) = self.restore.as_mut() {
                    let rows = form.row_count();
                    match other {
                        KeyCode::Up | KeyCode::Char('k') => {
                            form.cursor = if form.cursor == 0 {
                                rows - 1
                            } else {
                                form.cursor - 1
                            };
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            form.cursor = (form.cursor + 1) % rows;
                        }
                        KeyCode::Char(' ') => form.toggle_current(),
                        _ => {}
                    }
                }
                SettingsAction::Continue
            }
        }
    }

    fn selected_backup(&self) -> Option<&BackupInfo> {
        self.backup_list
            .selected()
            .and_then(|sel| self.backups.get(sel))
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("⚙ {}", crate::i18n::t("tui-settings-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(1), // sub-tab bar
        Constraint::Length(1), // separator
        Constraint::Min(3),    // content
        Constraint::Length(1), // hints
    ])
    .split(inner);

    draw_sub_tabs(f, chunks[0], state.sub);

    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    match state.sub {
        SettingsSub::Providers => draw_providers(f, chunks[2], state),
        SettingsSub::Models => draw_models(f, chunks[2], state),
        SettingsSub::Tools => draw_tools(f, chunks[2], state),
        SettingsSub::Backups => draw_backups(f, chunks[2], state),
    }

    // Hints
    let hint_text = match state.sub {
        SettingsSub::Providers if state.input_mode => crate::i18n::t("tui-settings-hints-input"),
        SettingsSub::Providers => crate::i18n::t("tui-settings-hints-providers"),
        SettingsSub::Models => crate::i18n::t("tui-settings-hints-models"),
        SettingsSub::Tools => crate::i18n::t("tui-settings-hints-tools"),
        SettingsSub::Backups if state.restore.is_some() => {
            crate::i18n::t("tui-settings-hints-restore")
        }
        SettingsSub::Backups => crate::i18n::t("tui-settings-hints-backups"),
    };
    if state.sub == SettingsSub::Backups {
        f.render_widget(
            widgets::confirm_or_status_or_hint(
                state.confirm_delete,
                &crate::i18n::t("tui-settings-backups-delete-confirm"),
                &state.status_msg,
                &hint_text,
            ),
            chunks[3],
        );
    } else {
        f.render_widget(widgets::hint_bar(&hint_text), chunks[3]);
    }
}

fn draw_sub_tabs(f: &mut Frame, area: Rect, active: SettingsSub) {
    let tabs = [
        (
            SettingsSub::Providers,
            crate::i18n::t("tui-settings-tab-providers"),
        ),
        (
            SettingsSub::Models,
            crate::i18n::t("tui-settings-tab-models"),
        ),
        (SettingsSub::Tools, crate::i18n::t("tui-settings-tab-tools")),
        (
            SettingsSub::Backups,
            crate::i18n::t("tui-settings-tab-backups"),
        ),
    ];
    let mut spans = vec![Span::raw("  ")];
    for (i, (sub, label)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(theme::BORDER)));
        }
        if *sub == active {
            spans.push(Span::styled(format!(" ● {label} "), theme::tab_active()));
        } else {
            spans.push(Span::styled(format!(" ○ {label} "), theme::tab_inactive()));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_providers(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(2), // input / test result
    ])
    .split(area);

    let provider_hdr = crate::i18n::t("tui-settings-providers-header-provider");
    let status_hdr = crate::i18n::t("tui-settings-providers-header-status");
    let env_hdr = crate::i18n::t("tui-settings-providers-header-env");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<20} {:<20} {}", provider_hdr, status_hdr, env_hdr),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.providers.is_empty() {
        f.render_widget(
            widgets::spinner(
                state.tick,
                &crate::i18n::t("tui-settings-providers-loading"),
            ),
            chunks[1],
        );
    } else if state.providers.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-providers-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .providers
            .iter()
            .map(|p| {
                let (badge, badge_style) = if p.is_local {
                    match p.reachable {
                        Some(true) => {
                            let ms = p.latency_ms.unwrap_or(0);
                            (
                                format!(
                                    "● {}",
                                    crate::i18n::t_args(
                                        "tui-settings-providers-status-online",
                                        &[("ms", &ms.to_string())]
                                    )
                                ),
                                Style::default().fg(theme::GREEN),
                            )
                        }
                        Some(false) => (
                            format!(
                                "● {}",
                                crate::i18n::t("tui-settings-providers-status-offline")
                            ),
                            Style::default().fg(theme::RED),
                        ),
                        None => (
                            format!(
                                "○ {}",
                                crate::i18n::t("tui-settings-providers-status-local")
                            ),
                            theme::dim_style(),
                        ),
                    }
                } else if p.configured {
                    (
                        format!(
                            "● {}",
                            crate::i18n::t("tui-settings-providers-status-configured")
                        ),
                        Style::default().fg(theme::GREEN),
                    )
                } else {
                    (
                        format!(
                            "○ {}",
                            crate::i18n::t("tui-settings-providers-status-notset")
                        ),
                        theme::dim_style(),
                    )
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<20}", p.name),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(format!(" {:<20}", badge), badge_style),
                    Span::styled(format!(" {}", p.env_var), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.provider_list);
    }

    // Input mode or test result
    if state.input_mode {
        let provider_name = state.editing_provider.as_deref().unwrap_or("?");
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    format!(
                        "  🔑 {}",
                        crate::i18n::t_args(
                            "tui-settings-providers-input-prompt",
                            &[("provider", provider_name)]
                        )
                    ),
                    Style::default().fg(theme::YELLOW),
                )]),
                Line::from(vec![
                    Span::raw("  ▸ "),
                    Span::styled(
                        "•".repeat(state.input_buf.len().min(40)),
                        theme::input_style(),
                    ),
                    Span::styled(
                        "█",
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::SLOW_BLINK),
                    ),
                ]),
            ]),
            chunks[2],
        );
    } else if let Some(result) = &state.test_result {
        let (icon, style) = if result.success {
            ("●", Style::default().fg(theme::GREEN))
        } else {
            ("●", Style::default().fg(theme::RED))
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(format!("  {icon} "), style),
                    Span::styled(format!("{}: {}", result.provider, result.message), style),
                ]),
                Line::from(vec![Span::styled(
                    if result.success {
                        format!(
                            "  {}",
                            crate::i18n::t_args(
                                "tui-settings-providers-latency",
                                &[("ms", &result.latency_ms.to_string())]
                            )
                        )
                    } else {
                        String::new()
                    },
                    theme::dim_style(),
                )]),
            ]),
            chunks[2],
        );
    } else if !state.status_msg.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::GREEN),
            )])),
            chunks[2],
        );
    }
}

fn draw_models(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
    ])
    .split(area);

    let id_hdr = crate::i18n::t("tui-settings-models-header-id");
    let provider_hdr = crate::i18n::t("tui-settings-models-header-provider");
    let tier_hdr = crate::i18n::t("tui-settings-models-header-tier");
    let ctx_hdr = crate::i18n::t("tui-settings-models-header-context");
    let cost_hdr = crate::i18n::t("tui-settings-models-header-cost");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<28} {:<14} {:<10} {:<10} {}",
                id_hdr, provider_hdr, tier_hdr, ctx_hdr, cost_hdr
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.models.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-settings-models-loading")),
            chunks[1],
        );
    } else if state.models.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-models-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .models
            .iter()
            .map(|m| {
                let tier_style = match m.tier.as_str() {
                    "Frontier" => Style::default()
                        .fg(theme::PURPLE)
                        .add_modifier(Modifier::BOLD),
                    "Smart" => Style::default()
                        .fg(theme::BLUE)
                        .add_modifier(Modifier::BOLD),
                    "Balanced" => Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                    "Fast" => Style::default()
                        .fg(theme::YELLOW)
                        .add_modifier(Modifier::BOLD),
                    _ => theme::dim_style(),
                };
                let ctx = format_context(m.context_window);
                let cost = format!("${:.2}/${:.2}", m.cost_input, m.cost_output);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<28}", widgets::truncate(&m.id, 27)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {:<14}", widgets::truncate(&m.provider, 13)),
                        theme::dim_style(),
                    ),
                    Span::styled(format!(" {:<10}", m.tier), tier_style),
                    Span::styled(format!(" {:<10}", ctx), Style::default().fg(theme::YELLOW)),
                    Span::styled(format!(" {cost}"), theme::dim_style()),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.model_list);
    }
}

fn draw_tools(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
    ])
    .split(area);

    let name_hdr = crate::i18n::t("tui-settings-tools-header-name");
    let desc_hdr = crate::i18n::t("tui-settings-tools-header-desc");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<24} {}", name_hdr, desc_hdr),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.tools.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-tools-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .tools
            .iter()
            .map(|t| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<24}", widgets::truncate(&t.name, 23)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&t.description, 50)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.tool_list);
    }
}

fn draw_backups(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    if let Some(form) = &state.restore {
        draw_restore_form(f, area, form);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
    ])
    .split(area);

    let name_hdr = crate::i18n::t("tui-settings-backups-header-filename");
    let size_hdr = crate::i18n::t("tui-settings-backups-header-size");
    let created_hdr = crate::i18n::t("tui-settings-backups-header-created");
    let components_hdr = crate::i18n::t("tui-settings-backups-header-components");
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<38} {:>10} {:<20} {}",
                name_hdr, size_hdr, created_hdr, components_hdr
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.backups.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-settings-backups-loading")),
            chunks[1],
        );
        return;
    }
    if state.backups.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-backups-empty")),
            chunks[1],
        );
        return;
    }

    let items: Vec<ListItem> = state
        .backups
        .iter()
        .map(|b| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {:<38}", widgets::truncate(&b.filename, 37)),
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(
                    format!(" {:>10}", format_size(b.size_bytes)),
                    Style::default().fg(theme::YELLOW),
                ),
                Span::styled(
                    format!(" {:<20}", widgets::truncate(&b.created_at, 19)),
                    theme::dim_style(),
                ),
                Span::styled(
                    format!(" {}", widgets::truncate(&b.components.join(", "), 40)),
                    theme::dim_style(),
                ),
            ]))
        })
        .collect();

    let list = widgets::themed_list(items);
    f.render_stateful_widget(list, chunks[1], &mut state.backup_list);
}

fn draw_restore_form(f: &mut Frame, area: Rect, form: &RestoreForm) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // warning
        Constraint::Min(3),    // toggles
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {}",
                crate::i18n::t_args(
                    "tui-settings-backups-restore-title",
                    &[("filename", &form.filename)]
                )
            ),
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        )])),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {}",
                crate::i18n::t("tui-settings-backups-restore-warning")
            ),
            Style::default().fg(theme::YELLOW),
        )])),
        chunks[1],
    );

    let mut rows: Vec<Line> = Vec::with_capacity(form.row_count() + 1);
    rows.push(render_toggle_row(
        form.cursor == 0,
        form.keep_config,
        &crate::i18n::t("tui-settings-backups-restore-keep-config"),
    ));
    for (i, (name, selected)) in form.components.iter().enumerate() {
        rows.push(render_toggle_row(form.cursor == i + 1, *selected, name));
    }
    if form.components.iter().all(|(_, on)| *on) {
        rows.push(Line::from(vec![Span::styled(
            format!("  {}", crate::i18n::t("tui-settings-backups-restore-all")),
            theme::dim_style(),
        )]));
    }
    f.render_widget(Paragraph::new(rows), chunks[2]);
}

fn render_toggle_row(focused: bool, checked: bool, label: &str) -> Line<'static> {
    let marker = if focused { "\u{25b8}" } else { " " };
    let box_glyph = if checked { "[x]" } else { "[ ]" };
    let style = if focused {
        Style::default()
            .fg(theme::CYAN)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::from(vec![Span::styled(
        ["  ", marker, " ", box_glyph, " ", label].concat(),
        style,
    )])
}

/// Human-readable archive size. No space before the unit, so the whole thing
/// stays one column-aligned token in the list.
fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KIB * KIB * KIB {
        format!("{:.1}GiB", b / (KIB * KIB * KIB))
    } else if b >= KIB * KIB {
        format!("{:.1}MiB", b / (KIB * KIB))
    } else if b >= KIB {
        format!("{:.1}KiB", b / KIB)
    } else {
        format!("{bytes}B")
    }
}

fn format_context(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn backup(components: &[&str]) -> BackupInfo {
        BackupInfo {
            filename: "librefang-backup-20260101-000000.zip".to_string(),
            size_bytes: 4096,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            components: components.iter().map(|c| c.to_string()).collect(),
        }
    }

    fn on_backups_tab(components: &[&str]) -> SettingsState {
        let mut state = SettingsState::new();
        state.sub = SettingsSub::Backups;
        state.backups = vec![backup(components)];
        state.backup_list.select(Some(0));
        state
    }

    /// Selecting everything must omit `components` rather than list them all:
    /// the endpoint restores archive entries no component owns only when the
    /// field is absent, so listing every name is a narrower restore than the
    /// operator asked for.
    #[test]
    fn a_full_selection_omits_the_components_field() {
        let form = RestoreForm::new(&backup(&["config", "skills", "data"]));
        let body = restore_request_body(&form).expect("a full selection is restorable");
        assert!(
            body.get("components").is_none(),
            "every component selected must mean 'restore everything', not an explicit list"
        );
        assert_eq!(body["keep_config"], serde_json::json!(false));
        assert_eq!(
            body["filename"],
            serde_json::json!("librefang-backup-20260101-000000.zip")
        );
    }

    #[test]
    fn a_partial_selection_sends_exactly_the_selected_names() {
        let mut form = RestoreForm::new(&backup(&["config", "skills", "data"]));
        // Row 0 is keep_config, so `skills` is row 2.
        form.cursor = 2;
        form.toggle_current();
        let body = restore_request_body(&form).expect("a partial selection is restorable");
        assert_eq!(
            body["components"],
            serde_json::json!(["config", "data"]),
            "the deselected component must be the only one missing"
        );
    }

    /// `components: []` is a 400 by design, so the form must never produce it.
    #[test]
    fn an_empty_selection_produces_no_request_at_all() {
        let mut form = RestoreForm::new(&backup(&["config"]));
        form.cursor = 1;
        form.toggle_current();
        assert!(
            restore_request_body(&form).is_none(),
            "an empty selection must be refused here, not sent as a request the API rejects"
        );
    }

    /// An archive whose `manifest.json` could not be read lists no components.
    /// That is "restore everything", not "restore nothing" — the empty-list
    /// guard must not swallow it.
    #[test]
    fn an_archive_without_a_manifest_restores_everything() {
        let form = RestoreForm::new(&backup(&[]));
        let body =
            restore_request_body(&form).expect("a manifest-less archive is still restorable");
        assert!(body.get("components").is_none());
    }

    #[test]
    fn keep_config_reaches_the_request_body() {
        let mut form = RestoreForm::new(&backup(&["config"]));
        form.cursor = 0;
        form.toggle_current();
        let body = restore_request_body(&form).expect("clone mode is restorable");
        assert_eq!(body["keep_config"], serde_json::json!(true));
    }

    #[test]
    fn the_fourth_number_key_opens_the_backups_tab_and_loads_it() {
        let mut state = SettingsState::new();
        let action = state.handle_key(key(KeyCode::Char('4')));
        assert!(state.sub == SettingsSub::Backups);
        assert!(matches!(action, SettingsAction::RefreshBackups));
    }

    /// The backups list itself binds no number keys, so `1`-`4` keep working
    /// from it — the collision the issue warned about never happens.
    #[test]
    fn the_backups_list_does_not_swallow_the_sub_tab_switch_keys() {
        let mut state = on_backups_tab(&["config"]);
        let action = state.handle_key(key(KeyCode::Char('2')));
        assert!(state.sub == SettingsSub::Models);
        assert!(matches!(action, SettingsAction::RefreshModels));
    }

    /// The restore form does hold the number keys, so `Esc` must give them
    /// back in one keystroke.
    #[test]
    fn esc_closes_the_restore_form_and_returns_the_switch_keys() {
        let mut state = on_backups_tab(&["config"]);
        state.handle_key(key(KeyCode::Enter));
        assert!(state.restore.is_some(), "Enter must open the restore form");
        assert!(
            state.handle_key(key(KeyCode::Char('1'))).is_noop(),
            "the open form must hold the switch keys"
        );
        assert!(state.sub == SettingsSub::Backups);

        state.handle_key(key(KeyCode::Esc));
        assert!(state.restore.is_none(), "Esc must close the restore form");
        let action = state.handle_key(key(KeyCode::Char('1')));
        assert!(state.sub == SettingsSub::Providers);
        assert!(matches!(action, SettingsAction::RefreshProviders));
    }

    /// Leaving and re-entering the Settings tab is the second escape. `sub`
    /// used to persist, so a modal sub-tab stayed in front of the operator for
    /// the rest of the session.
    #[test]
    fn reset_sub_returns_the_screen_to_its_landing_tab() {
        let mut state = on_backups_tab(&["config"]);
        state.handle_key(key(KeyCode::Enter));
        state.confirm_delete = true;
        state.status_msg = "stale".to_string();
        state.reset_sub();
        assert!(state.sub == SettingsSub::Providers);
        assert!(state.restore.is_none());
        assert!(!state.confirm_delete);
        assert!(
            state.status_msg.is_empty(),
            "the Providers pane must not inherit the Backups pane's status line"
        );
    }

    #[test]
    fn delete_asks_before_it_deletes() {
        let mut state = on_backups_tab(&["config"]);
        let action = state.handle_key(key(KeyCode::Char('d')));
        assert!(action.is_noop(), "the first press must only arm the prompt");
        assert!(state.confirm_delete);

        match state.handle_key(key(KeyCode::Char('y'))) {
            SettingsAction::DeleteBackup(name) => {
                assert_eq!(name, "librefang-backup-20260101-000000.zip");
            }
            _ => panic!("y must confirm the delete"),
        }
        assert!(!state.confirm_delete);
    }

    #[test]
    fn any_other_key_cancels_the_delete_prompt() {
        let mut state = on_backups_tab(&["config"]);
        state.handle_key(key(KeyCode::Char('d')));
        assert!(state.handle_key(key(KeyCode::Char('n'))).is_noop());
        assert!(!state.confirm_delete);
    }

    #[test]
    fn c_asks_the_daemon_for_a_new_archive() {
        let mut state = on_backups_tab(&["config"]);
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('c'))),
            SettingsAction::CreateBackup
        ));
    }

    #[test]
    fn enter_in_the_form_emits_the_restore_request() {
        let mut state = on_backups_tab(&["config", "skills"]);
        state.handle_key(key(KeyCode::Enter));
        match state.handle_key(key(KeyCode::Enter)) {
            SettingsAction::RestoreBackup(body) => {
                assert_eq!(
                    body["filename"],
                    serde_json::json!("librefang-backup-20260101-000000.zip")
                );
            }
            _ => panic!("Enter must submit the restore"),
        }
        assert!(state.restore.is_none(), "submitting must close the form");
    }

    /// Submitting an empty selection must keep the form open and say why,
    /// instead of firing a request the API answers with a 400.
    #[test]
    fn submitting_an_empty_selection_keeps_the_form_open() {
        let mut state = on_backups_tab(&["config"]);
        state.handle_key(key(KeyCode::Enter));
        state.handle_key(key(KeyCode::Down));
        state.handle_key(key(KeyCode::Char(' ')));
        assert!(state.handle_key(key(KeyCode::Enter)).is_noop());
        assert!(state.restore.is_some());
        assert!(!state.status_msg.is_empty());
    }

    #[test]
    fn the_form_cursor_wraps_over_every_row() {
        let mut state = on_backups_tab(&["config", "skills"]);
        state.handle_key(key(KeyCode::Enter));
        // 1 keep-config row + 2 component rows.
        for expected in [1, 2, 0] {
            state.handle_key(key(KeyCode::Down));
            assert_eq!(state.restore.as_ref().unwrap().cursor, expected);
        }
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.restore.as_ref().unwrap().cursor, 2);
    }

    #[test]
    fn sizes_render_without_a_space_before_the_unit() {
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(2048), "2.0KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0MiB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.0GiB");
    }

    impl SettingsAction {
        fn is_noop(&self) -> bool {
            matches!(self, SettingsAction::Continue)
        }
    }
}
