//! Channels screen: per-instance CRUD over `[[sidecar_channels]]`.
//!
//! The backend has supported several instances of one adapter type since
//! `[[sidecar_channels]]` became an array-of-tables, but the only way to add,
//! edit or drop one was to hand-edit `~/.librefang/config.toml`.
//! This screen drives the four endpoints that already exist, so an operator
//! can run two Telegram bots against two agents without opening an editor.
//!
//! # Instance name vs adapter name
//!
//! These are two different identifiers and the API keys its endpoints by
//! different ones, which is the source of the bugs reported in #8055 and #8063.
//! A channel named `slack-hr` runs on the adapter `slack`; the name is the
//! `[[sidecar_channels]].name` an operator chose, the adapter is the catalog
//! key that decides which sidecar executable is spawned and which field schema
//! the form renders.
//!
//! - `POST /api/channels/sidecar/{adapter}/configure` takes the **adapter** in
//!   the path — it is looked up in the daemon's sidecar catalog and a request
//!   carrying an instance name there is rejected with 404 — and the instance
//!   name in the body's `instance_name` field.
//! - `DELETE /api/channels/sidecar/{instance_name}` takes the **instance
//!   name** in the path, because it matches the `name` key of the
//!   `[[sidecar_channels]]` block it removes.
//!
//! So the same `{name}` path segment means opposite things on the two routes.
//! [`ConfigureRequest`] and [`ChannelAction::DeleteInstance`] keep the two
//! apart in the type, and the tests at the bottom of this file pin which
//! identifier reaches which endpoint.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::collections::BTreeMap;

// ── Data types ──────────────────────────────────────────────────────────────

/// One editable field of an adapter's `--describe` schema.
///
/// `value` is only ever populated for non-secret fields: the daemon returns
/// stored secrets as `has_value: true` with no value, so a secret can be
/// replaced from here but never read back.
#[derive(Clone, Default)]
pub struct ChannelFieldInfo {
    pub key: String,
    pub label: String,
    /// `text` / `secret` / `list` / `bool` / `select`, matching the daemon's
    /// `SidecarSchemaField.field_type`.
    pub field_type: String,
    pub required: bool,
    pub placeholder: String,
    pub advanced: bool,
    pub options: Vec<String>,
    pub value: String,
    pub has_value: bool,
}

impl ChannelFieldInfo {
    fn is_secret(&self) -> bool {
        self.field_type == "secret"
    }

    fn is_bool(&self) -> bool {
        self.field_type == "bool"
    }
}

/// A configured `[[sidecar_channels]]` entry.
#[derive(Clone, Default)]
pub struct ChannelInstance {
    /// `[[sidecar_channels]].name` — the instance identity, and the path
    /// segment `DELETE /api/channels/sidecar/{name}` expects.
    pub name: String,
    /// `[[sidecar_channels]].channel_type` — the adapter, and the path segment
    /// `POST /api/channels/sidecar/{name}/configure` expects. Falls back to
    /// `name` when the entry omits `channel_type`, exactly as the daemon does.
    pub adapter: String,
    /// Per-instance default agent, `None` when the entry binds no agent.
    pub agent: Option<String>,
    /// Whether the supervisor has an adapter registered under this name at all.
    pub supervised: bool,
    pub connected: bool,
    /// Sticky: the supervisor sets it on any failure and never clears it, so a
    /// connected instance that carries one is "was unhealthy at least once",
    /// not "is broken now".
    pub last_error: Option<String>,
    pub messages_received: u64,
    pub messages_sent: u64,
    pub fields: Vec<ChannelFieldInfo>,
}

/// An adapter type the daemon's sidecar catalog knows about.
#[derive(Clone, Default)]
pub struct ChannelAdapterInfo {
    /// Catalog key — the value that goes in the configure path.
    pub name: String,
    pub display_name: String,
    pub fields: Vec<ChannelFieldInfo>,
    /// Why `--describe` produced no schema, when it produced none. The picker
    /// marks such an adapter, but the form still opens on it: a save is
    /// refused by the daemon itself (`configure` answers 503 when no schema is
    /// cached) and that response carries the reason, which is more specific
    /// than anything this side knows.
    pub schema_error: Option<String>,
}

/// Per-instance health, collapsed to the four states the list can render.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstanceHealth {
    /// Connected, and the supervisor has never recorded a failure.
    Online,
    /// Connected, but carries a sticky `last_error` from an earlier failure.
    Degraded,
    /// Supervised but not connected.
    Stopped,
    /// No adapter is registered for this name — either `start_adapter` failed
    /// or the entry was written without a following channel reload.
    Unsupervised,
}

impl ChannelInstance {
    pub fn health(&self) -> InstanceHealth {
        if !self.supervised {
            InstanceHealth::Unsupervised
        } else if !self.connected {
            InstanceHealth::Stopped
        } else if self.last_error.is_some() {
            InstanceHealth::Degraded
        } else {
            InstanceHealth::Online
        }
    }
}

/// One rendered line of the grouped list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ChannelRow {
    /// A group heading for one adapter type, carrying how many instances it has.
    Header { adapter: String, count: usize },
    /// An index into [`ChannelState::instances`].
    Instance(usize),
    /// The trailing "add an instance" affordance.
    AddNew,
}

impl ChannelRow {
    fn selectable(&self) -> bool {
        !matches!(self, ChannelRow::Header { .. })
    }
}

// ── Request shape ───────────────────────────────────────────────────────────

/// A validated `POST /api/channels/sidecar/{adapter}/configure` call.
///
/// Both identifiers are carried separately and named for what they are, so a
/// caller cannot accidentally put the instance name in the path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConfigureRequest {
    /// Catalog key — goes in the URL path.
    pub adapter: String,
    /// `[[sidecar_channels]].name` — goes in the body.
    pub instance_name: String,
    /// `None` clears the per-instance default agent.
    pub agent: Option<String>,
    /// Schema field key → value. `BTreeMap` so the serialized body is stable
    /// across runs, which keeps the payload diffable in logs and tests.
    pub values: BTreeMap<String, String>,
}

impl ConfigureRequest {
    /// Path suffix, relative to the API root.
    ///
    /// Deliberately built from `adapter`: the daemon looks this segment up in
    /// its sidecar catalog, so an instance name here would 404.
    pub fn path(&self) -> String {
        format!("/api/channels/sidecar/{}/configure", self.adapter)
    }

