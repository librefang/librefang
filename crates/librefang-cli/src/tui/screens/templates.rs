//! Templates screen: browse agent templates and spawn with one click.

use crate::tui::{theme, widgets};
use librefang_types::agent::ToolProfile;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

/// Where a row came from, and therefore how it is spawned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TemplateSource {
    /// A compiled-in starter template.
    /// It has no `agent.toml` anywhere, so [`builtin_profile`] is its capability declaration.
    Builtin,
    /// An operator-created agent type backed by a real `agent.toml`.
    /// Spawning fetches that file verbatim rather than reconstructing it.
    Manifest,
}

#[derive(Clone)]
pub struct TemplateInfo {
    pub name: String,
    pub description: String,
    pub category: String,
    pub provider: String,
    pub model: String,
    pub source: TemplateSource,
}

#[derive(Clone)]
pub struct ProviderAuth {
    pub name: String,
    pub configured: bool,
}

// ── Built-in templates ──────────────────────────────────────────────────────

// NOTE: These static template names are used to dynamically derive i18n keys (e.g. tui-templates-name-*).
// Do not rename or edit these strings without updating the corresponding Fluent translation keys.
const BUILTIN_TEMPLATES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "General Assistant",
        "Versatile AI assistant for everyday tasks",
        "General",
        "default",
        "default",
    ),
    (
        "Code Helper",
        "Programming assistant with code review and debugging",
        "Development",
        "default",
        "default",
    ),
    (
        "Researcher",
        "Deep research and analysis with web search",
        "Research",
        "default",
        "default",
    ),
    (
        "Writer",
        "Creative and technical writing assistant",
        "Writing",
        "default",
        "default",
    ),
    (
        "Data Analyst",
        "Data analysis, visualization, and SQL queries",
        "Development",
        "default",
        "default",
    ),
    (
        "DevOps Engineer",
        "Infrastructure, CI/CD, and deployment assistance",
        "Development",
        "default",
        "default",
    ),
    (
        "Customer Support",
        "Professional customer service agent",
        "Business",
        "default",
        "default",
    ),
    (
        "Tutor",
        "Patient educational assistant for learning any subject",
        "General",
        "default",
        "default",
    ),
    (
        "API Designer",
        "REST/GraphQL API design and documentation",
        "Development",
        "default",
        "default",
    ),
    (
        "Meeting Notes",
        "Meeting transcription, summary, and action items",
        "Business",
        "default",
        "default",
    ),
];

// ── Categories ──────────────────────────────────────────────────────────────

/// Category assigned to every manifest-backed agent type.
///
/// Operator tags are free-form, so the screen does not guess which of the fixed builtin categories a manifest belongs to — it gets its own bucket.
pub const MANIFEST_CATEGORY: &str = "Custom";

const CATEGORIES: &[&str] = &[
    "All",
    "General",
    "Development",
    "Research",
    "Writing",
    "Business",
    MANIFEST_CATEGORY,
];

/// The tool profile each builtin declares.
///
/// Builtins have no manifest on disk, so this table *is* their capability declaration.
/// It exists because the spawn path used to invent one: every row, whatever it was for, was spawned with shell execution plus filesystem write plus network access (#7760).
/// Profiles are assigned per template and none of them is [`ToolProfile::Full`].
pub fn builtin_profile(name: &str) -> ToolProfile {
    match name {
        // Shell is warranted: these run code, queries and infrastructure.
        "Code Helper" | "Data Analyst" | "API Designer" => ToolProfile::Coding,
        "DevOps Engineer" => ToolProfile::Automation,
        // Web plus file read/write, no shell.
        "General Assistant" | "Researcher" | "Writer" | "Tutor" => ToolProfile::Research,
        // Messaging and memory, no shell and no filesystem.
        "Customer Support" | "Meeting Notes" => ToolProfile::Messaging,
        // An unrecognised name must fail closed, never open.
        _ => ToolProfile::Minimal,
    }
}

