//! Generic configuration-section editor, rendered as a sub-tab of the Settings screen.
//!
//! The editor holds no field list of its own.
//! Sections, field names, types and the writable/read-only verdict all come from `GET /api/config/schema`, which is the same payload the dashboard's `ConfigPage` renders, and every write goes to `POST /api/config/set`, which is the same endpoint the dashboard posts to.
//! That is deliberate: a hardcoded panel per setting would need a TUI change every time a field is added to `KernelConfig`, and the two surfaces would drift on which paths are writable — the server already resolves that question in `is_writable_config_path` and ships the answer as `x-non-writable`.
//!
//! Current values come from `GET /api/config`, which is redacted, so a secret-bearing field shows its redaction marker rather than the secret.
//! Those fields are also in `x-non-writable`, so the editor renders them read-only and never offers to overwrite a redaction marker back onto the real value.

use crate::tui::screens::settings::SettingsAction;
use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::collections::{BTreeMap, BTreeSet};

// ── Data types ──────────────────────────────────────────────────────────────

/// Which editor a field's declared JSON type asks for.
///
/// `Complex` covers arrays and nested objects. Those are readable but not
/// editable here: `POST /api/config/set` accepts a wholesale JSON value at a
/// section path, and typing one into a single-line prompt is how an operator
/// clobbers a table they meant to amend. They stay edit-on-disk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKind {
    Bool,
    Integer,
    Number,
    Text,
    Complex,
}

/// One editable leaf, already resolved against the schema and the live config.
#[derive(Clone, Debug)]
pub struct ConfigField {
    /// Leaf name as the schema declares it, e.g. `registry_repo`.
    pub name: String,
    /// Dotted path posted to `POST /api/config/set`, e.g. `skills.registry_repo`.
    pub path: String,
    pub kind: FieldKind,
    /// Current value, `Null` when the key is absent from `GET /api/config`.
    pub value: serde_json::Value,
    /// False when the server listed this path under `x-non-writable`.
    pub writable: bool,
    /// Enum choices the schema declares, shown as a hint under the field.
    pub options: Vec<String>,
}

/// One group in the editor's left-hand list, mirroring one `x-sections` entry.
#[derive(Clone, Debug)]
pub struct ConfigSection {
    /// Section key as the schema declares it, e.g. `skills`.
    pub key: String,
    pub fields: Vec<ConfigField>,
}

// ── Schema parsing ──────────────────────────────────────────────────────────

/// Follow a property to whatever actually carries its `type` / `enum`.
///
/// schemars renders a plain scalar inline, an `Option<Scalar>` as
/// `"type": ["string", "null"]`, and a struct or enum as a `$ref` (or an
/// `allOf` / `anyOf` wrapping one). Resolving the indirection here is what
/// keeps `log_level` — an enum behind a `$ref` — an editable string rather
/// than falling through to `Complex`.
fn resolve_property<'a>(
    schema: &'a serde_json::Value,
    property: &'a serde_json::Value,
) -> &'a serde_json::Value {
    if property.get("type").is_some() || property.get("enum").is_some() {
        return property;
    }
    let referenced = property
        .get("$ref")
        .and_then(|v| v.as_str())
        .or_else(|| {
            ["allOf", "anyOf", "oneOf"].iter().find_map(|combinator| {
                property
                    .get(combinator)?
                    .as_array()?
                    .iter()
                    .find_map(|entry| entry.get("$ref")?.as_str())
            })
        })
        .and_then(|reference| reference.rsplit('/').next());
    match referenced.and_then(|name| schema.get("definitions")?.get(name)) {
        Some(definition) => definition,
        None => property,
    }
}

/// The declared type of a resolved property, ignoring the `null` half of an optional.
fn declared_type(resolved: &serde_json::Value) -> Option<&str> {
    match resolved.get("type")? {
        serde_json::Value::String(single) => Some(single.as_str()),
        serde_json::Value::Array(union) => union
            .iter()
            .filter_map(|entry| entry.as_str())
            .find(|entry| *entry != "null"),
        _ => None,
    }
}