    /// Request body matching the daemon's `ConfigureSidecarBody`.
    pub fn body(&self) -> serde_json::Value {
        serde_json::json!({
            "values": self.values,
            "instance_name": self.instance_name,
            "agent": self.agent,
        })
    }
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChannelSubScreen {
    List,
    /// Pick which adapter a brand-new instance runs on.
    AdapterPicker,
    Form,
    ConfirmDelete,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChannelFormMode {
    Create,
    /// Editing an existing instance. The name is fixed in this mode — see
    /// [`ChannelForm::name_editable`].
    Edit,
}

/// The add / edit form.
#[derive(Clone, Default)]
pub struct ChannelForm {
    pub adapter: String,
    pub name: String,
    pub agent: String,
    pub fields: Vec<ChannelFieldInfo>,
    /// 0 = instance name, 1 = default agent, 2.. = schema fields.
    pub focus: usize,
    pub error: Option<String>,
    mode: Option<ChannelFormMode>,
}

/// Focus indices of the two fixed fields that precede the schema fields.
const FOCUS_NAME: usize = 0;
const FOCUS_AGENT: usize = 1;
const FIXED_FIELDS: usize = 2;

impl ChannelForm {
    pub fn mode(&self) -> ChannelFormMode {
        self.mode.unwrap_or(ChannelFormMode::Create)
    }

    /// Renaming an instance is not an edit the API can express: `configure`
    /// upserts by name, so a save under a new name would append a second block
    /// and leave the original running. The name is therefore fixed once
    /// created — an operator who wants a different name deletes and re-adds.
    pub fn name_editable(&self) -> bool {
        self.mode() == ChannelFormMode::Create
    }

    fn focus_count(&self) -> usize {
        FIXED_FIELDS + self.fields.len()
    }

    /// Seed a create form from a catalog adapter.
    pub fn for_create(adapter: &ChannelAdapterInfo) -> Self {
        Self {
            adapter: adapter.name.clone(),
            name: adapter.name.clone(),
            agent: String::new(),
            fields: ordered_fields(&adapter.fields),
            focus: FOCUS_NAME,
            error: None,
            mode: Some(ChannelFormMode::Create),
        }
    }

    /// Seed an edit form from a configured instance.
    pub fn for_edit(instance: &ChannelInstance) -> Self {
        Self {
            adapter: instance.adapter.clone(),
            name: instance.name.clone(),
            agent: instance.agent.clone().unwrap_or_default(),
            fields: ordered_fields(&instance.fields),
            // The name is not editable here, so start on the first field the
            // operator can actually change.
            focus: FOCUS_AGENT,
            error: None,
            mode: Some(ChannelFormMode::Edit),
        }
    }

    /// Validate the form and build the request it would send.
    ///
    /// `existing` is every configured instance name, used for the create-time
    /// uniqueness check. Editing an instance necessarily reuses its own name,
    /// so that name is exempt.
    pub fn to_request(&self, existing: &[String]) -> Result<ConfigureRequest, String> {
        if self.adapter.trim().is_empty() {
            return Err(crate::i18n::t("tui-channels-error-no-adapter"));
        }
        let name = self.name.trim();
        if name.is_empty() {
            return Err(crate::i18n::t("tui-channels-error-name-required"));
        }
        // The instance name is both a `[[sidecar_channels]].name` and the path
        // segment of `DELETE /api/channels/sidecar/{name}`. A `/` in it stops
        // matching that route at all and a `?` or `#` truncates the segment, so
        // an instance saved under such a name could never be deleted from this
        // screen again. Refuse it at creation rather than writing an entry only
        // a text editor can remove. Existing entries are exempt: their name is
        // fixed here anyway, and rejecting one would make it uneditable too.
        if self.name_editable()
            && !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(crate::i18n::t_args(
                "tui-channels-error-name-invalid",
                &[("name", name)],
            ));
        }
        if self.name_editable() && existing.iter().any(|n| n == name) {
            return Err(crate::i18n::t_args(
                "tui-channels-error-name-taken",
                &[("name", name)],
            ));
        }
        // Distinct names are not enough. Every secret this instance stores goes
        // to `secrets.env` under `<PREFIX>__<KEY>`, and `instance_secret_prefix`
        // uppercases the name and maps every non-alphanumeric character to `_`,
        // so `slack-hr`, `slack_hr` and `slack.hr` — all three accepted by the
        // charset check above — collapse to one namespace. `build_spawn_env`
        // then hands each child every `<PREFIX>__KEY` in it, so two instances
        // with colliding names receive each other's tokens. The daemon already
        // detects this (`librefang_channels::sidecar::warn_secret_prefix_collisions`),
        // but only as a WARN in a log nobody is reading while filling in this
        // form, and only after the credentials are already on disk.
        // Refuse the name here instead: the collision is fully predictable from
        // the two names, and this screen is the thing creating it.
        if self.name_editable() {
            let prefix = librefang_channels::sidecar::instance_secret_prefix(name);
            if let Some(other) = existing
                .iter()
                .find(|n| librefang_channels::sidecar::instance_secret_prefix(n) == prefix)
            {
                return Err(crate::i18n::t_args(
                    "tui-channels-error-name-secret-collision",
                    &[("name", name), ("other", other), ("prefix", &prefix)],
                ));
            }
        }

        let mut values = BTreeMap::new();
        for field in &self.fields {
            let value = field.value.trim();
            if value.is_empty() {
                if field.required {
                    // A stored secret is not readable, so it cannot be
                    // resubmitted on the operator's behalf: the daemon
                    // validates `required` against the payload and would
                    // reject a save that omitted it. Say which case this is
                    // rather than letting the request fail server-side.
                    let key = if field.is_secret() && field.has_value {
                        "tui-channels-error-secret-required"
                    } else {
                        "tui-channels-error-field-required"
                    };
                    return Err(crate::i18n::t_args(key, &[("label", &field.label)]));
                }
                continue;
            }
            values.insert(field.key.clone(), value.to_string());
        }

        let agent = {
            let trimmed = self.agent.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };

        Ok(ConfigureRequest {
            adapter: self.adapter.clone(),
            instance_name: name.to_string(),
            agent,
            values,
        })
    }
}

pub struct ChannelState {
    pub sub: ChannelSubScreen,
    pub instances: Vec<ChannelInstance>,
    pub adapters: Vec<ChannelAdapterInfo>,
    /// Flattened, grouped view of `instances`; rebuilt whenever the list loads.
    pub rows: Vec<ChannelRow>,
    pub list_state: ListState,
    pub adapter_list_state: ListState,
    pub form: ChannelForm,
    /// Instance name queued for deletion while the confirm prompt is up.
    pub pending_delete: Option<String>,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum ChannelAction {
    Continue,
    Refresh,
    /// Add or update one instance.
    SaveInstance(ConfigureRequest),
    /// Remove one instance. Carries the **instance name**, which is what
    /// `DELETE /api/channels/sidecar/{name}` matches on.
    DeleteInstance {
        instance_name: String,
    },
    /// Re-read `[[sidecar_channels]]` and restart the sidecar children.
    ReloadChannels,
}

impl ChannelState {
    pub fn new() -> Self {
        Self {
            sub: ChannelSubScreen::List,
            instances: Vec::new(),
            adapters: Vec::new(),
            rows: Vec::new(),
            list_state: ListState::default(),
            adapter_list_state: ListState::default(),
            form: ChannelForm::default(),
            pending_delete: None,
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Replace the instance list and rebuild the grouped rows.
    pub fn set_instances(&mut self, instances: Vec<ChannelInstance>) {
        self.instances = instances;
        self.rebuild_rows();
        let first = self.rows.iter().position(ChannelRow::selectable);
        self.list_state.select(first);
    }

    /// Group instances by adapter type, sorted by adapter then instance name.
    ///
    /// Sorting is what makes the screen stable across refreshes — the config
    /// order is whatever the operator's file happens to be in, and the daemon
    /// preserves it, so an unsorted list would reshuffle under the cursor
    /// whenever a save rewrote the file.
    fn rebuild_rows(&mut self) {
        let mut grouped: BTreeMap<&str, Vec<(&str, usize)>> = BTreeMap::new();
        for (idx, instance) in self.instances.iter().enumerate() {
            grouped
                .entry(instance.adapter.as_str())
                .or_default()
                .push((instance.name.as_str(), idx));
        }
        let mut rows = Vec::new();
        for (adapter, mut members) in grouped {
            members.sort_by(|a, b| a.0.cmp(b.0));
            rows.push(ChannelRow::Header {
                adapter: adapter.to_string(),
                count: members.len(),
            });
            rows.extend(
                members
                    .into_iter()
                    .map(|(_, idx)| ChannelRow::Instance(idx)),
            );
        }
        rows.push(ChannelRow::AddNew);
        self.rows = rows;
    }

    /// Every configured instance name, for the uniqueness check.
    pub fn instance_names(&self) -> Vec<String> {
        self.instances.iter().map(|i| i.name.clone()).collect()
    }

    fn selected_instance(&self) -> Option<&ChannelInstance> {
        match self.rows.get(self.list_state.selected()?)? {
            ChannelRow::Instance(idx) => self.instances.get(*idx),
            _ => None,
        }
    }

    /// Move the list cursor by `delta` rows, skipping group headers and
    /// wrapping at both ends.
    fn move_selection(&mut self, delta: isize) {
        let selectable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.selectable())
            .map(|(i, _)| i)
            .collect();
        if selectable.is_empty() {
            self.list_state.select(None);
            return;
        }
        let current = self.list_state.selected().unwrap_or(selectable[0]);
        let pos = selectable.iter().position(|&i| i == current).unwrap_or(0) as isize;
        let len = selectable.len() as isize;
        let next = ((pos + delta) % len + len) % len;
        self.list_state.select(Some(selectable[next as usize]));
    }

    fn open_adapter_picker(&mut self) {
        self.sub = ChannelSubScreen::AdapterPicker;
        if self.adapter_list_state.selected().is_none() && !self.adapters.is_empty() {
            self.adapter_list_state.select(Some(0));
        }
    }

    /// Open the edit form for the selected instance. A no-op when the cursor
    /// sits on a group header or the add row, neither of which is an instance.
    fn open_edit(&mut self) {
        if let Some(instance) = self.selected_instance() {
            self.form = ChannelForm::for_edit(instance);
            self.sub = ChannelSubScreen::Form;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ChannelAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return ChannelAction::Continue;
        }
        match self.sub {
            ChannelSubScreen::List => self.handle_list(key),
            ChannelSubScreen::AdapterPicker => self.handle_adapter_picker(key),
            ChannelSubScreen::Form => self.handle_form(key),
            ChannelSubScreen::ConfirmDelete => self.handle_confirm_delete(key),
        }
    }

    fn handle_list(&mut self, key: KeyEvent) -> ChannelAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Enter => {
                let is_add = matches!(
                    self.list_state.selected().and_then(|i| self.rows.get(i)),
                    Some(ChannelRow::AddNew)
                );
                if is_add {
                    self.open_adapter_picker();
                } else {
                    self.open_edit();
                }
            }
            KeyCode::Char('a') | KeyCode::Char('n') => self.open_adapter_picker(),
            KeyCode::Char('e') => self.open_edit(),
            KeyCode::Char('d') => {
                if let Some(instance) = self.selected_instance() {
                    self.pending_delete = Some(instance.name.clone());
                    self.sub = ChannelSubScreen::ConfirmDelete;
                }
            }
            KeyCode::Char('r') => return ChannelAction::Refresh,
            KeyCode::Char('R') => return ChannelAction::ReloadChannels,
            _ => {}
        }
        ChannelAction::Continue
    }

