//! The TUI's per-agent inference-parameter editor.
//!
//! Same seven knobs, same tri-state, and the same two step ladders as the
//! dashboard's agent editor — the TUI is not a reduced view of the WebUI here,
//! it can set everything the WebUI can.
//!
//! Two shapes of control, because the knobs are two different kinds of thing:
//!
//! * **Ladders** for the token counts. `context_window` and `max_tokens` are
//!   order-of-magnitude choices, so ← / → walk the presets in
//!   [`librefang_types::inference_params`] and `e` opens a field for the value
//!   that is not on the ladder. Stepping left off the first rung lands on
//!   *inherit*, which is a real position rather than a number.
//! * **Increments** for the sampling knobs. Temperature and the penalties are
//!   continuous, so ← / → nudge them by a sensible step within their range.
//!
//! The state here is deliberately free of any ratatui or HTTP dependency so
//! the stepping rules can be unit-tested directly.

use librefang_types::inference_params::{CONTEXT_WINDOW_LADDER, MAX_OUTPUT_TOKENS_LADDER};

/// One editable knob.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamField {
    Temperature,
    TopP,
    FrequencyPenalty,
    PresencePenalty,
    MaxTokens,
    ContextWindow,
    MaxOutputTokens,
}

/// Display order — preferences first, then the two endpoint limits, matching
/// the dashboard's grouping so the surfaces read the same way.
pub const FIELDS: &[ParamField] = &[
    ParamField::Temperature,
    ParamField::TopP,
    ParamField::FrequencyPenalty,
    ParamField::PresencePenalty,
    ParamField::MaxTokens,
    ParamField::ContextWindow,
    ParamField::MaxOutputTokens,
];

impl ParamField {
    /// The JSON key on `PATCH /api/agents/{id}/config`.
    pub fn key(self) -> &'static str {
        match self {
            ParamField::Temperature => "temperature",
            ParamField::TopP => "top_p",
            ParamField::FrequencyPenalty => "frequency_penalty",
            ParamField::PresencePenalty => "presence_penalty",
            ParamField::MaxTokens => "max_tokens",
            ParamField::ContextWindow => "context_window",
            ParamField::MaxOutputTokens => "max_output_tokens",
        }
    }

    /// Human-friendly label, localized.
    ///
    /// Per the TUI convention the variable name lives in the hint, not the
    /// label, so these read as prose and the `snake_case` identifier appears
    /// once, in [`Self::hint`].
    pub fn label(self) -> String {
        crate::i18n::t(match self {
            ParamField::Temperature => "tui-agents-param-temperature",
            ParamField::TopP => "tui-agents-param-top-p",
            ParamField::FrequencyPenalty => "tui-agents-param-frequency-penalty",
            ParamField::PresencePenalty => "tui-agents-param-presence-penalty",
            ParamField::MaxTokens => "tui-agents-param-max-tokens",
            ParamField::ContextWindow => "tui-agents-param-context-window",
            ParamField::MaxOutputTokens => "tui-agents-param-max-output-tokens",
        })
    }

    /// One-line explanation shown under the cursor, localized.
    pub fn hint(self) -> String {
        crate::i18n::t(match self {
            ParamField::Temperature => "tui-agents-param-temperature-hint",
            ParamField::TopP => "tui-agents-param-top-p-hint",
            ParamField::FrequencyPenalty => "tui-agents-param-frequency-penalty-hint",
            ParamField::PresencePenalty => "tui-agents-param-presence-penalty-hint",
            ParamField::MaxTokens => "tui-agents-param-max-tokens-hint",
            ParamField::ContextWindow => "tui-agents-param-context-window-hint",
            ParamField::MaxOutputTokens => "tui-agents-param-max-output-tokens-hint",
        })
    }

    /// Whether this knob steps along a preset ladder rather than by increments.
    pub fn is_ladder(self) -> bool {
        matches!(
            self,
            ParamField::MaxTokens | ParamField::ContextWindow | ParamField::MaxOutputTokens
        )
    }

    /// Increment applied by ← / → for a continuous knob.
    fn increment(self) -> f64 {
        match self {
            ParamField::Temperature | ParamField::TopP => 0.05,
            _ => 0.1,
        }
    }

    /// Inclusive range a continuous knob is clamped to.
    fn range(self) -> (f64, f64) {
        match self {
            ParamField::Temperature => (0.0, 2.0),
            ParamField::TopP => (0.0, 1.0),
            _ => (-2.0, 2.0),
        }
    }

    /// The preset ladder for a ladder knob, in tokens.
    fn ladder(self) -> Vec<u64> {
        match self {
            ParamField::ContextWindow => CONTEXT_WINDOW_LADDER.to_vec(),
            _ => MAX_OUTPUT_TOKENS_LADDER
                .iter()
                .copied()
                .map(u64::from)
                .collect(),
        }
    }
}

