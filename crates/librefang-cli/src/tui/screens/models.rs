//! Models screen: the model catalogue with its operator-editable capacity limits.
//!
//! Refs #7774. A model's context window is a property of the model, and it was
//! reachable from the dashboard and the API but from nothing in the terminal —
//! an operator running LibreFang over SSH had no way to correct a window at all.
//!
//! The list shows the value **in force** next to the value the catalog would
//! supply, so a corrected model and an uncorrected one are distinguishable at a
//! glance, and a model nothing knows a window for is called out rather than
//! shown as a plausible-looking number.
//! Editing writes `model_overrides.json` through
//! `PUT /api/models/overrides/{provider}:{id}`, which is why the correction
//! survives a registry sync: it is keyed by model id and never attached to the
//! catalog entry the sync rewrites.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

/// One row of `GET /api/models`, reduced to what this screen renders.
///
/// The `*_effective` values are what the runtime actually uses; the `*_catalog`
/// values are what the registry or a discovery probe declared. Keeping both is
/// the whole point — an operator correcting a window needs to see what they are
/// correcting, and what a reset would restore.
#[derive(Clone, Default)]
pub struct ModelRow {
    pub id: String,
    pub provider: String,
    pub tier: String,
    /// Context window in force, or `0` when nothing knows one.
    pub context_window_effective: u64,
    /// Context window the catalog entry declares, or `0` when it declares none.
    pub context_window_catalog: u64,
    /// Maximum output tokens in force, or `0` when nothing knows one.
    pub max_output_tokens_effective: u64,
    /// Maximum output tokens the catalog entry declares, or `0` for none.
    pub max_output_tokens_catalog: u64,
}

impl ModelRow {
    /// The override key this model is stored under, `provider:model_id`.
    ///
    /// Deliberately built from the model as the operator sees it rather than
    /// from a resolved catalog entry: a gateway-served model has no entry at
    /// all, and it is exactly the case the override exists for.
    pub fn override_key(&self) -> String {
        format!("{}:{}", self.provider, self.id)
    }

    /// Whether an operator override is currently displacing a catalog value.
    ///
    /// A limit the catalog does not declare (`0`) but the effective value does
    /// counts: that is a window supplied entirely by the operator.
    pub fn has_limit_override(&self) -> bool {
        self.context_window_effective != self.context_window_catalog
            || self.max_output_tokens_effective != self.max_output_tokens_catalog
    }

    /// Whether the runtime has no context window for this model at all.
    ///
    /// The reported failure: the agent loop falls back to a conservative 8192
    /// and a conversation well inside the model's real window is refused for an
    /// overflow that only exists in that assumption.
    pub fn window_unknown(&self) -> bool {
        self.context_window_effective == 0
    }
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModelsSub {
    List,
    Edit,
}

/// Which limit the edit form is typing into.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    ContextWindow,
    MaxOutputTokens,
}

pub struct ModelsState {
    pub sub: ModelsSub,
    pub models: Vec<ModelRow>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
    /// Substring filter over `provider/id`, typed on the list.
    pub filter: String,
    /// True while the filter line is capturing keystrokes.
    pub filter_mode: bool,
    /// The model the edit form is bound to, captured on open so a background
    /// refresh reordering the list cannot retarget an in-progress edit.
    pub editing: Option<ModelRow>,
    pub edit_field: EditField,
    pub context_window_buf: String,
    pub max_output_tokens_buf: String,
}

pub enum ModelsAction {
    Continue,
    Refresh,
    /// Persist the operator's capacity limits for one model.
    ///
    /// `None` clears that limit and lets the catalog answer again; the sibling
    /// inference parameters stored under the same key are preserved by the
    /// caller, which merges rather than replacing the document.
    SaveLimits {
        key: String,
        context_window: Option<u64>,
        max_output_tokens: Option<u64>,
    },
    /// Drop this model's capacity-limit overrides and fall back to the catalog.
    ResetLimits {
        key: String,
    },
}