    fn handle_adapter_picker(&mut self, key: KeyEvent) -> ChannelAction {
        let total = self.adapters.len();
        match key.code {
            KeyCode::Esc => self.sub = ChannelSubScreen::List,
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                let i = self.adapter_list_state.selected().unwrap_or(0);
                self.adapter_list_state
                    .select(Some(if i == 0 { total - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                let i = self.adapter_list_state.selected().unwrap_or(0);
                self.adapter_list_state.select(Some((i + 1) % total));
            }
            KeyCode::Enter => {
                if let Some(adapter) = self
                    .adapter_list_state
                    .selected()
                    .and_then(|i| self.adapters.get(i))
                {
                    self.form = ChannelForm::for_create(adapter);
                    // A second instance cannot reuse the adapter name, which
                    // the first instance conventionally holds, so pre-suffix
                    // the seed rather than opening a form that fails its own
                    // uniqueness check.
                    let existing = self.instance_names();
                    if !name_is_free(&self.form.name, &existing) {
                        self.form.name = next_free_name(&self.form.adapter, &existing);
                    }
                    self.sub = ChannelSubScreen::Form;
                }
            }
            _ => {}
        }
        ChannelAction::Continue
    }

    fn handle_form(&mut self, key: KeyEvent) -> ChannelAction {
        let count = self.form.focus_count();
        match key.code {
            KeyCode::Esc => {
                self.sub = ChannelSubScreen::List;
                self.form.error = None;
            }
            // `Tab` and `BackTab` are consumed by the global tab-cycling in
            // `tui/mod.rs::handle_key` before any screen sees them, so in the
            // running TUI only the arrow keys move field focus — which is what
            // `tui-channels-hints-form` advertises. The arms stay because they
            // are the correct bindings for this widget if that ever yields.
            KeyCode::Up | KeyCode::BackTab => {
                self.form.focus = if self.form.focus == 0 {
                    count - 1
                } else {
                    self.form.focus - 1
                };
                self.skip_locked_focus(-1);
            }
            KeyCode::Down | KeyCode::Tab => {
                self.form.focus = (self.form.focus + 1) % count;
                self.skip_locked_focus(1);
            }
            KeyCode::Enter => {
                let existing = self.instance_names();
                match self.form.to_request(&existing) {
                    Ok(request) => {
                        self.form.error = None;
                        self.sub = ChannelSubScreen::List;
                        return ChannelAction::SaveInstance(request);
                    }
                    Err(message) => self.form.error = Some(message),
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                // Cycle the choice-shaped field types in place; a space in a
                // free-text field still types a space.
                if let Some(field) = self.focused_field_mut() {
                    if field.is_bool() {
                        field.value = if field.value == "true" {
                            "false".to_string()
                        } else {
                            "true".to_string()
                        };
                        return ChannelAction::Continue;
                    }
                    if !field.options.is_empty() {
                        let forward = !matches!(key.code, KeyCode::Left);
                        cycle_option(field, forward);
                        return ChannelAction::Continue;
                    }
                }
                if key.code == KeyCode::Char(' ') {
                    self.push_char(' ');
                }
            }
            KeyCode::Char(c) => self.push_char(c),
            KeyCode::Backspace => match self.form.focus {
                FOCUS_NAME if self.form.name_editable() => {
                    self.form.name.pop();
                }
                FOCUS_AGENT => {
                    self.form.agent.pop();
                }
                _ => {
                    if let Some(field) = self.focused_field_mut() {
                        field.value.pop();
                    }
                }
            },
            _ => {}
        }
        ChannelAction::Continue
    }

    /// Step past the instance-name row when it is not editable.
    fn skip_locked_focus(&mut self, delta: isize) {
        if self.form.focus == FOCUS_NAME && !self.form.name_editable() {
            let count = self.form.focus_count() as isize;
            let next = ((self.form.focus as isize + delta) % count + count) % count;
            self.form.focus = next as usize;
        }
    }

    fn focused_field_mut(&mut self) -> Option<&mut ChannelFieldInfo> {
        self.form
            .focus
            .checked_sub(FIXED_FIELDS)
            .and_then(|i| self.form.fields.get_mut(i))
    }

    fn push_char(&mut self, c: char) {
        match self.form.focus {
            FOCUS_NAME if self.form.name_editable() => self.form.name.push(c),
            FOCUS_NAME => {}
            FOCUS_AGENT => self.form.agent.push(c),
            _ => {
                if let Some(field) = self.focused_field_mut() {
                    field.value.push(c);
                }
            }
        }
    }

    fn handle_confirm_delete(&mut self, key: KeyEvent) -> ChannelAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let pending = self.pending_delete.take();
                self.sub = ChannelSubScreen::List;
                if let Some(instance_name) = pending {
                    return ChannelAction::DeleteInstance { instance_name };
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.pending_delete = None;
                self.sub = ChannelSubScreen::List;
            }
            _ => {}
        }
        ChannelAction::Continue
    }
}

impl Default for ChannelState {
    fn default() -> Self {
        Self::new()
    }
}

/// Basic fields first, advanced ones last, each keeping its schema order.
///
/// An adapter's `--describe` schema interleaves the two, and an `advanced`
/// knob sitting between two required credentials reads as though it were part
/// of the minimum setup. Ordering them puts everything an operator must fill
/// at the top of the form, where the cursor starts.
fn ordered_fields(fields: &[ChannelFieldInfo]) -> Vec<ChannelFieldInfo> {
    let mut ordered: Vec<ChannelFieldInfo> =
        fields.iter().filter(|f| !f.advanced).cloned().collect();
    ordered.extend(fields.iter().filter(|f| f.advanced).cloned());
    ordered
}

/// Advance a `select` field to its next (or previous) option.
///
/// A value that is not among `options` (a select field with no default,
/// which has not been touched yet) is treated as sitting *before* the first
/// option rather than *on* it — otherwise the first `Right` press would jump
/// straight to `options[1]`, silently skipping `options[0]`.
fn cycle_option(field: &mut ChannelFieldInfo, forward: bool) {
    let len = field.options.len();
    let next = match field.options.iter().position(|o| o == &field.value) {
        Some(current) => {
            if forward {
                (current + 1) % len
            } else {
                (current + len - 1) % len
            }
        }
        None => {
            if forward {
                0
            } else {
                len - 1
            }
        }
    };
    field.value = field.options[next].clone();
}

/// First `{adapter}-{n}` name free of every existing instance, starting at 2.
///
/// `2` rather than `1` because the unsuffixed adapter name is the first
/// instance by convention, so the next one an operator adds is the second.
///
/// "Free" is tested on the secret prefix, not the literal name, because that
/// is the stricter of the two checks [`ChannelForm::to_request`] applies — an
/// existing `telegram_2` makes `telegram-2` unusable even though the two
/// strings differ. Seeding a name the form would immediately reject is a
/// dead end an operator cannot resolve without guessing what is wrong.
pub fn next_free_name(adapter: &str, existing: &[String]) -> String {
    let taken: std::collections::BTreeSet<String> = existing
        .iter()
        .map(|n| librefang_channels::sidecar::instance_secret_prefix(n))
        .collect();
    (2..)
        .map(|n| format!("{adapter}-{n}"))
        .find(|candidate| {
            !taken.contains(&librefang_channels::sidecar::instance_secret_prefix(
                candidate,
            ))
        })
        .unwrap_or_else(|| adapter.to_string())
}

/// Whether `name` can still be used for a new instance, by both the
/// exact-name and the secret-prefix test.
fn name_is_free(name: &str, existing: &[String]) -> bool {
    let prefix = librefang_channels::sidecar::instance_secret_prefix(name);
    !existing
        .iter()
        .any(|n| n == name || librefang_channels::sidecar::instance_secret_prefix(n) == prefix)
}

// ── Rendering ───────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut ChannelState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("◈ {}", crate::i18n::t("tui-channels-title-screen")),
    );