/// Render the manifest for a builtin row.
///
/// Capabilities are expressed as the row's named [`ToolProfile`] and expanded kernel-side by `manifest_to_capabilities`.
/// Nothing here writes a tool list, which is the whole point: the previous version pinned `tools = ["shell", "file_read", "file_write", "web_fetch", "web_search"]` onto every template regardless of the template (#7760).
/// Values are emitted through `toml::Value` so a name or description can never break out of its string literal.
pub fn builtin_manifest_toml(t: &TemplateInfo) -> String {
    let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
    let profile = builtin_profile(&t.name);
    let profile_name = toml::Value::try_from(&profile)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "minimal".to_string());
    format!(
        r#"name = {name}
description = {description}
profile = {profile}

[model]
provider = {provider}
model = {model}
"#,
        name = quote(&t.name),
        description = quote(&t.description),
        profile = quote(&profile_name),
        provider = quote(&t.provider),
        model = quote(&t.model),
    )
}

fn builtin_templates() -> Vec<TemplateInfo> {
    BUILTIN_TEMPLATES
        .iter()
        .map(|(name, desc, cat, prov, model)| TemplateInfo {
            name: name.to_string(),
            description: desc.to_string(),
            category: cat.to_string(),
            provider: prov.to_string(),
            model: model.to_string(),
            source: TemplateSource::Builtin,
        })
        .collect()
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct TemplatesState {
    pub templates: Vec<TemplateInfo>,
    pub providers: Vec<ProviderAuth>,
    pub category_filter: usize,
    pub filtered: Vec<usize>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum TemplatesAction {
    Continue,
    Refresh,
    SpawnTemplate {
        name: String,
        source: TemplateSource,
    },
}

impl TemplatesState {
    pub fn new() -> Self {
        let templates = builtin_templates();
        let filtered: Vec<usize> = (0..templates.len()).collect();
        let mut state = Self {
            templates,
            providers: Vec::new(),
            category_filter: 0,
            filtered,
            list_state: ListState::default(),
            loading: false,
            tick: 0,
            status_msg: String::new(),
        };
        state.list_state.select(Some(0));
        state
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Replace the manifest-backed rows, keeping the compiled-in builtins.
    ///
    /// A manifest whose name matches a builtin replaces it: an operator who names a directory after a builtin is overriding it, not duplicating it.
    pub fn set_manifest_templates(&mut self, mut incoming: Vec<TemplateInfo>) {
        incoming.sort_by(|a, b| a.name.cmp(&b.name));
        let mut merged: Vec<TemplateInfo> = builtin_templates()
            .into_iter()
            .filter(|b| !incoming.iter().any(|i| i.name == b.name))
            .collect();
        merged.extend(incoming);
        self.templates = merged;
        self.refilter();
    }

    fn refilter(&mut self) {
        let cat = CATEGORIES[self.category_filter];
        if cat == "All" {
            self.filtered = (0..self.templates.len()).collect();
        } else {
            self.filtered = self
                .templates
                .iter()
                .enumerate()
                .filter(|(_, t)| t.category == cat)
                .map(|(i, _)| i)
                .collect();
        }
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    fn provider_configured(&self, provider: &str) -> bool {
        // "default" is not a provider id — it means "inherit whatever the daemon has configured".
        // `/api/providers` never returns it, so without this arm every builtin row is gated off the moment providers load.
        if provider == "default" {
            return true;
        }
        self.providers
            .iter()
            .any(|p| p.name == provider && p.configured)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> TemplatesAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return TemplatesAction::Continue;
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
                        let t = &self.templates[idx];
                        if !self.provider_configured(&t.provider) && !self.providers.is_empty() {
                            self.status_msg = crate::i18n::t_args(
                                "tui-templates-provider-not-configured",
                                &[("provider", &t.provider)],
                            );
                            return TemplatesAction::Continue;
                        }
                        return TemplatesAction::SpawnTemplate {
                            name: t.name.clone(),
                            source: t.source,
                        };
                    }
                }
            }
            KeyCode::Char('f') => {
                self.category_filter = (self.category_filter + 1) % CATEGORIES.len();
                self.refilter();
            }
            KeyCode::Char('r') => return TemplatesAction::Refresh,
            _ => {}
        }
        TemplatesAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut TemplatesState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("{} {}", "\u{25a2}", crate::i18n::t("tui-templates-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(2), // header + category filter
        Constraint::Min(3),    // list
        Constraint::Length(3), // detail preview
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Category filter + header ──
    let active_cat = CATEGORIES[state.category_filter];
    let mut cat_spans: Vec<Span> = vec![Span::raw("  ")];
    for (i, &c) in CATEGORIES.iter().enumerate() {
        if i > 0 {
            cat_spans.push(Span::styled(
                " \u{2502} ",
                Style::default().fg(theme::BORDER),
            ));
        }
        let localized_cat = match c {
            "All" => crate::i18n::t("tui-templates-cat-all"),
            "General" => crate::i18n::t("tui-templates-cat-general"),
            "Development" => crate::i18n::t("tui-templates-cat-development"),
            "Research" => crate::i18n::t("tui-templates-cat-research"),
            "Writing" => crate::i18n::t("tui-templates-cat-writing"),
            "Business" => crate::i18n::t("tui-templates-cat-business"),
            MANIFEST_CATEGORY => crate::i18n::t("tui-templates-cat-custom"),
            other => other.to_string(),
        };
        if c == active_cat {
            cat_spans.push(Span::styled(
                format!(" {} {} ", "\u{25cf}", localized_cat),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            cat_spans.push(Span::styled(
                format!(" {} {} ", "\u{25cb}", localized_cat),
                theme::dim_style(),
            ));
        }
    }
    f.render_widget(
        Paragraph::new(vec![
            Line::from(cat_spans),
            Line::from(vec![Span::styled(
                format!(
                    "  {:<22} {:<14} {:<16} {}",
                    crate::i18n::t("tui-templates-header-template"),
                    crate::i18n::t("tui-templates-header-category"),
                    crate::i18n::t("tui-templates-header-provider-model"),
                    crate::i18n::t("tui-templates-header-description")
                ),
                theme::table_header(),
            )]),
        ]),
        chunks[0],
    );

    // ── List ──
    if state.loading {
        f.render_widget(
            widgets::spinner(state.tick, &crate::i18n::t("tui-templates-loading")),
            chunks[1],
        );
    } else if state.filtered.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-templates-empty")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .filtered
            .iter()
            .map(|&idx| {
                let t = &state.templates[idx];
                let configured = state.provider_configured(&t.provider);
                let auth_badge = if state.providers.is_empty() {
                    Span::raw("")
                } else if configured {
                    Span::styled(" \u{25cf}", Style::default().fg(theme::GREEN))
                } else {
                    Span::styled(" \u{25cb}", Style::default().fg(theme::RED))
                };
                let prov_model = format!("{}/{}", t.provider, widgets::truncate(&t.model, 12));
                let localized_name = localize_template_name(&t.name);
                let localized_desc = localize_template_desc(&t.name, &t.description);
                let localized_cat = match t.category.as_str() {
                    "General" => crate::i18n::t("tui-templates-cat-general"),
                    "Development" => crate::i18n::t("tui-templates-cat-development"),
                    "Research" => crate::i18n::t("tui-templates-cat-research"),
                    "Writing" => crate::i18n::t("tui-templates-cat-writing"),
                    "Business" => crate::i18n::t("tui-templates-cat-business"),
                    MANIFEST_CATEGORY => crate::i18n::t("tui-templates-cat-custom"),
                    other => other.to_string(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<22}", widgets::truncate(&localized_name, 21)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(" {:<14}", localized_cat),
                        Style::default().fg(theme::YELLOW),
                    ),
                    Span::styled(
                        format!(" {:<16}", widgets::truncate(&prov_model, 15)),
                        Style::default().fg(theme::BLUE),
                    ),
                    auth_badge,
                    Span::styled(
                        format!("  {}", widgets::truncate(&localized_desc, 28)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = widgets::themed_list(items);
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    // ── Detail preview ──
    if let Some(sel) = state.list_state.selected() {
        if let Some(&idx) = state.filtered.get(sel) {
            let t = &state.templates[idx];
            let localized_name = localize_template_name(&t.name);
            let localized_desc = localize_template_desc(&t.name, &t.description);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![Span::styled(
                        format!("  {} ", localized_name),
                        Style::default()
                            .fg(theme::CYAN)
                            .add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(&localized_desc, theme::dim_style()),
                    ]),
                    Line::from(vec![Span::styled(
                        crate::i18n::t_args(
                            "tui-templates-detail-provider",
                            &[("provider", &t.provider), ("model", &t.model)],
                        ),
                        Style::default().fg(theme::BLUE),
                    )]),
                ]),
                chunks[2],
            );
        }
    }

    // ── Hints / status ──
    if !state.status_msg.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::YELLOW),
            )])),
            chunks[3],
        );
    } else {
        f.render_widget(
            widgets::hint_bar(&crate::i18n::t("tui-templates-hints")),
            chunks[3],
        );
    }
}