impl ModelsState {
    pub fn new() -> Self {
        Self {
            sub: ModelsSub::List,
            models: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            tick: 0,
            status_msg: String::new(),
            filter: String::new(),
            filter_mode: false,
            editing: None,
            edit_field: EditField::ContextWindow,
            context_window_buf: String::new(),
            max_output_tokens_buf: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// The rows the list is showing, after the filter.
    pub fn visible(&self) -> Vec<&ModelRow> {
        if self.filter.is_empty() {
            return self.models.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.models
            .iter()
            .filter(|m| {
                m.id.to_lowercase().contains(&needle) || m.provider.to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// The row the cursor is on, or `None` when the visible list is empty.
    pub fn selected(&self) -> Option<ModelRow> {
        let visible = self.visible();
        let idx = self.list_state.selected().unwrap_or(0);
        visible.get(idx).map(|m| (*m).clone())
    }

    /// Open the edit form on the selected model, pre-filled with the values in
    /// force.
    ///
    /// Pre-filling with the effective value rather than an empty box is what
    /// makes a correction an edit instead of a re-entry: the operator sees the
    /// number they are about to change. A limit nothing knows starts empty
    /// because there is no honest number to show.
    fn open_edit(&mut self) {
        let Some(row) = self.selected() else {
            return;
        };
        self.context_window_buf = if row.context_window_effective > 0 {
            row.context_window_effective.to_string()
        } else {
            String::new()
        };
        self.max_output_tokens_buf = if row.max_output_tokens_effective > 0 {
            row.max_output_tokens_effective.to_string()
        } else {
            String::new()
        };
        self.edit_field = EditField::ContextWindow;
        self.editing = Some(row);
        self.sub = ModelsSub::Edit;
        self.status_msg.clear();
    }

    /// Parse a field buffer: empty means "clear this override".
    ///
    /// A `0` is treated as empty too, matching the rule the catalog and the
    /// override layer already share — zero is "unknown", never a limit, and
    /// letting one through would pin a model's window to nothing.
    fn parsed(buf: &str) -> Option<u64> {
        buf.trim().parse::<u64>().ok().filter(|v| *v > 0)
    }

    fn active_buf_mut(&mut self) -> &mut String {
        match self.edit_field {
            EditField::ContextWindow => &mut self.context_window_buf,
            EditField::MaxOutputTokens => &mut self.max_output_tokens_buf,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ModelsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return ModelsAction::Continue;
        }
        match self.sub {
            ModelsSub::List => self.handle_list_key(key),
            ModelsSub::Edit => self.handle_edit_key(key),
        }
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> ModelsAction {
        // While the filter line is capturing, printable keys are filter text —
        // otherwise typing "de" to find deepseek would delete an override.
        if self.filter_mode {
            match key.code {
                KeyCode::Esc => {
                    self.filter_mode = false;
                    self.filter.clear();
                    self.list_state.select(Some(0));
                }
                KeyCode::Enter => self.filter_mode = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.list_state.select(Some(0));
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.list_state.select(Some(0));
                }
                _ => {}
            }
            return ModelsAction::Continue;
        }

        let total = self.visible().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1) % total));
            }
            KeyCode::Char('/') => {
                self.filter_mode = true;
                self.status_msg.clear();
            }
            KeyCode::Char('r') => return ModelsAction::Refresh,
            KeyCode::Enter | KeyCode::Char('e') => self.open_edit(),
            KeyCode::Char('d') => {
                if let Some(row) = self.selected() {
                    if row.has_limit_override() {
                        return ModelsAction::ResetLimits {
                            key: row.override_key(),
                        };
                    }
                    self.status_msg = crate::i18n::t("tui-models-status-no-override");
                }
            }
            _ => {}
        }
        ModelsAction::Continue
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> ModelsAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = ModelsSub::List;
                self.editing = None;
                return ModelsAction::Continue;
            }
            KeyCode::Tab | KeyCode::Down => {
                self.edit_field = EditField::MaxOutputTokens;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.edit_field = EditField::ContextWindow;
            }
            KeyCode::Backspace => {
                self.active_buf_mut().pop();
            }
            // Digits only: these are token counts, and accepting arbitrary text
            // would only defer the rejection to the server.
            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.active_buf_mut().push(c);
            }
            KeyCode::Enter => {
                let Some(row) = self.editing.clone() else {
                    self.sub = ModelsSub::List;
                    return ModelsAction::Continue;
                };
                let context_window = Self::parsed(&self.context_window_buf);
                let max_output_tokens = Self::parsed(&self.max_output_tokens_buf);
                self.sub = ModelsSub::List;
                self.editing = None;
                return ModelsAction::SaveLimits {
                    key: row.override_key(),
                    context_window,
                    max_output_tokens,
                };
            }
            _ => {}
        }
        ModelsAction::Continue
    }
}