/// Render a token count the way operators read them: `128K`, `1M`, or the raw
/// number when it is not a clean multiple.
pub fn format_tokens(v: u64) -> String {
    if v >= 1 << 20 && v.is_multiple_of(1 << 20) {
        format!("{}M", v >> 20)
    } else if v >= 1 << 10 && v.is_multiple_of(1 << 10) {
        format!("{}K", v >> 10)
    } else {
        v.to_string()
    }
}

/// The editor's state for one agent.
pub struct ModelParamsEditor {
    /// Current value per [`FIELDS`] position. `None` is the inherit state, and
    /// it is a real position on the ladder rather than a missing value.
    values: Vec<Option<f64>>,
    /// The value each field held when the editor opened, so only genuine edits
    /// are sent.
    original: Vec<Option<f64>>,
    cursor: usize,
    /// `Some` while the operator is typing a value that is not on the ladder.
    custom: Option<String>,
    /// The model's known output cap, when there is one. Caps the output ladder
    /// so the editor never offers a rung this endpoint cannot honour. `None`
    /// when the limit was never sourced — an unknown limit is not a ceiling, so
    /// the full ladder stays available (#7780).
    output_cap: Option<u64>,
    /// Same, for the context ladder.
    context_cap: Option<u64>,
    pub status: String,
}

impl Default for ModelParamsEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelParamsEditor {
    pub fn new() -> Self {
        Self {
            values: vec![None; FIELDS.len()],
            original: vec![None; FIELDS.len()],
            cursor: 0,
            custom: None,
            output_cap: None,
            context_cap: None,
            status: String::new(),
        }
    }

    /// Load the agent's current values, as returned by `GET /api/agents/{id}`.
    /// A `null` in that payload is the inherit state and stays `None` here.
    pub fn load(&mut self, model: &serde_json::Value) {
        for (i, f) in FIELDS.iter().enumerate() {
            self.values[i] = model.get(f.key()).and_then(serde_json::Value::as_f64);
        }
        self.original.clone_from(&self.values);
        self.cursor = 0;
        self.custom = None;
        self.status.clear();
    }