    match state.sub {
        ChannelSubScreen::List | ChannelSubScreen::ConfirmDelete => draw_list(f, inner, state),
        ChannelSubScreen::AdapterPicker => draw_adapter_picker(f, inner, state),
        ChannelSubScreen::Form => draw_form(f, inner, state),
    }
}

fn health_span(instance: &ChannelInstance) -> Span<'static> {
    let (key, color) = match instance.health() {
        InstanceHealth::Online => ("tui-channels-status-online", theme::GREEN),
        InstanceHealth::Degraded => ("tui-channels-status-degraded", theme::YELLOW),
        InstanceHealth::Stopped => ("tui-channels-status-stopped", theme::RED),
        InstanceHealth::Unsupervised => ("tui-channels-status-unsupervised", theme::TEXT_TERTIARY),
    };
    Span::styled(
        format!(" {:<13}", crate::i18n::t(key)),
        Style::default().fg(color),
    )
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut ChannelState) {
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
                "  {:<20} {:<16} {:<13} {}",
                crate::i18n::t("tui-channels-header-instance"),
                crate::i18n::t("tui-channels-header-agent"),
                crate::i18n::t("tui-channels-header-status"),
                crate::i18n::t("tui-channels-header-traffic")
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );
    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-channels-loading")),
            chunks[2],
        );
    } else if state.instances.is_empty() && state.rows.len() <= 1 {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-channels-empty-state")),
            chunks[2],
        );
    } else {
        let unbound = crate::i18n::t("tui-channels-agent-unbound");
        let items: Vec<ListItem> = state
            .rows
            .iter()
            .map(|row| match row {
                ChannelRow::Header { adapter, count } => ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {adapter}"),
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        crate::i18n::t_args(
                            "tui-channels-group-count",
                            &[("count", &count.to_string())],
                        ),
                        theme::dim_style(),
                    ),
                ])),
                ChannelRow::Instance(idx) => {
                    let instance = &state.instances[*idx];
                    let agent = instance.agent.as_deref().unwrap_or(unbound.as_str());
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("    {:<18}", widgets::truncate(&instance.name, 17)),
                            Style::default().fg(theme::CYAN),
                        ),
                        Span::styled(
                            format!(" {:<16}", widgets::truncate(agent, 15)),
                            Style::default().fg(theme::YELLOW),
                        ),
                        health_span(instance),
                        Span::styled(
                            format!(" {}/{}", instance.messages_received, instance.messages_sent),
                            Style::default().fg(theme::TEXT_SECONDARY),
                        ),
                    ]))
                }
                ChannelRow::AddNew => ListItem::new(Line::from(vec![Span::styled(
                    crate::i18n::t("tui-channels-add-new-option"),
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                )])),
            })
            .collect();
        f.render_stateful_widget(
            widgets::themed_list(items),
            chunks[2],
            &mut state.list_state,
        );
    }

    let confirming = state.sub == ChannelSubScreen::ConfirmDelete;
    let confirm_msg = state
        .pending_delete
        .as_deref()
        .map(|name| crate::i18n::t_args("tui-channels-confirm-delete", &[("name", name)]))
        .unwrap_or_default();
    f.render_widget(
        widgets::confirm_or_status_or_hint(
            confirming,
            &confirm_msg,
            &state.status_msg,
            &crate::i18n::t("tui-channels-hints-list"),
        ),
        chunks[3],
    );
}