fn field_kind(resolved: &serde_json::Value) -> FieldKind {
    match declared_type(resolved) {
        Some("boolean") => FieldKind::Bool,
        Some("integer") => FieldKind::Integer,
        Some("number") => FieldKind::Number,
        // A schemars enum reaches here as a `"type": "string"` with an `enum`
        // list, and a plain `String` as the same type with no list.
        Some("string") => FieldKind::Text,
        _ => FieldKind::Complex,
    }
}

fn enum_options(resolved: &serde_json::Value) -> Vec<String> {
    resolved
        .get("enum")
        .and_then(|v| v.as_array())
        .map(|choices| {
            choices
                .iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Build the editor's section list from `GET /api/config/schema` and `GET /api/config`.
///
/// Section order is the server's — `x-sections` is a curated array, so it is
/// already deterministic and it is also the order the dashboard shows.
/// Field order within a section is the curated `fields` array for the
/// root-level group, and sorted by name everywhere else: schema `definitions`
/// are JSON objects, whose iteration order is not something to render a list
/// from (#3298).
pub fn parse_config_sections(
    schema: &serde_json::Value,
    values: &serde_json::Value,
) -> Vec<ConfigSection> {
    let non_writable: BTreeSet<&str> = schema
        .get("x-non-writable")
        .and_then(|v| v.as_array())
        .map(|paths| paths.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default();
    let properties = schema.get("properties");

    let mut sections = Vec::new();
    for entry in schema
        .get("x-sections")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let Some(key) = entry.get("key").and_then(|v| v.as_str()) else {
            continue;
        };

        // Root-level group: the schema names the exact top-level scalars that
        // belong to it, in the order it wants them shown.
        let leaves: Vec<(String, &serde_json::Value, &serde_json::Value)> =
            if entry.get("root_level").and_then(|v| v.as_bool()) == Some(true) {
                entry
                    .get("fields")
                    .and_then(|v| v.as_array())
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|f| f.as_str())
                    .filter_map(|name| {
                        let property = properties?.get(name)?;
                        Some((
                            name.to_string(),
                            property,
                            values.get(name).unwrap_or(&serde_json::Value::Null),
                        ))
                    })
                    .collect()
            } else {
                let Some(struct_field) = entry.get("struct_field").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(nested) = properties
                    .and_then(|p| p.get(struct_field))
                    .map(|p| resolve_property(schema, p))
                    .and_then(|d| d.get("properties"))
                    .and_then(|v| v.as_object())
                else {
                    continue;
                };
                let section_values = values.get(struct_field);
                // BTreeMap rather than the object's own order (#3298).
                nested
                    .iter()
                    .map(|(name, property)| {
                        (
                            name.clone(),
                            (
                                property,
                                section_values
                                    .and_then(|v| v.get(name))
                                    .unwrap_or(&serde_json::Value::Null),
                            ),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .map(|(name, (property, value))| (name, property, value))
                    .collect()
            };

        let struct_field = entry.get("struct_field").and_then(|v| v.as_str());
        let fields: Vec<ConfigField> = leaves
            .into_iter()
            .map(|(name, property, value)| {
                let path = match struct_field {
                    Some(prefix) => format!("{prefix}.{name}"),
                    None => name.clone(),
                };
                let resolved = resolve_property(schema, property);
                let kind = field_kind(resolved);
                ConfigField {
                    writable: kind != FieldKind::Complex && !non_writable.contains(path.as_str()),
                    options: enum_options(resolved),
                    name,
                    path,
                    kind,
                    value: value.clone(),
                }
            })
            .collect();

        if !fields.is_empty() {
            sections.push(ConfigSection {
                key: key.to_string(),
                fields,
            });
        }
    }
    sections
}

/// Turn what the operator typed into the JSON `POST /api/config/set` expects,
/// or `None` when it does not parse as the field's declared type.
///
/// An empty input is `null`, which the endpoint treats as "remove this key"
/// rather than "write an empty string" — the only way to put an optional
/// field back to its compiled default from here.
pub fn parse_field_input(kind: FieldKind, raw: &str) -> Option<serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(serde_json::Value::Null);
    }
    match kind {
        FieldKind::Integer => trimmed.parse::<i64>().ok().map(serde_json::Value::from),
        FieldKind::Number => trimmed.parse::<f64>().ok().map(serde_json::Value::from),
        FieldKind::Text => Some(serde_json::Value::from(trimmed)),
        // Booleans are toggled, not typed; complex values are edit-on-disk.
        FieldKind::Bool | FieldKind::Complex => None,
    }
}

/// Render a value for the field list, and as the seed of the edit buffer.
///
/// `Null` renders empty rather than the JSON word, so re-submitting an
/// untouched prompt over an unset field leaves it unset.
pub fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `registry_repo` → `Registry repo`.
///
/// Section and field labels are derived rather than translated: the schema
/// carries hundreds of leaves and grows with every `KernelConfig` field, so a
/// hand-kept label table would be stale the day after it landed. The exact
/// config key stays visible in the detail line under the list, which is where
/// the convention puts the variable name.
fn humanize(key: &str) -> String {
    let mut out = String::new();
    for word in key.split(['_', '.']) {
        if word.is_empty() {
            continue;
        }
        if out.is_empty() {
            // Sentence case, not title case: these are settings names, and
            // `Proactive memory` reads as one while `Proactive Memory` reads
            // as a heading.
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        } else {
            out.push(' ');
            out.push_str(word);
        }
    }
    out
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ConfigEditorState {
    pub sections: Vec<ConfigSection>,
    pub section_list: ListState,
    pub field_list: ListState,
    /// Index of the expanded section. `None` while the group list has focus —
    /// the group level is the landing view and the fields sit behind it.
    pub expanded: Option<usize>,
    /// Edit buffer, `Some` only while a value prompt is open.
    pub input: Option<String>,
    pub loading: bool,
    pub status_msg: String,
}

impl ConfigEditorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the value prompt holds the keyboard, so the Settings screen
    /// knows not to read `1`-`5` as sub-tab switches.
    pub fn is_editing(&self) -> bool {
        self.input.is_some()
    }

    /// Drop every transient view state, keeping the loaded sections.
    pub fn reset(&mut self) {
        self.expanded = None;
        self.input = None;
        self.status_msg.clear();
        self.field_list.select(None);
    }

    /// Adopt a freshly fetched section list, keeping the operator where they were.
    ///
    /// A save triggers a refetch, so clobbering the cursors here would bounce
    /// the operator back to the top of the list after every single edit.
    pub fn set_sections(&mut self, sections: Vec<ConfigSection>) {
        self.sections = sections;
        self.loading = false;
        if self.sections.is_empty() {
            self.section_list.select(None);
            self.expanded = None;
            return;
        }
        let section = self
            .section_list
            .selected()
            .unwrap_or(0)
            .min(self.sections.len().saturating_sub(1));
        self.section_list.select(Some(section));
        if let Some(expanded) = self.expanded {
            if expanded >= self.sections.len() {
                self.expanded = None;
                self.field_list.select(None);
                return;
            }
            let total = self.sections[expanded].fields.len();
            if total == 0 {
                self.field_list.select(None);
            } else {
                let field = self.field_list.selected().unwrap_or(0).min(total - 1);
                self.field_list.select(Some(field));
            }
        }
    }

    fn selected_field(&self) -> Option<&ConfigField> {
        let section = self.sections.get(self.expanded?)?;
        section.fields.get(self.field_list.selected()?)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        if self.input.is_some() {
            return self.handle_input(key);
        }
        match self.expanded {
            None => self.handle_sections(key),
            Some(_) => self.handle_fields(key),
        }
    }

    fn handle_sections(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.sections.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.section_list.selected().unwrap_or(0);
                self.section_list
                    .select(Some(if i == 0 { total - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.section_list.selected().unwrap_or(0);
                self.section_list.select(Some((i + 1) % total));
            }
            KeyCode::Enter | KeyCode::Right if total > 0 => {
                let i = self.section_list.selected().unwrap_or(0);
                self.expanded = Some(i);
                self.field_list
                    .select((!self.sections[i].fields.is_empty()).then_some(0));
                self.status_msg.clear();
            }
            KeyCode::Char('r') => return SettingsAction::RefreshConfig,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_fields(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self
            .expanded
            .and_then(|i| self.sections.get(i))
            .map(|s| s.fields.len())
            .unwrap_or(0);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.field_list.selected().unwrap_or(0);
                self.field_list
                    .select(Some(if i == 0 { total - 1 } else { i - 1 }));
                self.status_msg.clear();
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.field_list.selected().unwrap_or(0);
                self.field_list.select(Some((i + 1) % total));
                self.status_msg.clear();
            }
            KeyCode::Esc | KeyCode::Left => {
                self.expanded = None;
                self.field_list.select(None);
                self.status_msg.clear();
            }
            KeyCode::Enter => return self.begin_edit(),
            KeyCode::Char('r') => return SettingsAction::RefreshConfig,
            _ => {}
        }
        SettingsAction::Continue
    }

    /// Open the prompt for the selected field, or say why it has none.
    ///
    /// A read-only field is answered here rather than round-tripped into a
    /// `403` toast: the server already told us the verdict in `x-non-writable`,
    /// so posting the write only to be refused would be a request made to
    /// learn something we were handed.
    fn begin_edit(&mut self) -> SettingsAction {
        let Some(field) = self.selected_field() else {
            return SettingsAction::Continue;
        };
        if field.kind == FieldKind::Complex {
            self.status_msg = crate::i18n::t_args(
                "tui-settings-config-complex",
                &[("path", field.path.as_str())],
            );
            return SettingsAction::Continue;
        }
        if !field.writable {
            self.status_msg = crate::i18n::t_args(
                "tui-settings-config-readonly-msg",
                &[("path", field.path.as_str())],
            );
            return SettingsAction::Continue;
        }
        if field.kind == FieldKind::Bool {
            let path = field.path.clone();
            let flipped = !field.value.as_bool().unwrap_or(false);
            self.status_msg.clear();
            return SettingsAction::SaveConfigValue {
                path,
                value: serde_json::Value::Bool(flipped),
            };
        }
        self.input = Some(render_value(&field.value));
        self.status_msg.clear();
        SettingsAction::Continue
    }

    fn handle_input(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.input = None;
            }
            KeyCode::Enter => {
                let Some(raw) = self.input.clone() else {
                    return SettingsAction::Continue;
                };
                let Some(field) = self.selected_field() else {
                    self.input = None;
                    return SettingsAction::Continue;
                };
                let path = field.path.clone();
                match parse_field_input(field.kind, &raw) {
                    Some(value) => {
                        self.input = None;
                        self.status_msg.clear();
                        return SettingsAction::SaveConfigValue { path, value };
                    }
                    // Keep the prompt open so the typed value is still there to fix.
                    None => {
                        self.status_msg =
                            crate::i18n::t_args("tui-settings-config-invalid", &[("path", &path)]);
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(buf) = self.input.as_mut() {
                    buf.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(buf) = self.input.as_mut() {
                    buf.push(c);
                }
            }
            _ => {}
        }
        SettingsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut ConfigEditorState, tick: usize) {
    if state.loading && state.sections.is_empty() {
        f.render_widget(
            widgets::spinner(tick, &crate::i18n::t("tui-settings-config-loading")),
            area,
        );
        return;
    }
    if state.sections.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-config-empty")),
            area,
        );
        return;
    }

    let panes = Layout::horizontal([Constraint::Percentage(32), Constraint::Min(24)]).split(area);
    draw_sections(f, panes[0], state);
    draw_fields(f, panes[1], state);
}

fn draw_sections(f: &mut Frame, area: Rect, state: &mut ConfigEditorState) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {}", crate::i18n::t("tui-settings-config-header-section")),
            theme::table_header(),
        ))),
        chunks[0],
    );

    let expanded = state.expanded;
    let items: Vec<ListItem> = state
        .sections
        .iter()
        .enumerate()
        .map(|(i, section)| {
            let marker = if expanded == Some(i) { "▾" } else { "▸" };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {marker} {}", humanize(&section.key)),
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(format!(" ({})", section.fields.len()), theme::dim_style()),
            ]))
        })
        .collect();
    f.render_stateful_widget(
        widgets::themed_list(items),
        chunks[1],
        &mut state.section_list,
    );
}