/// `i18n::t` renders an unknown key as `[key]`, so a bare `rendered == key` comparison never fires.
/// Manifest-backed agent types have no Fluent key at all, which is how their names would otherwise render as `[tui-templates-name-…]`.
fn is_untranslated(rendered: &str, key: &str) -> bool {
    rendered == key || rendered == format!("[{key}]")
}

fn localize_template_name(name: &str) -> String {
    let key = format!(
        "tui-templates-name-{}",
        name.to_lowercase().replace(' ', "-")
    );
    let localized = crate::i18n::t(&key);
    if is_untranslated(&localized, &key) {
        name.to_string()
    } else {
        localized
    }
}

fn localize_template_desc(name: &str, default_desc: &str) -> String {
    let key = format!(
        "tui-templates-desc-{}",
        name.to_lowercase().replace(' ', "-")
    );
    let localized = crate::i18n::t(&key);
    if is_untranslated(&localized, &key) {
        default_desc.to_string()
    } else {
        localized
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::agent::AgentManifest;

    /// Every tool the previous fabricated spawn string granted unconditionally.
    /// `shell` was not even a real tool id — the real one is `shell_exec` — so the list was both over-broad and partly inert.
    const FABRICATED_TOOLS: &[&str] = &[
        "shell",
        "file_read",
        "file_write",
        "web_fetch",
        "web_search",
    ];

    fn builtin(name: &str) -> TemplateInfo {
        builtin_templates()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no builtin named {name}"))
    }

    #[test]
    fn builtin_manifest_declares_a_profile_and_no_inline_tool_list() {
        for t in builtin_templates() {
            let rendered = builtin_manifest_toml(&t);
            assert!(
                !rendered.contains("[capabilities]"),
                "{} must not carry an inline capability block; capabilities come \
                 from its declared profile: {rendered}",
                t.name
            );
            assert!(
                !rendered.contains("tools ="),
                "{} must not spell out a tool list: {rendered}",
                t.name
            );
            let manifest: AgentManifest =
                toml::from_str(&rendered).unwrap_or_else(|e| panic!("{}: {e}\n{rendered}", t.name));
            assert!(
                manifest.capabilities.tools.is_empty(),
                "{} parsed with an explicit tool list, which would shadow its \
                 profile: {:?}",
                t.name,
                manifest.capabilities.tools
            );
            let profile = manifest
                .profile
                .unwrap_or_else(|| panic!("{} declares no profile", t.name));
            assert_eq!(
                profile,
                builtin_profile(&t.name),
                "{} must spawn with the profile its table declares",
                t.name
            );
        }
    }

    #[test]
    fn no_builtin_spawns_with_the_fabricated_tool_superset() {
        for t in builtin_templates() {
            let manifest: AgentManifest = toml::from_str(&builtin_manifest_toml(&t)).unwrap();
            let tools = manifest.profile.unwrap().tools();
            assert!(
                !tools.iter().any(|tool| tool == "*"),
                "{} must not spawn with wildcard tool access: {tools:?}",
                t.name
            );
            // The fabricated string granted all five to everything.
            // A template may legitimately declare some of them; it must never declare the whole set, which is what "no capability inflation" means here.
            let granted = FABRICATED_TOOLS
                .iter()
                .filter(|w| tools.iter().any(|tool| tool == *w))
                .count();
            assert!(
                granted < FABRICATED_TOOLS.len(),
                "{} still receives the whole fabricated superset: {tools:?}",
                t.name
            );
        }
    }

    #[test]
    fn writing_and_messaging_builtins_get_no_shell() {
        for name in ["Writer", "Tutor", "Customer Support", "Meeting Notes"] {
            let manifest: AgentManifest =
                toml::from_str(&builtin_manifest_toml(&builtin(name))).unwrap();
            let tools = manifest.profile.unwrap().tools();
            assert!(
                !tools.iter().any(|t| t == "shell_exec" || t == "*"),
                "{name} must not spawn with shell execution: {tools:?}"
            );
        }
    }

    #[test]
    fn manifest_backed_rows_round_trip_the_served_toml_verbatim() {
        // The manifest path must never reconstruct anything.
        // Pin the contract that what the API serves is what gets spawned, byte for byte.
        let served = "name = \"payroll\"\ndescription = \"reads ledgers\"\n\n\
                      [model]\nprovider = \"anthropic\"\nmodel = \"claude-x\"\n\n\
                      [capabilities]\ntools = [\"file_read\"]\n";
        let manifest: AgentManifest = toml::from_str(served).unwrap();
        assert_eq!(manifest.capabilities.tools, vec!["file_read".to_string()]);
        assert_eq!(manifest.model.provider, "anthropic");
        assert!(
            manifest.profile.is_none(),
            "a served manifest must keep its own shape, profile included"
        );
    }

    #[test]
    fn manifest_templates_override_a_same_named_builtin_and_keep_the_rest() {
        let mut state = TemplatesState::new();
        let builtin_count = builtin_templates().len();
        state.set_manifest_templates(vec![
            TemplateInfo {
                name: "Writer".to_string(),
                description: "operator override".to_string(),
                category: MANIFEST_CATEGORY.to_string(),
                provider: "anthropic".to_string(),
                model: "claude-x".to_string(),
                source: TemplateSource::Manifest,
            },
            TemplateInfo {
                name: "payroll".to_string(),
                description: "operator type".to_string(),
                category: MANIFEST_CATEGORY.to_string(),
                provider: "openai".to_string(),
                model: "gpt-x".to_string(),
                source: TemplateSource::Manifest,
            },
        ]);
        assert_eq!(state.templates.len(), builtin_count + 1);
        let writer = state
            .templates
            .iter()
            .find(|t| t.name == "Writer")
            .expect("Writer row");
        assert_eq!(writer.source, TemplateSource::Manifest);
        assert_eq!(writer.provider, "anthropic");
        assert!(state.templates.iter().any(|t| t.name == "payroll"));
        assert!(state.templates.iter().any(|t| t.name == "Code Helper"));
    }

    #[test]
    fn manifest_rows_are_reachable_through_the_custom_category_filter() {
        let mut state = TemplatesState::new();
        state.set_manifest_templates(vec![TemplateInfo {
            name: "payroll".to_string(),
            description: "operator type".to_string(),
            category: MANIFEST_CATEGORY.to_string(),
            provider: "openai".to_string(),
            model: "gpt-x".to_string(),
            source: TemplateSource::Manifest,
        }]);
        let custom = CATEGORIES
            .iter()
            .position(|c| *c == MANIFEST_CATEGORY)
            .expect("Custom category must be selectable");
        state.category_filter = custom;
        state.refilter();
        let names: Vec<&str> = state
            .filtered
            .iter()
            .map(|&i| state.templates[i].name.as_str())
            .collect();
        assert_eq!(names, vec!["payroll"]);
    }

    #[test]
    fn operator_names_render_as_themselves_when_no_translation_exists() {
        assert_eq!(localize_template_name("payroll"), "payroll");
        assert_eq!(
            localize_template_desc("payroll", "operator type"),
            "operator type"
        );
    }

    #[test]
    fn default_provider_is_not_gated_as_unconfigured() {
        let mut state = TemplatesState::new();
        state.providers = vec![ProviderAuth {
            name: "anthropic".to_string(),
            configured: true,
        }];
        assert!(
            state.provider_configured("default"),
            "\"default\" means \"inherit the daemon's provider\", not a provider id"
        );
        assert!(!state.provider_configured("openai"));
    }
}