    /// Record the model's own limits so the ladders stop where the endpoint does.
    pub fn set_caps(&mut self, context: Option<u64>, output: Option<u64>) {
        self.context_cap = context;
        self.output_cap = output;
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn custom_buffer(&self) -> Option<&str> {
        self.custom.as_deref()
    }

    pub fn value(&self, index: usize) -> Option<f64> {
        self.values.get(index).copied().flatten()
    }

    /// The value as it should appear on screen — `inherit` when the agent has
    /// no opinion.
    pub fn display(&self, index: usize) -> String {
        match self.values[index] {
            None => "inherit".to_string(),
            Some(v) if FIELDS[index].is_ladder() => format_tokens(v.max(0.0) as u64),
            Some(v) => format!("{v:.2}"),
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.custom.is_some() {
            return;
        }
        let len = FIELDS.len() as isize;
        self.cursor = ((self.cursor as isize + delta).rem_euclid(len)) as usize;
    }

    /// Hand the field under the cursor back to inherit.
    pub fn set_inherit(&mut self) {
        self.values[self.cursor] = None;
    }

    /// Walk one position along the field's ladder (or one increment along its
    /// range).
    ///
    /// Stepping down off the first ladder rung lands on inherit rather than
    /// stopping at the smallest number — "no opinion" has to be reachable with
    /// the same key that reaches every other position.
    pub fn step(&mut self, delta: i32) {
        let field = FIELDS[self.cursor];
        if field.is_ladder() {
            let rungs = self.visible_ladder(field);
            let current = self.values[self.cursor].map(|v| v.max(0.0) as u64);
            let next = step_ladder(&rungs, current, delta);
            self.values[self.cursor] = next.map(|v| v as f64);
        } else {
            let (lo, hi) = field.range();
            let base = self.values[self.cursor].unwrap_or(0.0);
            if delta < 0 && self.values[self.cursor].is_none() {
                return; // already at "no opinion"; nothing below it
            }
            let stepped = base + field.increment() * f64::from(delta);
            // Round to the increment's precision so repeated steps don't drift
            // into 0.30000000000000004.
            let stepped = (stepped * 100.0).round() / 100.0;
            self.values[self.cursor] = Some(stepped.clamp(lo, hi));
        }
    }

    /// The ladder rungs this field can currently offer, trimmed to the model's
    /// known cap. An unknown cap trims nothing.
    fn visible_ladder(&self, field: ParamField) -> Vec<u64> {
        let cap = match field {
            ParamField::ContextWindow => self.context_cap,
            _ => self.output_cap,
        };
        let mut rungs = field.ladder();
        if let Some(cap) = cap {
            rungs.retain(|r| *r <= cap);
            // Keep the cap itself reachable even when it sits between rungs.
            if !rungs.contains(&cap) {
                rungs.push(cap);
                rungs.sort_unstable();
            }
        }
        rungs
    }

    /// Start typing a value that is not on the ladder.
    pub fn begin_custom(&mut self) {
        self.custom = Some(
            self.values[self.cursor]
                .map(|v| {
                    if FIELDS[self.cursor].is_ladder() {
                        format!("{}", v.max(0.0) as u64)
                    } else {
                        format!("{v}")
                    }
                })
                .unwrap_or_default(),
        );
    }

    pub fn cancel_custom(&mut self) {
        self.custom = None;
    }

    pub fn push_custom_char(&mut self, c: char) {
        if let Some(buf) = self.custom.as_mut() {
            if c.is_ascii_digit() || c == '.' || c == '-' {
                buf.push(c);
            }
        }
    }

    pub fn pop_custom_char(&mut self) {
        if let Some(buf) = self.custom.as_mut() {
            buf.pop();
        }
    }

    /// Commit the typed value. An empty buffer means inherit — the same
    /// "no opinion" the CLI spells `inherit`.
    ///
    /// Returns an error message when the text is not a number for this field;
    /// the buffer stays open so the operator can correct it.
    pub fn commit_custom(&mut self) -> Result<(), String> {
        let Some(buf) = self.custom.clone() else {
            return Ok(());
        };
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            self.values[self.cursor] = None;
            self.custom = None;
            return Ok(());
        }
        let field = FIELDS[self.cursor];
        let parsed: f64 = trimmed.parse().map_err(|_| {
            crate::i18n::t_args(
                "tui-agents-param-not-a-number",
                &[("field", &field.label())],
            )
        })?;
        if field.is_ladder() {
            if parsed < 1.0 || parsed.fract() != 0.0 {
                return Err(crate::i18n::t_args(
                    "tui-agents-param-not-whole",
                    &[("field", &field.label())],
                ));
            }
        } else {
            let (lo, hi) = field.range();
            if !(lo..=hi).contains(&parsed) {
                return Err(crate::i18n::t_args(
                    "tui-agents-param-out-of-range",
                    &[
                        ("field", &field.label()),
                        ("min", &lo.to_string()),
                        ("max", &hi.to_string()),
                    ],
                ));
            }
        }
        self.values[self.cursor] = Some(parsed);
        self.custom = None;
        Ok(())
    }

    /// The edits to send, as `(json key, value)` pairs. A `None` value is a
    /// JSON `null`, which the endpoint reads as "hand this back to inherit".
    ///
    /// Only genuinely changed fields are included, so opening the editor and
    /// leaving without touching anything sends nothing.
    pub fn changes(&self) -> Vec<(&'static str, Option<f64>)> {
        FIELDS
            .iter()
            .enumerate()
            .filter(|(i, _)| self.values[*i] != self.original[*i])
            .map(|(i, f)| (f.key(), self.values[i]))
            .collect()
    }
}

/// Move `current` one rung along `rungs`.
///
/// `None` is the position below the first rung. Stepping up from it lands on
/// the smallest rung; stepping down from the smallest rung returns to it. A
/// value that is not on the ladder (typed by hand) steps to the nearest rung in
/// the requested direction rather than snapping to an arbitrary index, so a
/// custom 50000 followed by → lands on the next rung above it.
fn step_ladder(rungs: &[u64], current: Option<u64>, delta: i32) -> Option<u64> {
    if rungs.is_empty() {
        return current;
    }
    match (current, delta.signum()) {
        (_, 0) => current,
        (None, 1) => Some(rungs[0]),
        (None, _) => None,
        (Some(v), 1) => Some(rungs.iter().copied().find(|r| *r > v).unwrap_or(v)),
        (Some(v), _) => rungs.iter().rev().copied().find(|r| *r < v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> ModelParamsEditor {
        ModelParamsEditor::new()
    }

    fn index_of(field: ParamField) -> usize {
        FIELDS.iter().position(|f| *f == field).expect("field")
    }

    fn focus(ed: &mut ModelParamsEditor, field: ParamField) {
        ed.cursor = index_of(field);
    }

    #[test]
    fn a_fresh_editor_shows_every_knob_as_inherit() {
        let ed = editor();
        for i in 0..FIELDS.len() {
            assert_eq!(ed.display(i), "inherit");
        }
        assert!(
            ed.changes().is_empty(),
            "opening the editor changes nothing"
        );
    }

    #[test]
    fn stepping_walks_the_ladder_and_returns_to_inherit_below_it() {
        let mut ed = editor();
        focus(&mut ed, ParamField::MaxTokens);

        ed.step(1);
        assert_eq!(ed.display(index_of(ParamField::MaxTokens)), "1K");
        ed.step(1);
        assert_eq!(ed.display(index_of(ParamField::MaxTokens)), "4K");
        ed.step(-1);
        assert_eq!(ed.display(index_of(ParamField::MaxTokens)), "1K");
        ed.step(-1);
        assert_eq!(
            ed.display(index_of(ParamField::MaxTokens)),
            "inherit",
            "below the first rung is 'no opinion', not the smallest number"
        );
    }

    #[test]
    fn the_ladder_stops_at_its_top_rung() {
        let mut ed = editor();
        focus(&mut ed, ParamField::ContextWindow);
        for _ in 0..20 {
            ed.step(1);
        }
        assert_eq!(ed.display(index_of(ParamField::ContextWindow)), "2M");
    }

    /// A known cap trims the ladder — the editor never offers a rung the
    /// endpoint cannot honour — and the cap itself stays reachable even when it
    /// sits between two rungs.
    #[test]
    fn a_known_cap_trims_the_ladder_and_stays_reachable() {
        let mut ed = editor();
        ed.set_caps(None, Some(20_000));
        focus(&mut ed, ParamField::MaxTokens);
        for _ in 0..20 {
            ed.step(1);
        }
        assert_eq!(ed.value(index_of(ParamField::MaxTokens)), Some(20_000.0));
    }

    /// An unknown cap trims nothing. A limit nobody measured is not a ceiling,
    /// so the operator keeps the whole ladder (#7780).
    #[test]
    fn an_unknown_cap_leaves_the_whole_ladder_available() {
        let mut ed = editor();
        ed.set_caps(None, None);
        focus(&mut ed, ParamField::MaxTokens);
        for _ in 0..20 {
            ed.step(1);
        }
        assert_eq!(ed.display(index_of(ParamField::MaxTokens)), "128K");
    }

    #[test]
    fn a_custom_value_steps_to_the_neighbouring_rung() {
        let mut ed = editor();
        focus(&mut ed, ParamField::MaxTokens);
        ed.begin_custom();
        for c in "50000".chars() {
            ed.push_custom_char(c);
        }
        ed.commit_custom().expect("50000 is a valid custom value");
        assert_eq!(ed.value(index_of(ParamField::MaxTokens)), Some(50_000.0));

        ed.step(1);
        assert_eq!(
            ed.value(index_of(ParamField::MaxTokens)),
            Some(65_536.0),
            "→ from an off-ladder value lands on the next rung above it"
        );
    }

    #[test]
    fn an_empty_custom_buffer_means_inherit() {
        let mut ed = editor();
        focus(&mut ed, ParamField::Temperature);
        ed.step(1);
        assert_ne!(ed.display(index_of(ParamField::Temperature)), "inherit");

        ed.begin_custom();
        while ed.custom_buffer().is_some_and(|b| !b.is_empty()) {
            ed.pop_custom_char();
        }
        ed.commit_custom().expect("an empty buffer is valid");
        assert_eq!(ed.display(index_of(ParamField::Temperature)), "inherit");
    }

    #[test]
    fn a_custom_value_outside_the_range_is_rejected_and_the_buffer_stays_open() {
        let mut ed = editor();
        focus(&mut ed, ParamField::Temperature);
        ed.begin_custom();
        for c in "3.5".chars() {
            ed.push_custom_char(c);
        }
        assert!(ed.commit_custom().is_err());
        assert!(
            ed.custom_buffer().is_some(),
            "a rejected value must leave the operator's text in place to fix"
        );
    }

    #[test]
    fn continuous_knobs_step_by_increments_within_their_range() {
        let mut ed = editor();
        focus(&mut ed, ParamField::Temperature);
        ed.step(1);
        assert_eq!(ed.display(index_of(ParamField::Temperature)), "0.05");
        for _ in 0..100 {
            ed.step(1);
        }
        assert_eq!(
            ed.display(index_of(ParamField::Temperature)),
            "2.00",
            "temperature is clamped at its documented maximum"
        );
    }

    #[test]
    fn only_edited_fields_are_sent_and_a_cleared_field_sends_null() {
        let mut ed = editor();
        ed.load(&serde_json::json!({
            "temperature": 0.7,
            "max_tokens": 4096,
            "top_p": serde_json::Value::Null,
        }));
        assert!(ed.changes().is_empty(), "loading is not editing");

        focus(&mut ed, ParamField::Temperature);
        ed.set_inherit();
        focus(&mut ed, ParamField::TopP);
        ed.step(1);

        let changes = ed.changes();
        assert_eq!(changes.len(), 2, "untouched fields stay out of the payload");
        assert!(changes.contains(&("temperature", None)));
        assert!(changes.contains(&("top_p", Some(0.05))));
    }

    #[test]
    fn token_counts_are_formatted_the_way_operators_read_them() {
        assert_eq!(format_tokens(8_192), "8K");
        assert_eq!(format_tokens(131_072), "128K");
        assert_eq!(format_tokens(1_048_576), "1M");
        assert_eq!(format_tokens(2_097_152), "2M");
        assert_eq!(format_tokens(50_000), "50000");
    }
}