impl Default for ModelsState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Render a limit as a token count, or a dash when nothing knows one.
fn limit_cell(value: u64) -> String {
    if value == 0 {
        "—".to_string()
    } else {
        value.to_string()
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut ModelsState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("▤ {}", crate::i18n::t("tui-models-title")),
    );
    match state.sub {
        ModelsSub::List => draw_list(f, inner, state),
        ModelsSub::Edit => draw_edit(f, inner, state),
    }
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut ModelsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // filter
        Constraint::Length(1), // column header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints / status
    ])
    .split(area);

    let filter_line = if state.filter_mode || !state.filter.is_empty() {
        widgets::search_input(&state.filter)
    } else {
        Paragraph::new(Line::from(Span::styled(
            format!(
                "  {}",
                crate::i18n::t_args(
                    "tui-models-count",
                    &[("count", &state.models.len().to_string())]
                )
            ),
            Style::default().fg(theme::TEXT_SECONDARY),
        )))
    };
    f.render_widget(filter_line, chunks[0]);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ", theme::table_header()),
            Span::styled(
                format!("{:<34}", crate::i18n::t("tui-models-header-model")),
                theme::table_header(),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
            Span::styled(
                format!("{:<12}", crate::i18n::t("tui-models-header-provider")),
                theme::table_header(),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
            Span::styled(
                format!("{:<12}", crate::i18n::t("tui-models-header-window")),
                theme::table_header(),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
            Span::styled(
                format!("{:<12}", crate::i18n::t("tui-models-header-catalog")),
                theme::table_header(),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
            Span::styled(
                crate::i18n::t("tui-models-header-max-output"),
                theme::table_header(),
            ),
        ])),
        chunks[1],
    );

    let visible = state.visible();
    if state.loading && visible.is_empty() {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-models-loading")),
            chunks[2],
        );
    } else if visible.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-models-empty")),
            chunks[2],
        );
    } else {
        let items: Vec<ListItem> = visible
            .iter()
            .map(|m| {
                let (window_text, window_style) = if m.window_unknown() {
                    (
                        crate::i18n::t("tui-models-window-unknown"),
                        Style::default().fg(theme::YELLOW),
                    )
                } else if m.has_limit_override() {
                    (
                        m.context_window_effective.to_string(),
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        m.context_window_effective.to_string(),
                        Style::default().fg(theme::TEXT_PRIMARY),
                    )
                };
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        format!("{:<34}", widgets::truncate(&m.id, 33)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{:<12}", widgets::truncate(&m.provider, 11)),
                        Style::default().fg(theme::PURPLE),
                    ),
                    Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
                    Span::styled(format!("{:<12}", window_text), window_style),
                    Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        format!("{:<12}", limit_cell(m.context_window_catalog)),
                        theme::dim_style(),
                    ),
                    Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        limit_cell(m.max_output_tokens_effective),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                ]))
            })
            .collect();
        f.render_stateful_widget(
            widgets::themed_list(items),
            chunks[2],
            &mut state.list_state,
        );
    }

    f.render_widget(
        widgets::status_or_hint(
            &state.status_msg,
            &format!("  {}", crate::i18n::t("tui-models-hints")),
        ),
        chunks[3],
    );
}