fn draw_adapter_picker(f: &mut Frame, area: Rect, state: &mut ChannelState) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {}", crate::i18n::t("tui-channels-title-picker")),
            Style::default()
                .fg(theme::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )])),
        chunks[0],
    );
    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    if state.adapters.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-channels-picker-empty")),
            chunks[2],
        );
    } else {
        let items: Vec<ListItem> = state
            .adapters
            .iter()
            .map(|adapter| {
                let mut spans = vec![
                    Span::styled(
                        format!("  {:<16}", widgets::truncate(&adapter.name, 15)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&adapter.display_name, 32)),
                        Style::default().fg(theme::TEXT_SECONDARY),
                    ),
                ];
                if adapter.schema_error.is_some() || adapter.fields.is_empty() {
                    spans.push(Span::styled(
                        format!("  {}", crate::i18n::t("tui-channels-no-schema")),
                        Style::default().fg(theme::YELLOW),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        f.render_stateful_widget(
            widgets::themed_list(items),
            chunks[2],
            &mut state.adapter_list_state,
        );
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-channels-hints-picker")),
        chunks[3],
    );
}

/// One `label: value` row of the form, with a cursor when focused.
fn field_line(label: String, value: String, focused: bool, dim: bool) -> Line<'static> {
    let marker = if focused { "\u{25b8} " } else { "  " };
    let value_style = if dim {
        theme::dim_style()
    } else {
        theme::input_style()
    };
    let mut spans = vec![
        Span::styled(
            format!("{marker}{label:<26}"),
            if focused {
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT_SECONDARY)
            },
        ),
        Span::styled(value, value_style),
    ];
    if focused {
        spans.push(Span::styled(
            "\u{2588}",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
    }
    Line::from(spans)
}

fn draw_form(f: &mut Frame, area: Rect, state: &mut ChannelState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // separator
        Constraint::Min(4),    // fields
        Constraint::Length(1), // error
        Constraint::Length(1), // hints
    ])
    .split(area);

    let title_key = match state.form.mode() {
        ChannelFormMode::Create => "tui-channels-title-create",
        ChannelFormMode::Edit => "tui-channels-title-edit",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("  {}", crate::i18n::t(title_key)),
                Style::default()
                    .fg(theme::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  [{}]", state.form.adapter),
                Style::default().fg(theme::CYAN),
            ),
        ])),
        chunks[0],
    );
    f.render_widget(widgets::separator(chunks[1].width), chunks[1]);

    let mut lines = Vec::new();
    let name_label = if state.form.name_editable() {
        crate::i18n::t("tui-channels-label-instance-name")
    } else {
        crate::i18n::t("tui-channels-label-instance-name-locked")
    };
    lines.push(field_line(
        name_label,
        state.form.name.clone(),
        state.form.focus == FOCUS_NAME && state.form.name_editable(),
        !state.form.name_editable(),
    ));
    let agent_value = if state.form.agent.is_empty() {
        crate::i18n::t("tui-channels-placeholder-agent")
    } else {
        state.form.agent.clone()
    };
    lines.push(field_line(
        crate::i18n::t("tui-channels-label-agent"),
        agent_value,
        state.form.focus == FOCUS_AGENT,
        state.form.agent.is_empty(),
    ));

    for (i, field) in state.form.fields.iter().enumerate() {
        let focused = state.form.focus == FIXED_FIELDS + i;
        let required_mark = if field.required { "*" } else { " " };
        let label = format!("{}{required_mark}", field.label);
        // A stored secret is never echoed back by the daemon; show that it is
        // set and that saving needs it typed again.
        let (value, dim) = if field.value.is_empty() {
            if field.is_secret() && field.has_value {
                (crate::i18n::t("tui-channels-secret-stored"), true)
            } else if field.placeholder.is_empty() {
                (String::new(), true)
            } else {
                (field.placeholder.clone(), true)
            }
        } else if field.is_secret() {
            ("\u{2022}".repeat(field.value.chars().count()), false)
        } else {
            (field.value.clone(), false)
        };
        lines.push(field_line(label, value, focused, dim));
    }

    f.render_widget(Paragraph::new(lines), chunks[2]);

    if let Some(error) = &state.form.error {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {error}"),
                Style::default().fg(theme::RED),
            ))),
            chunks[3],
        );
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-channels-hints-form")),
        chunks[4],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(state: &mut ChannelState, text: &str) {
        for c in text.chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn secret(key: &str) -> ChannelFieldInfo {
        ChannelFieldInfo {
            key: key.to_string(),
            label: key.to_string(),
            field_type: "secret".to_string(),
            required: true,
            ..Default::default()
        }
    }

    fn telegram_adapter() -> ChannelAdapterInfo {
        ChannelAdapterInfo {
            name: "telegram".to_string(),
            display_name: "Telegram".to_string(),
            fields: vec![
                secret("TELEGRAM_BOT_TOKEN"),
                ChannelFieldInfo {
                    key: "ALLOWED_USERS".to_string(),
                    label: "Allowed users".to_string(),
                    field_type: "list".to_string(),
                    ..Default::default()
                },
            ],
            schema_error: None,
        }
    }

    fn instance(name: &str, adapter: &str, agent: Option<&str>) -> ChannelInstance {
        ChannelInstance {
            name: name.to_string(),
            adapter: adapter.to_string(),
            agent: agent.map(str::to_string),
            supervised: true,
            connected: true,
            fields: vec![secret("TELEGRAM_BOT_TOKEN")],
            ..Default::default()
        }
    }

    /// The whole point of the screen: a save must put the adapter in the path
    /// and the instance name in the body, never the other way round.
    #[test]
    fn create_sends_adapter_in_path_and_instance_name_in_body() {
        let mut state = ChannelState::new();
        state.adapters = vec![telegram_adapter()];
        state.set_instances(vec![instance("telegram", "telegram", Some("alice"))]);

        state.handle_key(key(KeyCode::Char('a')));
        assert_eq!(state.sub, ChannelSubScreen::AdapterPicker);
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.sub, ChannelSubScreen::Form);

        // Seeded away from the taken `telegram`, then replaced wholesale.
        assert_eq!(state.form.name, "telegram-2");
        for _ in 0.."telegram-2".len() {
            state.handle_key(key(KeyCode::Backspace));
        }
        typed(&mut state, "telegram-support");
        state.handle_key(key(KeyCode::Down));
        typed(&mut state, "bob");
        state.handle_key(key(KeyCode::Down));
        typed(&mut state, "12345:secret");

        let action = state.handle_key(key(KeyCode::Enter));
        let request = match action {
            ChannelAction::SaveInstance(request) => request,
            _ => panic!("expected a save"),
        };

        assert_eq!(request.adapter, "telegram");
        assert_eq!(request.instance_name, "telegram-support");
        assert_eq!(
            request.path(),
            "/api/channels/sidecar/telegram/configure",
            "the configure path is keyed by adapter; an instance name here 404s"
        );
        assert_eq!(request.agent.as_deref(), Some("bob"));
        assert_eq!(
            request.values.get("TELEGRAM_BOT_TOKEN").map(String::as_str),
            Some("12345:secret")
        );
        // Optional and untouched, so it is not sent at all.
        assert!(!request.values.contains_key("ALLOWED_USERS"));

        let body = request.body();
        assert_eq!(body["instance_name"], "telegram-support");
        assert_eq!(body["agent"], "bob");
        assert_eq!(body["values"]["TELEGRAM_BOT_TOKEN"], "12345:secret");
        assert_eq!(state.sub, ChannelSubScreen::List);
    }

    #[test]
    fn duplicate_instance_name_is_rejected_before_any_request() {
        let mut state = ChannelState::new();
        state.adapters = vec![telegram_adapter()];
        state.set_instances(vec![instance("telegram", "telegram", None)]);

        let mut form = ChannelForm::for_create(&telegram_adapter());
        form.name = "telegram".to_string();
        form.fields[0].value = "token".to_string();
        state.form = form;
        state.sub = ChannelSubScreen::Form;

        let action = state.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, ChannelAction::Continue));
        assert_eq!(state.sub, ChannelSubScreen::Form, "the form stays open");
        let error = state.form.error.clone().expect("a collision is an error");
        assert!(
            error.contains("telegram"),
            "the message must name the collision, got: {error}"
        );
    }

    /// The instance name is a URL path segment on the delete route, so a `/`
    /// in it would produce a request that no longer matches that route at all
    /// — the instance would be creatable and then undeletable from here.
    #[test]
    fn an_instance_name_that_would_break_the_delete_path_is_refused() {
        let mut form = ChannelForm::for_create(&telegram_adapter());
        form.fields[0].value = "token".to_string();

        for bad in ["tele/gram", "tele gram", "tele?gram", "tele#gram"] {
            form.name = bad.to_string();
            let error = form
                .to_request(&[])
                .expect_err("a name with URL punctuation cannot round-trip through the API path");
            assert!(
                error.contains(bad),
                "the message must name the input: {error}"
            );
        }

        form.name = "telegram-support.2_b".to_string();
        assert!(
            form.to_request(&[]).is_ok(),
            "letters, digits, `-`, `_` and `.` are all usable"
        );
    }

    /// Two names that differ only in punctuation collapse to one
    /// `instance_secret_prefix`, and `build_spawn_env` then hands each child
    /// every `<PREFIX>__KEY` in that namespace — so the second instance would
    /// silently receive the first one's credentials. The exact-name uniqueness
    /// check does not see it, because the names really are different.
    #[test]
    fn a_name_that_shares_a_secret_prefix_with_an_existing_instance_is_refused() {
        let existing = vec!["slack-hr".to_string()];
        for colliding in ["slack_hr", "slack.hr", "SLACK-HR"] {
            let mut form = ChannelForm::for_create(&telegram_adapter());
            form.name = colliding.to_string();
            form.fields[0].value = "token".to_string();

            let error = form
                .to_request(&existing)
                .expect_err("a colliding secret prefix must be refused before the write");
            assert!(
                error.contains("SLACK_HR"),
                "the message must name the shared prefix so the fix is obvious, got: {error}"
            );
        }

        let mut form = ChannelForm::for_create(&telegram_adapter());
        form.name = "slack-legal".to_string();
        form.fields[0].value = "token".to_string();
        assert!(
            form.to_request(&existing).is_ok(),
            "a name that does not collapse onto an existing one stays usable"
        );
    }

    /// The seeded name must satisfy the same checks the form applies to it, or
    /// the picker opens a form that refuses its own default with no obvious fix.
    #[test]
    fn the_seeded_name_clears_the_secret_prefix_check_too() {
        let existing = vec!["telegram".to_string(), "telegram_2".to_string()];
        let seeded = next_free_name("telegram", &existing);
        assert_eq!(
            seeded, "telegram-3",
            "`telegram-2` collapses onto the existing `telegram_2`, so it is not free"
        );

        let mut form = ChannelForm::for_create(&telegram_adapter());
        form.name = seeded;
        form.fields[0].value = "token".to_string();
        assert!(
            form.to_request(&existing).is_ok(),
            "the seed the picker offers must be one the form accepts"
        );
    }

    /// An instance hand-written into config.toml under an odd name predates
    /// this check and cannot be renamed here, so editing it must still work —
    /// refusing would make it uneditable as well as undeletable.
    #[test]
    fn an_existing_odd_name_is_still_editable() {
        let mut existing = instance("tele/gram", "telegram", Some("alice"));
        existing.fields[0].value = "token".to_string();
        let form = ChannelForm::for_edit(&existing);

        let request = form
            .to_request(&["tele/gram".to_string()])
            .expect("an existing entry is exempt from the charset check");
        assert_eq!(request.instance_name, "tele/gram");
    }

    /// Editing reuses the instance's own name, so uniqueness must not fire.
    #[test]
    fn edit_reuses_its_own_name_without_a_collision() {
        let existing = vec!["telegram".to_string(), "telegram-support".to_string()];
        let mut form = ChannelForm::for_edit(&instance("telegram-support", "telegram", None));
        form.fields[0].value = "fresh-token".to_string();

        let request = form.to_request(&existing).expect("edit must validate");
        assert_eq!(request.instance_name, "telegram-support");
        assert_eq!(request.adapter, "telegram");
        assert!(
            !form.name_editable(),
            "renaming would append a second block"
        );
    }

    #[test]
    fn edit_prefills_agent_and_leaves_the_name_row_unfocusable() {
        let mut state = ChannelState::new();
        state.set_instances(vec![instance(
            "telegram-support",
            "telegram",
            Some("carol"),
        )]);
        // Row 0 is the `telegram` group header, row 1 the instance.
        state.list_state.select(Some(1));

        state.handle_key(key(KeyCode::Char('e')));
        assert_eq!(state.sub, ChannelSubScreen::Form);
        assert_eq!(state.form.mode(), ChannelFormMode::Edit);
        assert_eq!(state.form.agent, "carol");
        assert_eq!(state.form.focus, FOCUS_AGENT);

        // Cycling backwards from the agent row must not land on the locked
        // name row, and typing must not be able to reach it either.
        state.handle_key(key(KeyCode::Up));
        assert_ne!(state.form.focus, FOCUS_NAME);
        state.form.focus = FOCUS_NAME;
        typed(&mut state, "zzz");
        assert_eq!(state.form.name, "telegram-support");
    }

    #[test]
    fn delete_is_keyed_by_instance_name_and_needs_confirmation() {
        let mut state = ChannelState::new();
        state.set_instances(vec![instance("telegram-support", "telegram", None)]);
        state.list_state.select(Some(1));

        let action = state.handle_key(key(KeyCode::Char('d')));
        assert!(matches!(action, ChannelAction::Continue));
        assert_eq!(state.sub, ChannelSubScreen::ConfirmDelete);
        assert_eq!(state.pending_delete.as_deref(), Some("telegram-support"));

        // Declining leaves everything alone.
        state.handle_key(key(KeyCode::Char('n')));
        assert_eq!(state.sub, ChannelSubScreen::List);
        assert!(state.pending_delete.is_none());

        state.handle_key(key(KeyCode::Char('d')));
        match state.handle_key(key(KeyCode::Char('y'))) {
            ChannelAction::DeleteInstance { instance_name } => {
                assert_eq!(
                    instance_name, "telegram-support",
                    "delete matches the [[sidecar_channels]] name, not the adapter"
                );
            }
            _ => panic!("expected a delete"),
        }
    }

    /// A header row is never a delete target — pressing `d` on one used to be
    /// the shape of bug that deletes the wrong thing.
    #[test]
    fn a_group_header_is_not_a_delete_target() {
        let mut state = ChannelState::new();
        state.set_instances(vec![instance("telegram", "telegram", None)]);
        state.list_state.select(Some(0));

        let action = state.handle_key(key(KeyCode::Char('d')));
        assert!(matches!(action, ChannelAction::Continue));
        assert_eq!(state.sub, ChannelSubScreen::List);
        assert!(state.pending_delete.is_none());
    }

    #[test]
    fn missing_required_field_blocks_the_save() {
        let form = ChannelForm::for_create(&telegram_adapter());
        let error = form.to_request(&[]).expect_err("the token is required");
        assert!(error.contains("TELEGRAM_BOT_TOKEN"), "got: {error}");
    }

    /// The daemon validates `required` against the payload and cannot read a
    /// stored secret back, so an edit that leaves it blank must be refused
    /// here with a message that says why, not sent and 400'd.
    #[test]
    fn a_stored_secret_must_be_retyped_to_save() {
        let mut existing = instance("telegram-support", "telegram", None);
        existing.fields[0].has_value = true;
        let form = ChannelForm::for_edit(&existing);

        let error = form
            .to_request(&["telegram-support".to_string()])
            .expect_err("a blank stored secret cannot be saved");
        assert!(error.contains("TELEGRAM_BOT_TOKEN"), "got: {error}");
        assert_ne!(
            error,
            crate::i18n::t_args(
                "tui-channels-error-field-required",
                &[("label", "TELEGRAM_BOT_TOKEN")]
            ),
            "a stored-but-unreadable secret gets its own explanation"
        );
    }

    #[test]
    fn instances_are_grouped_by_adapter_and_sorted_within_each_group() {
        let mut state = ChannelState::new();
        state.set_instances(vec![
            instance("slack-hr", "slack", None),
            instance("telegram-support", "telegram", None),
            instance("telegram", "telegram", None),
        ]);

        let shape: Vec<String> = state
            .rows
            .iter()
            .map(|row| match row {
                ChannelRow::Header { adapter, count } => format!("# {adapter} ({count})"),
                ChannelRow::Instance(i) => state.instances[*i].name.clone(),
                ChannelRow::AddNew => "+".to_string(),
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                "# slack (1)",
                "slack-hr",
                "# telegram (2)",
                "telegram",
                "telegram-support",
                "+",
            ]
        );
        // The cursor starts on the first instance, not on the header above it.
        assert_eq!(state.list_state.selected(), Some(1));
    }

    #[test]
    fn navigation_skips_group_headers_and_wraps() {
        let mut state = ChannelState::new();
        state.set_instances(vec![
            instance("slack-hr", "slack", None),
            instance("telegram", "telegram", None),
        ]);
        // slack header, slack-hr, telegram header, telegram, AddNew
        assert_eq!(state.list_state.selected(), Some(1));
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.list_state.selected(), Some(3));
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.list_state.selected(), Some(4));
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.list_state.selected(), Some(1), "wraps to the top");
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.list_state.selected(), Some(4), "wraps to the bottom");
    }

    #[test]
    fn enter_on_the_add_row_opens_the_adapter_picker() {
        let mut state = ChannelState::new();
        state.adapters = vec![telegram_adapter()];
        state.set_instances(Vec::new());
        assert_eq!(state.rows, vec![ChannelRow::AddNew]);

        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.sub, ChannelSubScreen::AdapterPicker);
        assert_eq!(state.adapter_list_state.selected(), Some(0));
    }

    #[test]
    fn bool_and_select_fields_cycle_instead_of_taking_text() {
        let mut adapter = telegram_adapter();
        adapter.fields = vec![
            ChannelFieldInfo {
                key: "CLEAR_DONE".to_string(),
                label: "Clear done".to_string(),
                field_type: "bool".to_string(),
                ..Default::default()
            },
            ChannelFieldInfo {
                key: "REGION".to_string(),
                label: "Region".to_string(),
                field_type: "select".to_string(),
                options: vec!["cn".to_string(), "intl".to_string()],
                value: "cn".to_string(),
                ..Default::default()
            },
        ];
        let mut state = ChannelState::new();
        state.form = ChannelForm::for_create(&adapter);
        state.sub = ChannelSubScreen::Form;

        state.form.focus = FIXED_FIELDS;
        state.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(state.form.fields[0].value, "true");
        state.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(state.form.fields[0].value, "false");

        state.form.focus = FIXED_FIELDS + 1;
        state.handle_key(key(KeyCode::Right));
        assert_eq!(state.form.fields[1].value, "intl");
        state.handle_key(key(KeyCode::Left));
        assert_eq!(state.form.fields[1].value, "cn");
    }

    #[test]
    fn a_select_field_with_no_default_lands_on_the_first_option_not_the_second() {
        let mut field = ChannelFieldInfo {
            key: "REGION".to_string(),
            label: "Region".to_string(),
            field_type: "select".to_string(),
            options: vec!["cn".to_string(), "intl".to_string()],
            value: String::new(),
            ..Default::default()
        };
        cycle_option(&mut field, true);
        assert_eq!(field.value, "cn");

        field.value = String::new();
        cycle_option(&mut field, false);
        assert_eq!(field.value, "intl");
    }

    #[test]
    fn an_empty_agent_clears_the_binding_rather_than_sending_a_blank() {
        let mut existing = instance("telegram", "telegram", Some("alice"));
        existing.fields[0].value = "token".to_string();
        let mut form = ChannelForm::for_edit(&existing);
        form.agent = "   ".to_string();

        let request = form.to_request(&["telegram".to_string()]).unwrap();
        assert_eq!(request.agent, None);
        assert_eq!(request.body()["agent"], serde_json::Value::Null);
    }

    #[test]
    fn health_reads_the_supervisor_facts_not_just_connected() {
        let mut instance = instance("telegram", "telegram", None);
        assert_eq!(instance.health(), InstanceHealth::Online);
        instance.last_error = Some("circuit break".to_string());
        assert_eq!(
            instance.health(),
            InstanceHealth::Degraded,
            "last_error is sticky, so it degrades rather than kills"
        );
        instance.connected = false;
        assert_eq!(instance.health(), InstanceHealth::Stopped);
        instance.supervised = false;
        assert_eq!(instance.health(), InstanceHealth::Unsupervised);
    }

    #[test]
    fn next_free_name_starts_at_two_and_skips_taken_names() {
        assert_eq!(next_free_name("telegram", &[]), "telegram-2");
        assert_eq!(
            next_free_name("telegram", &["telegram-2".to_string()]),
            "telegram-3"
        );
    }

    #[test]
    fn advanced_fields_sink_below_the_basic_ones() {
        let mut adapter = telegram_adapter();
        adapter.fields = vec![
            ChannelFieldInfo {
                key: "ALLOWED_USERS".to_string(),
                field_type: "list".to_string(),
                advanced: true,
                ..Default::default()
            },
            secret("TELEGRAM_BOT_TOKEN"),
            ChannelFieldInfo {
                key: "CLEAR_DONE".to_string(),
                field_type: "bool".to_string(),
                advanced: true,
                ..Default::default()
            },
        ];

        let form = ChannelForm::for_create(&adapter);
        let keys: Vec<&str> = form.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["TELEGRAM_BOT_TOKEN", "ALLOWED_USERS", "CLEAR_DONE"],
            "the required credential must come first, advanced knobs after"
        );
    }

    #[test]
    fn refresh_and_reload_are_distinct_actions() {
        let mut state = ChannelState::new();
        state.set_instances(Vec::new());
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('r'))),
            ChannelAction::Refresh
        ));
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('R'))),
            ChannelAction::ReloadChannels
        ));
    }
}