fn draw_fields(f: &mut Frame, area: Rect, state: &mut ConfigEditorState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(1),    // field list
        Constraint::Length(2), // detail line / value prompt
    ])
    .split(area);

    let Some(section) = state.expanded.and_then(|i| state.sections.get(i)) else {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-settings-config-select-section")),
            area,
        );
        return;
    };

    let name_hdr = crate::i18n::t("tui-settings-config-header-setting");
    let value_hdr = crate::i18n::t("tui-settings-config-header-value");
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {:<30} {}", name_hdr, value_hdr),
            theme::table_header(),
        ))),
        chunks[0],
    );

    let readonly_badge = crate::i18n::t("tui-settings-config-readonly");
    let unset = crate::i18n::t("tui-settings-config-unset");
    let items: Vec<ListItem> = section
        .fields
        .iter()
        .map(|field| {
            let (shown, value_style) = match (&field.value, field.kind) {
                (serde_json::Value::Null, _) => (unset.clone(), theme::dim_style()),
                (serde_json::Value::Bool(true), _) => (
                    format!("● {}", crate::i18n::t("tui-settings-config-on")),
                    Style::default().fg(theme::GREEN),
                ),
                (serde_json::Value::Bool(false), _) => (
                    format!("○ {}", crate::i18n::t("tui-settings-config-off")),
                    theme::dim_style(),
                ),
                (other, _) => (
                    widgets::truncate(&render_value(other), 46),
                    Style::default().fg(theme::TEXT),
                ),
            };
            let mut spans = vec![
                Span::styled(
                    format!("  {:<30}", widgets::truncate(&humanize(&field.name), 30)),
                    Style::default().fg(theme::TEXT_PRIMARY),
                ),
                Span::styled(format!(" {shown}"), value_style),
            ];
            if !field.writable {
                spans.push(Span::styled(
                    format!("  [{readonly_badge}]"),
                    Style::default().fg(theme::YELLOW),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let selected_field = state
        .field_list
        .selected()
        .and_then(|i| section.fields.get(i))
        .cloned();
    f.render_stateful_widget(
        widgets::themed_list(items),
        chunks[1],
        &mut state.field_list,
    );

    // Detail line, or the value prompt while one is open. The config key
    // itself lives here rather than in the label above, so the list reads as
    // settings and this line answers "which variable is that".
    let detail = match (&state.input, selected_field) {
        (Some(buf), Some(field)) => vec![
            Line::from(Span::styled(
                format!(
                    "  {}",
                    crate::i18n::t_args(
                        "tui-settings-config-prompt",
                        &[("path", field.path.as_str())]
                    )
                ),
                Style::default().fg(theme::YELLOW),
            )),
            Line::from(vec![
                Span::raw("  ▸ "),
                Span::styled(buf.clone(), theme::input_style()),
                Span::styled(
                    "█",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ]),
        ],
        (None, Some(field)) => {
            let options = if field.options.is_empty() {
                String::new()
            } else {
                format!("  {}", field.options.join(" | "))
            };
            vec![
                Line::from(Span::styled(
                    format!("  {}", field.path),
                    theme::dim_style(),
                )),
                Line::from(Span::styled(options, theme::hint_style())),
            ]
        }
        _ => Vec::new(),
    };
    f.render_widget(Paragraph::new(detail), chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A trimmed stand-in for `GET /api/config/schema`, carrying the shapes the
    /// parser has to survive: a curated root-level group, a `$ref`'d struct
    /// section, an optional string rendered as a type union, an enum behind a
    /// `$ref`, an array leaf, and a populated `x-non-writable` list.
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "properties": {
                "log_level": {"allOf": [{"$ref": "#/definitions/LogLevel"}]},
                "api_key": {"type": ["string", "null"]},
                "skills": {"$ref": "#/definitions/SkillsConfig"}
            },
            "definitions": {
                "LogLevel": {"type": "string", "enum": ["debug", "info", "warn"]},
                "SkillsConfig": {
                    "properties": {
                        "registry_repo": {"type": ["string", "null"]},
                        "auto_update": {"type": "boolean"},
                        "max_concurrent": {"type": "integer"},
                        "disabled": {"type": "array", "items": {"type": "string"}}
                    }
                }
            },
            "x-sections": [
                {"key": "general", "root_level": true, "fields": ["log_level", "api_key"]},
                {"key": "skills", "struct_field": "skills"}
            ],
            "x-non-writable": ["api_key"]
        })
    }

    fn values() -> serde_json::Value {
        serde_json::json!({
            "log_level": "info",
            "api_key": "***",
            "skills": {"auto_update": false, "max_concurrent": 4, "disabled": ["noisy"]}
        })
    }

    fn loaded() -> ConfigEditorState {
        let mut state = ConfigEditorState::new();
        state.set_sections(parse_config_sections(&schema(), &values()));
        state
    }

    fn field<'a>(state: &'a ConfigEditorState, path: &str) -> &'a ConfigField {
        state
            .sections
            .iter()
            .flat_map(|s| s.fields.iter())
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("the editor must expose {path}"))
    }

    /// Walk to a field by path so the keystroke tests do not hardcode indices.
    fn focus(state: &mut ConfigEditorState, path: &str) {
        let (section_idx, field_idx) = state
            .sections
            .iter()
            .enumerate()
            .find_map(|(si, s)| {
                s.fields
                    .iter()
                    .position(|f| f.path == path)
                    .map(|fi| (si, fi))
            })
            .unwrap_or_else(|| panic!("the editor must expose {path}"));
        state.section_list.select(Some(section_idx));
        state.handle_key(key(KeyCode::Enter));
        state.field_list.select(Some(field_idx));
    }

    /// The case the issue was filed over: `skills.registry_repo` is writable
    /// through `POST /api/config/set` and through the dashboard, and now
    /// through the TUI — without the TUI naming the field anywhere.
    #[test]
    fn the_registry_repo_setting_is_reachable_from_the_schema_alone() {
        let state = loaded();
        let registry = field(&state, "skills.registry_repo");
        assert_eq!(registry.kind, FieldKind::Text);
        assert!(
            registry.writable,
            "the server did not list skills.registry_repo as non-writable"
        );
        assert_eq!(registry.value, serde_json::Value::Null);
    }

    #[test]
    fn enter_on_a_text_field_posts_what_was_typed_to_its_config_path() {
        let mut state = loaded();
        focus(&mut state, "skills.registry_repo");
        assert!(matches!(
            state.handle_key(key(KeyCode::Enter)),
            SettingsAction::Continue
        ));
        assert!(state.is_editing(), "Enter must open the value prompt");
        for c in "acme/registry".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        match state.handle_key(key(KeyCode::Enter)) {
            SettingsAction::SaveConfigValue { path, value } => {
                assert_eq!(path, "skills.registry_repo");
                assert_eq!(value, serde_json::json!("acme/registry"));
            }
            _ => panic!("Enter must submit the typed value"),
        }
        assert!(!state.is_editing(), "submitting must close the prompt");
    }

    /// Clearing the prompt sends `null`, which the endpoint reads as "remove
    /// the key" — the only way back to the compiled default from here. An
    /// empty string would instead pin the field to `""`.
    #[test]
    fn an_emptied_prompt_removes_the_key_rather_than_writing_an_empty_string() {
        assert_eq!(
            parse_field_input(FieldKind::Text, "   "),
            Some(serde_json::Value::Null)
        );
    }

    /// The parser test above pins the mapping in isolation, which leaves the
    /// editor free to stop routing through it. Submitting the prompt without
    /// typing is how an operator actually asks for the default back, so drive
    /// that path end to end.
    #[test]
    fn submitting_an_untouched_prompt_posts_null_through_the_editor() {
        let mut state = loaded();
        focus(&mut state, "skills.registry_repo");
        state.handle_key(key(KeyCode::Enter));
        assert!(state.is_editing(), "Enter must open the value prompt");
        match state.handle_key(key(KeyCode::Enter)) {
            SettingsAction::SaveConfigValue { path, value } => {
                assert_eq!(path, "skills.registry_repo");
                assert_eq!(value, serde_json::Value::Null);
            }
            _ => panic!("submitting an empty prompt must post the removal"),
        }
        assert!(!state.is_editing(), "submitting must close the prompt");
    }

    #[test]
    fn a_boolean_field_toggles_without_opening_a_prompt() {
        let mut state = loaded();
        focus(&mut state, "skills.auto_update");
        match state.handle_key(key(KeyCode::Enter)) {
            SettingsAction::SaveConfigValue { path, value } => {
                assert_eq!(path, "skills.auto_update");
                assert_eq!(value, serde_json::json!(true), "false must flip to true");
            }
            _ => panic!("Enter on a boolean must submit the flip"),
        }
        assert!(!state.is_editing());
    }

    /// `x-non-writable` is the server's own verdict from `is_writable_config_path`,
    /// so a field it names must never produce a request that comes back 403.
    #[test]
    fn a_non_writable_field_is_never_offered_for_editing() {
        let mut state = loaded();
        assert!(!field(&state, "api_key").writable);
        focus(&mut state, "api_key");
        assert!(matches!(
            state.handle_key(key(KeyCode::Enter)),
            SettingsAction::Continue
        ));
        assert!(!state.is_editing());
        assert!(!state.status_msg.is_empty(), "the refusal must say why");
    }

    /// An array leaf is readable but not editable: a wholesale JSON value
    /// typed into a one-line prompt is how a table gets clobbered.
    #[test]
    fn an_array_field_is_readable_but_not_editable() {
        let mut state = loaded();
        let disabled = field(&state, "skills.disabled");
        assert_eq!(disabled.kind, FieldKind::Complex);
        assert!(!disabled.writable);
        focus(&mut state, "skills.disabled");
        state.handle_key(key(KeyCode::Enter));
        assert!(!state.is_editing());
    }

    /// A `$ref`'d enum must resolve to an editable string with its choices,
    /// not fall through to `Complex` because the property carried no inline type.
    #[test]
    fn an_enum_behind_a_ref_stays_editable_and_offers_its_choices() {
        let state = loaded();
        let log_level = field(&state, "log_level");
        assert_eq!(log_level.kind, FieldKind::Text);
        assert_eq!(log_level.options, vec!["debug", "info", "warn"]);
        assert_eq!(log_level.value, serde_json::json!("info"));
    }

    /// Field order inside a struct section must not depend on the schema
    /// object's iteration order (#3298).
    #[test]
    fn fields_within_a_section_are_sorted_by_name() {
        let state = loaded();
        let skills = state
            .sections
            .iter()
            .find(|s| s.key == "skills")
            .expect("the skills section must be present");
        let names: Vec<&str> = skills.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["auto_update", "disabled", "max_concurrent", "registry_repo"]
        );
    }

    /// The root-level group keeps the curated order the schema shipped.
    #[test]
    fn the_root_level_group_keeps_the_order_the_schema_declared() {
        let state = loaded();
        let general = state
            .sections
            .iter()
            .find(|s| s.key == "general")
            .expect("the general section must be present");
        let names: Vec<&str> = general.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["log_level", "api_key"]);
    }

    #[test]
    fn an_unparseable_number_keeps_the_prompt_open_with_the_typed_text() {
        let mut state = loaded();
        focus(&mut state, "skills.max_concurrent");
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            state.input.as_deref(),
            Some("4"),
            "the prompt seeds from the current value"
        );
        state.handle_key(key(KeyCode::Char('x')));
        assert!(matches!(
            state.handle_key(key(KeyCode::Enter)),
            SettingsAction::Continue
        ));
        assert_eq!(
            state.input.as_deref(),
            Some("4x"),
            "the bad text must stay editable"
        );
        assert!(!state.status_msg.is_empty());
    }

    /// Fields sit behind an expand, so the landing view is the group list.
    #[test]
    fn the_landing_view_is_the_group_list_and_esc_returns_to_it() {
        let mut state = loaded();
        assert!(state.expanded.is_none());
        state.section_list.select(Some(1));
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.expanded, Some(1));
        state.handle_key(key(KeyCode::Esc));
        assert!(
            state.expanded.is_none(),
            "Esc must collapse back to the groups"
        );
    }

    /// A save refetches, and the refetch must not bounce the operator back to
    /// the top of a list they were part-way down.
    #[test]
    fn a_refetch_keeps_the_operator_where_they_were() {
        let mut state = loaded();
        focus(&mut state, "skills.registry_repo");
        let (section, field_idx) = (state.section_list.selected(), state.field_list.selected());
        state.set_sections(parse_config_sections(&schema(), &values()));
        assert_eq!(state.section_list.selected(), section);
        assert_eq!(state.field_list.selected(), field_idx);
    }

    #[test]
    fn labels_read_as_settings_while_the_config_key_stays_exact() {
        assert_eq!(humanize("registry_repo"), "Registry repo");
        assert_eq!(humanize("skills"), "Skills");
        assert_eq!(humanize("proactive_memory"), "Proactive memory");
    }
}