fn draw_edit(f: &mut Frame, area: Rect, state: &mut ModelsState) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // model being edited
        Constraint::Length(2), // context window field
        Constraint::Length(2), // max output field
        Constraint::Min(1),    // explanation
        Constraint::Length(1), // hints
    ])
    .split(area);

    let row = state.editing.clone().unwrap_or_default();

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                row.override_key(),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", row.tier),
                Style::default().fg(theme::TEXT_TERTIARY),
            ),
        ])),
        chunks[0],
    );

    let field_line = |label: String, buf: &str, catalog: u64, active: bool| {
        let marker = if active { "\u{25b8} " } else { "  " };
        let value_style = if active {
            theme::input_style()
        } else {
            Style::default().fg(theme::TEXT_PRIMARY)
        };
        let shown = if buf.is_empty() {
            crate::i18n::t("tui-models-field-cleared")
        } else {
            buf.to_string()
        };
        Paragraph::new(vec![Line::from(vec![
            Span::styled(marker, Style::default().fg(theme::ACCENT)),
            Span::styled(format!("{:<22}", label), theme::dim_style()),
            Span::styled(shown, value_style),
            Span::styled(
                format!(
                    "   {}",
                    crate::i18n::t_args(
                        "tui-models-field-catalog-hint",
                        &[("value", &limit_cell(catalog))]
                    )
                ),
                Style::default().fg(theme::TEXT_TERTIARY),
            ),
        ])])
    };

    f.render_widget(
        field_line(
            crate::i18n::t("tui-models-field-context-window"),
            &state.context_window_buf,
            row.context_window_catalog,
            state.edit_field == EditField::ContextWindow,
        ),
        chunks[1],
    );
    f.render_widget(
        field_line(
            crate::i18n::t("tui-models-field-max-output"),
            &state.max_output_tokens_buf,
            row.max_output_tokens_catalog,
            state.edit_field == EditField::MaxOutputTokens,
        ),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("  {}", crate::i18n::t("tui-models-edit-explainer")),
                Style::default().fg(theme::TEXT_SECONDARY),
            )),
            Line::from(Span::styled(
                format!("  {}", crate::i18n::t("tui-models-edit-not-max-tokens")),
                Style::default().fg(theme::YELLOW),
            )),
        ]),
        chunks[3],
    );

    f.render_widget(
        widgets::hint_bar(&format!("  {}", crate::i18n::t("tui-models-edit-hints"))),
        chunks[4],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn row(id: &str, effective: u64, catalog: u64) -> ModelRow {
        ModelRow {
            id: id.to_string(),
            provider: "litellm".to_string(),
            tier: "custom".to_string(),
            context_window_effective: effective,
            context_window_catalog: catalog,
            max_output_tokens_effective: 4_096,
            max_output_tokens_catalog: 4_096,
        }
    }

    fn state_with(rows: Vec<ModelRow>) -> ModelsState {
        let mut state = ModelsState::new();
        state.models = rows;
        state.list_state.select(Some(0));
        state
    }

    /// The gap #7774 names: the terminal had no models surface at all, so a
    /// window could be corrected from the dashboard and the API and from
    /// nowhere an SSH session can reach.
    #[test]
    fn editing_a_model_saves_the_window_under_its_override_key() {
        let mut state = state_with(vec![row("sensor-model-generic-high", 8_192, 0)]);

        state.handle_key(key(KeyCode::Enter));
        assert!(matches!(state.sub, ModelsSub::Edit));

        // Replace the pre-filled value with the model's real window.
        for _ in 0.."8192".len() {
            state.handle_key(key(KeyCode::Backspace));
        }
        for c in "16384".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        let action = state.handle_key(key(KeyCode::Enter));

        match action {
            ModelsAction::SaveLimits {
                key,
                context_window,
                max_output_tokens,
            } => {
                assert_eq!(
                    key, "litellm:sensor-model-generic-high",
                    "the override is keyed by provider:model_id, which is what makes it reachable for a model no catalog knows"
                );
                assert_eq!(context_window, Some(16_384));
                assert_eq!(max_output_tokens, Some(4_096));
            }
            _ => panic!("Enter in the edit form must persist the limits"),
        }
        assert!(
            matches!(state.sub, ModelsSub::List),
            "saving returns to the list"
        );
    }

    /// The form pre-fills with the value in force, so a correction is an edit
    /// rather than a re-entry — and an unknown window starts empty because
    /// there is no honest number to show.
    #[test]
    fn the_form_prefills_from_the_effective_value() {
        let mut state = state_with(vec![row("known", 32_768, 32_768)]);
        state.handle_key(key(KeyCode::Char('e')));
        assert_eq!(state.context_window_buf, "32768");

        let mut state = state_with(vec![ModelRow {
            context_window_effective: 0,
            context_window_catalog: 0,
            ..row("unknown-window", 0, 0)
        }]);
        state.handle_key(key(KeyCode::Char('e')));
        assert_eq!(state.context_window_buf, "");
    }

    /// An emptied field clears the override rather than submitting a zero.
    ///
    /// Zero is "unknown" everywhere this value is read; letting one through
    /// would pin the model's window to nothing and poison the budget math the
    /// override exists to fix.
    #[test]
    fn an_emptied_field_clears_the_override_instead_of_sending_zero() {
        let mut state = state_with(vec![row("m", 16_384, 32_768)]);
        state.handle_key(key(KeyCode::Enter));
        for _ in 0.."16384".len() {
            state.handle_key(key(KeyCode::Backspace));
        }
        let action = state.handle_key(key(KeyCode::Enter));

        match action {
            ModelsAction::SaveLimits { context_window, .. } => {
                assert_eq!(context_window, None);
            }
            _ => panic!("expected SaveLimits"),
        }
    }

    /// `d` reverts to the catalog only where there is something to revert.
    #[test]
    fn reset_is_offered_only_for_a_model_that_has_an_override() {
        let mut state = state_with(vec![row("overridden", 16_384, 32_768)]);
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('d'))),
            ModelsAction::ResetLimits { .. }
        ));

        let mut state = state_with(vec![row("untouched", 32_768, 32_768)]);
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('d'))),
            ModelsAction::Continue
        ));
        assert!(
            !state.status_msg.is_empty(),
            "a no-op key must say why it did nothing"
        );
    }

    /// A model the runtime has no window for is the reported failure, and the
    /// list has to be able to single it out.
    #[test]
    fn a_model_with_no_known_window_is_distinguishable() {
        let unknown = row("sensor-model-generic-high", 0, 0);
        assert!(unknown.window_unknown());
        assert!(!row("known", 32_768, 32_768).window_unknown());
    }

    /// An operator-supplied window for a model the catalog declares nothing for
    /// still counts as an override — otherwise `d` would refuse to revert the
    /// only case the override was created for.
    #[test]
    fn a_window_the_catalog_never_declared_counts_as_an_override() {
        assert!(row("gateway", 16_384, 0).has_limit_override());
    }

    /// Filter keystrokes must not double as list commands: typing "de" to find
    /// deepseek used to be one keystroke away from resetting an override.
    #[test]
    fn filter_typing_does_not_trigger_list_commands() {
        let mut state = state_with(vec![row("deepseek-v4", 16_384, 32_768)]);
        state.handle_key(key(KeyCode::Char('/')));
        for c in "de".chars() {
            assert!(matches!(
                state.handle_key(key(KeyCode::Char(c))),
                ModelsAction::Continue
            ));
        }
        assert_eq!(state.filter, "de");
        assert!(matches!(state.sub, ModelsSub::List));
    }

    #[test]
    fn the_filter_narrows_the_visible_rows() {
        let mut state = state_with(vec![
            row("deepseek-v4", 16_384, 16_384),
            row("claude-sonnet-4-6", 200_000, 200_000),
        ]);
        state.filter = "sonnet".to_string();
        let visible = state.visible();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "claude-sonnet-4-6");
    }

    #[test]
    fn escape_leaves_the_form_without_saving() {
        let mut state = state_with(vec![row("m", 16_384, 32_768)]);
        state.handle_key(key(KeyCode::Enter));
        let action = state.handle_key(key(KeyCode::Esc));
        assert!(matches!(action, ModelsAction::Continue));
        assert!(matches!(state.sub, ModelsSub::List));
        assert!(state.editing.is_none());
    }

    /// The form is bound to the model captured when it opened, so a background
    /// refresh that reorders the list cannot redirect the write to a different
    /// model.
    #[test]
    fn a_refresh_during_an_edit_does_not_retarget_the_write() {
        let mut state = state_with(vec![row("first", 8_192, 0), row("second", 8_192, 0)]);
        state.handle_key(key(KeyCode::Enter));

        state.models.reverse();

        let action = state.handle_key(key(KeyCode::Enter));
        match action {
            ModelsAction::SaveLimits { key, .. } => assert_eq!(key, "litellm:first"),
            _ => panic!("expected SaveLimits"),
        }
    }
}
