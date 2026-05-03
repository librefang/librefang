//! Standalone ratatui init wizard: 6-step onboarding flow.
//!
//! Launched by `librefang init` (without `--quick`). Takes over the terminal,
//! runs its own event loop, and returns an `InitResult`.

use ratatui::crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::tui::theme;
use crate::tui::widgets;
use librefang_extensions::dotenv;
use librefang_runtime::model_catalog::ModelCatalog;
use librefang_types::model_catalog::ModelTier;

const INIT_WIZARD_CONFIG_TEMPLATE: &str =
    include_str!("../../../templates/init_wizard_config.toml");

// ── Provider metadata ──────────────────────────────────────────────────────

struct ProviderInfo {
    name: &'static str,
    display: &'static str,
    env_var: &'static str,
    needs_key: bool,
    hint: &'static str,
}

const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        name: "groq",
        display: "Groq",
        env_var: "GROQ_API_KEY",
        needs_key: true,
        hint: "free tier",
    },
    ProviderInfo {
        name: "gemini",
        display: "Gemini",
        env_var: "GEMINI_API_KEY",
        needs_key: true,
        hint: "free tier",
    },
    ProviderInfo {
        name: "deepseek",
        display: "DeepSeek",
        env_var: "DEEPSEEK_API_KEY",
        needs_key: true,
        hint: "cheap",
    },
    ProviderInfo {
        name: "anthropic",
        display: "Anthropic",
        env_var: "ANTHROPIC_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "openai",
        display: "OpenAI",
        env_var: "OPENAI_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "openrouter",
        display: "OpenRouter",
        env_var: "OPENROUTER_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "together",
        display: "Together",
        env_var: "TOGETHER_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "mistral",
        display: "Mistral",
        env_var: "MISTRAL_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "fireworks",
        display: "Fireworks",
        env_var: "FIREWORKS_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "xai",
        display: "xAI (Grok)",
        env_var: "XAI_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "perplexity",
        display: "Perplexity",
        env_var: "PERPLEXITY_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "cohere",
        display: "Cohere",
        env_var: "COHERE_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "cerebras",
        display: "Cerebras",
        env_var: "CEREBRAS_API_KEY",
        needs_key: true,
        hint: "fast inference",
    },
    ProviderInfo {
        name: "sambanova",
        display: "SambaNova",
        env_var: "SAMBANOVA_API_KEY",
        needs_key: true,
        hint: "fast inference",
    },
    ProviderInfo {
        name: "qwen",
        display: "Qwen (Alibaba)",
        env_var: "QWEN_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "huggingface",
        display: "Hugging Face",
        env_var: "HUGGINGFACE_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "github-copilot",
        display: "GitHub Copilot",
        env_var: "GITHUB_TOKEN",
        needs_key: true,
        hint: "via PAT",
    },
    ProviderInfo {
        name: "replicate",
        display: "Replicate",
        env_var: "REPLICATE_API_KEY",
        needs_key: true,
        hint: "",
    },
    ProviderInfo {
        name: "claude-code",
        display: "Claude Code",
        env_var: "",
        needs_key: false,
        hint: "no API key",
    },
    ProviderInfo {
        name: "ollama",
        display: "Ollama",
        env_var: "OLLAMA_API_KEY",
        needs_key: false,
        hint: "local",
    },
    ProviderInfo {
        name: "lmstudio",
        display: "LM Studio",
        env_var: "LMSTUDIO_API_KEY",
        needs_key: false,
        hint: "local",
    },
    ProviderInfo {
        name: "vllm",
        display: "vLLM",
        env_var: "VLLM_API_KEY",
        needs_key: false,
        hint: "local",
    },
];

// ── Public result type ─────────────────────────────────────────────────────

/// What the user chose to do after init completes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LaunchChoice {
    Desktop,
    Dashboard,
    Chat,
}

pub enum InitResult {
    Completed {
        provider: String,
        model: String,
        daemon_started: bool,
        launch: LaunchChoice,
    },
    Cancelled,
}

// ── Internal state ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Migration,
    Provider,
    ApiKey,
    Model,
    Routing,
    Complete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MigrationPhase {
    Detecting,
    Offer,
    Running,
    Done,
}

/// Sub-state within the Routing step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RoutingPhase {
    /// Yes / No choice
    Choice,
    /// Picking model for a tier (0=fast, 1=balanced, 2=frontier)
    PickTier(usize),
}

#[derive(Clone, PartialEq, Eq)]
enum KeyTestState {
    Idle,
    Testing,
    Ok,
    Warn,
    // #3629: validation succeeded but persisting to .env failed. Surface this
    // explicitly so the user sees the disk error instead of a fake "Verified".
    SaveFailed(String),
}

/// A model entry for list display.
struct ModelEntry {
    id: String,
    display_name: String,
    tier: &'static str,
    cost: String,
}

const ROUTING_TIER_NAMES: [&str; 3] = ["Fast", "Balanced", "Frontier"];
const ROUTING_TIER_DESC: [&str; 3] = [
    "quick lookups, greetings, simple Q&A",
    "standard conversation, general tasks",
    "multi-step reasoning, code generation",
];

struct State {
    step: Step,
    tick: usize,

    // Migration
    migration_phase: MigrationPhase,
    migration_choice_list: ListState,
    openclaw_path: Option<PathBuf>,
    openclaw_scan: Option<librefang_migrate::openclaw::ScanResult>,
    openfang_path: Option<PathBuf>,
    migrate_source: Option<librefang_migrate::MigrateSource>,
    migration_report: Option<librefang_migrate::report::MigrationReport>,
    migration_error: Option<String>,
    migration_done_at: Option<Instant>,
    migrated_provider: Option<String>,

    // Provider selection
    provider_list: ListState,
    provider_order: Vec<usize>,
    selected_provider: Option<usize>,

    // API key
    api_key_input: String,
    api_key_from_env: bool,
    key_test: KeyTestState,
    key_test_started: Option<Instant>,

    // Model selection
    model_input: String,
    model_catalog: ModelCatalog,
    model_entries: Vec<ModelEntry>,
    model_list: ListState,

    // Routing
    routing_phase: RoutingPhase,
    routing_choice_list: ListState, // 0=Yes, 1=No
    routing_enabled: bool,
    /// Selected model IDs per tier: [fast, balanced, frontier]
    routing_models: [String; 3],
    routing_tier_list: ListState, // for PickTier model selection

    // Complete
    complete_list: ListState,
    daemon_started: bool,
    daemon_url: String,
    daemon_error: String,
    saving_done: bool,
    save_error: String,
}

impl State {
    fn new() -> Self {
        let mut s = Self {
            step: Step::Welcome,
            tick: 0,
            migration_phase: MigrationPhase::Detecting,
            migration_choice_list: ListState::default(),
            openclaw_path: None,
            openclaw_scan: None,
            openfang_path: None,
            migrate_source: None,
            migration_report: None,
            migration_error: None,
            migration_done_at: None,
            migrated_provider: None,
            provider_list: ListState::default(),
            provider_order: Vec::new(),
            selected_provider: None,
            api_key_input: String::new(),
            api_key_from_env: false,
            key_test: KeyTestState::Idle,
            key_test_started: None,
            model_input: String::new(),
            model_catalog: ModelCatalog::default(),
            model_entries: Vec::new(),
            model_list: ListState::default(),
            routing_phase: RoutingPhase::Choice,
            routing_choice_list: ListState::default(),
            routing_enabled: false,
            routing_models: [String::new(), String::new(), String::new()],
            routing_tier_list: ListState::default(),
            complete_list: ListState::default(),
            daemon_started: false,
            daemon_url: String::new(),
            daemon_error: String::new(),
            saving_done: false,
            save_error: String::new(),
        };
        s.build_provider_order();
        s.provider_list.select(Some(0));
        s.migration_choice_list.select(Some(0));
        s.routing_choice_list.select(Some(0));
        s.complete_list.select(Some(0));
        s
    }

    fn build_provider_order(&mut self) {
        let has_key = |var: &str| std::env::var(var).is_ok_and(|v| !v.trim().is_empty());
        self.provider_order.clear();
        let gemini_via_google = has_key("GOOGLE_API_KEY");
        for (i, p) in PROVIDERS.iter().enumerate() {
            let detected = if p.name == "claude-code" {
                librefang_runtime::drivers::claude_code::claude_code_available()
            } else {
                (!p.env_var.is_empty() && has_key(p.env_var))
                    || (p.name == "gemini" && gemini_via_google)
            };
            if detected {
                self.provider_order.push(i);
            }
        }
        for (i, p) in PROVIDERS.iter().enumerate() {
            let detected = if p.name == "claude-code" {
                librefang_runtime::drivers::claude_code::claude_code_available()
            } else {
                (!p.env_var.is_empty() && has_key(p.env_var))
                    || (p.name == "gemini" && gemini_via_google)
            };
            if !detected {
                self.provider_order.push(i);
            }
        }
    }

    fn provider(&self) -> Option<&'static ProviderInfo> {
        self.selected_provider.map(|i| &PROVIDERS[i])
    }

    fn step_label(&self) -> &'static str {
        match self.step {
            Step::Welcome => "1 of 7",
            Step::Migration => "2 of 7",
            Step::Provider => "3 of 7",
            Step::ApiKey => "4 of 7",
            Step::Model => "5 of 7",
            Step::Routing => "6 of 7",
            Step::Complete => "7 of 7",
        }
    }

    fn step_index(&self) -> usize {
        match self.step {
            Step::Welcome => 0,
            Step::Migration => 1,
            Step::Provider => 2,
            Step::ApiKey => 3,
            Step::Model => 4,
            Step::Routing => 5,
            Step::Complete => 6,
        }
    }

    /// Advance to the Provider step, optionally pre-selecting a migrated provider.
    fn advance_to_provider(&mut self) {
        if let Some(ref prov_name) = self.migrated_provider {
            // Find the provider in the ordered list and pre-select it
            for (list_idx, &prov_idx) in self.provider_order.iter().enumerate() {
                if PROVIDERS[prov_idx].name == prov_name.as_str() {
                    self.provider_list.select(Some(list_idx));
                    break;
                }
            }
        }
        self.step = Step::Provider;
    }

    fn is_provider_detected(&self, prov_idx: usize) -> bool {
        let has_key = |var: &str| std::env::var(var).is_ok_and(|v| !v.trim().is_empty());
        let p = &PROVIDERS[prov_idx];
        if p.name == "claude-code" {
            return librefang_runtime::drivers::claude_code::claude_code_available();
        }
        (!p.env_var.is_empty() && has_key(p.env_var))
            || (p.name == "gemini" && has_key("GOOGLE_API_KEY"))
    }

    /// Populate model_entries from the catalog for the selected provider.
    fn load_models_for_provider(&mut self) {
        self.model_entries.clear();
        let p = match self.provider() {
            Some(p) => p,
            None => return,
        };

        let models = self.model_catalog.models_by_provider(p.name);
        let default_model = default_model_for_provider(p.name, &self.model_catalog);
        let mut default_idx = 0usize;

        for (i, m) in models.iter().enumerate() {
            let tier = tier_label(m.tier);
            let cost = if m.input_cost_per_m == 0.0 && m.output_cost_per_m == 0.0 {
                "free".to_string()
            } else {
                format!("${:.2}/${:.2}", m.input_cost_per_m, m.output_cost_per_m)
            };

            if m.id == default_model {
                default_idx = i;
            }

            self.model_entries.push(ModelEntry {
                id: m.id.clone(),
                display_name: m.display_name.clone(),
                tier,
                cost,
            });
        }

        if self.model_entries.is_empty() {
            self.model_entries.push(ModelEntry {
                id: default_model.clone(),
                display_name: default_model,
                tier: "default",
                cost: String::new(),
            });
        }

        self.model_list.select(Some(default_idx));
    }

    fn selected_model_id(&self) -> String {
        if let Some(idx) = self.model_list.selected() {
            if let Some(entry) = self.model_entries.get(idx) {
                return entry.id.clone();
            }
        }
        self.provider()
            .map(|p| default_model_for_provider(p.name, &self.model_catalog))
            .unwrap_or_default()
    }

    /// Auto-select routing models based on the provider's catalog entries.
    fn auto_select_routing_models(&mut self) {
        let p = match self.provider() {
            Some(p) => p,
            None => return,
        };

        let models = self.model_catalog.models_by_provider(p.name);

        // Find best candidates per target tier
        let mut fast: Option<&str> = None;
        let mut balanced: Option<&str> = None;
        let mut frontier: Option<&str> = None;

        for m in &models {
            match m.tier {
                ModelTier::Fast | ModelTier::Local | ModelTier::Custom => {
                    if fast.is_none() {
                        fast = Some(&m.id);
                    }
                }
                ModelTier::Balanced => {
                    if balanced.is_none() {
                        balanced = Some(&m.id);
                    }
                }
                ModelTier::Smart => {
                    // Smart is a good balanced pick; also good frontier if no frontier exists
                    if balanced.is_none() {
                        balanced = Some(&m.id);
                    }
                    if frontier.is_none() {
                        frontier = Some(&m.id);
                    }
                }
                ModelTier::Frontier => {
                    if frontier.is_none() {
                        frontier = Some(&m.id);
                    }
                }
                _ => {}
            }
        }

        // Fallback: use selected default model for any missing tier
        let fallback = &self.model_input;
        self.routing_models[0] = fast.unwrap_or(fallback).to_string();
        self.routing_models[1] = balanced.unwrap_or(fallback).to_string();
        self.routing_models[2] = frontier.unwrap_or(fallback).to_string();
    }

    /// Pre-select the routing_tier_list to match the current routing_models[tier].
    fn select_routing_tier_model(&mut self, tier: usize) {
        let target = &self.routing_models[tier];
        let idx = self
            .model_entries
            .iter()
            .position(|e| e.id == *target)
            .unwrap_or(0);
        self.routing_tier_list.select(Some(idx));
    }
}

fn tier_label(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Frontier => "frontier",
        ModelTier::Smart => "smart",
        ModelTier::Balanced => "balanced",
        ModelTier::Fast => "fast",
        ModelTier::Local => "local",
        ModelTier::Custom => "custom",
        _ => "unknown",
    }
}

fn render_init_wizard_config(
    provider: &str,
    model: &str,
    api_key_line: &str,
    routing_section: &str,
) -> String {
    INIT_WIZARD_CONFIG_TEMPLATE
        .replace("{{provider}}", provider)
        .replace("{{model}}", model)
        .replace("{{api_key_line}}", api_key_line)
        .replace("{{routing_section}}", routing_section)
}

fn default_model_for_provider(provider: &str, model_catalog: &ModelCatalog) -> String {
    model_catalog
        .default_model_for_provider(provider)
        .unwrap_or_else(|| "local-model".to_string())
}

/// Build the `[default_routing]` TOML section emitted into `config.toml`.
///
/// Pure helper extracted from `save_config` (#3582) so the formatting can be
/// unit-tested without touching the filesystem or the wizard `State`. When
/// `enabled` is `false`, the wizard writes no routing section at all and we
/// must return an empty string (callers concat this directly into the
/// rendered template).
///
/// Issue #4466: this previously emitted `[routing]`, which is not a known
/// `KernelConfig` field — strict-config mode warned and the kernel silently
/// ignored the user's Smart Router selection. The kernel now reads
/// `[default_routing]` as a fallback for any agent without its own per-agent
/// `routing` block, so the wizard emits that exact key here.
fn build_routing_section(enabled: bool, models: &[String; 3]) -> String {
    if !enabled {
        return String::new();
    }
    format!(
        r#"
[default_routing]
simple_model = "{fast}"
medium_model = "{balanced}"
complex_model = "{frontier}"
simple_threshold = 100
complex_threshold = 500
"#,
        fast = models[0],
        balanced = models[1],
        frontier = models[2],
    )
}

/// Build the `api_key_env = "..."` line for providers that read their key
/// from an environment variable. Returns an empty string for providers that
/// don't need a key (e.g. `claude-code`, `local`) so the rendered template
/// has no dangling assignment.
fn build_api_key_line(env_var: &str) -> String {
    if env_var.is_empty() {
        String::new()
    } else {
        format!("api_key_env = \"{env_var}\"")
    }
}

// ── Entry point ────────────────────────────────────────────────────────────

pub fn run() -> InitResult {
    // Guard against non-TTY environments (Docker, piped, CI/CD)
    if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        return InitResult::Cancelled;
    }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        original_hook(info);
    }));

    let mut terminal = ratatui::init();

    // Enable bracketed paste so multi-character pastes (e.g. an API key) arrive
    // as a single `Paste` event instead of thousands of synthesized Key events
    // (#3638). We disable it again before `ratatui::restore()`.
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableBracketedPaste
    );

    let mut state = State::new();

    let (test_tx, test_rx) = std::sync::mpsc::channel::<bool>();
    let (migrate_tx, migrate_rx) =
        std::sync::mpsc::channel::<Result<librefang_migrate::report::MigrationReport, String>>();

    let result = loop {
        terminal
            .draw(|f| draw(f, f.area(), &mut state))
            .expect("draw failed");

        // Check for background key-test result.
        // The API key is written to disk HERE — after validation — not on the
        // initial Enter press (fixes #3629: write-before-validate).
        if state.key_test == KeyTestState::Testing {
            if let Ok(ok) = test_rx.try_recv() {
                // #3629: never advance silently. If the disk save fails after a
                // verified key, surface it as SaveFailed so the user retries
                // instead of seeing a fake "Verified" while ~/.librefang/.env
                // is still empty.
                state.key_test = if ok {
                    let mut save_err: Option<String> = None;
                    if let Some(p) = state.provider() {
                        if !p.env_var.is_empty() {
                            if let Err(e) = dotenv::save_env_key(p.env_var, &state.api_key_input) {
                                tracing::error!(
                                    provider = ?p.name,
                                    error = %e,
                                    "init wizard: failed to persist verified API key"
                                );
                                save_err = Some(e);
                            }
                        }
                    }
                    match save_err {
                        Some(e) => KeyTestState::SaveFailed(e),
                        None => KeyTestState::Ok,
                    }
                } else {
                    KeyTestState::Warn
                };
                state.key_test_started = Some(Instant::now());
            }
        }

        // Auto-advance from key test result after 600ms
        if matches!(state.key_test, KeyTestState::Ok | KeyTestState::Warn) {
            if let Some(started) = state.key_test_started {
                if started.elapsed() >= Duration::from_millis(600) {
                    state.load_models_for_provider();
                    state.step = Step::Model;
                    state.key_test = KeyTestState::Idle;
                    state.key_test_started = None;
                }
            }
        }

        // ── Migration detection (resolves in 1 frame) ──
        if state.step == Step::Migration && state.migration_phase == MigrationPhase::Detecting {
            // Check OpenClaw first (more complex migration with scan)
            let openclaw_found = match librefang_migrate::openclaw::detect_openclaw_home() {
                Some(path) => {
                    let scan = librefang_migrate::openclaw::scan_openclaw_workspace(&path);
                    let has_content = scan.has_config
                        || !scan.agents.is_empty()
                        || !scan.channels.is_empty()
                        || !scan.skills.is_empty()
                        || scan.has_memory;
                    if has_content {
                        state.openclaw_path = Some(path);
                        state.openclaw_scan = Some(scan);
                        state.migrate_source = Some(librefang_migrate::MigrateSource::OpenClaw);
                        true
                    } else {
                        false
                    }
                }
                None => false,
            };

            // If no OpenClaw, check OpenFang
            if !openclaw_found {
                let openfang_home = dirs::home_dir().map(|h| h.join(".openfang"));
                match openfang_home {
                    Some(path) if path.exists() && path.is_dir() => {
                        // OpenFang uses the same format — just check it has files
                        let has_content = path.join("config.toml").exists()
                            || path.join("workspaces").join("agents").exists()
                            || path.join("skills").exists();
                        if has_content {
                            state.openfang_path = Some(path);
                            state.migrate_source = Some(librefang_migrate::MigrateSource::OpenFang);
                            state.migration_phase = MigrationPhase::Offer;
                        } else {
                            state.advance_to_provider();
                        }
                    }
                    _ => {
                        state.advance_to_provider();
                    }
                }
            } else {
                state.migration_phase = MigrationPhase::Offer;
            }
        }

        // ── Migration background result polling ──
        if state.step == Step::Migration && state.migration_phase == MigrationPhase::Running {
            if let Ok(result) = migrate_rx.try_recv() {
                match result {
                    Ok(report) => {
                        // Extract provider from first imported agent for pre-selection
                        if let Some(scan) = &state.openclaw_scan {
                            for agent in &scan.agents {
                                if !agent.provider.is_empty() {
                                    state.migrated_provider = Some(agent.provider.clone());
                                    break;
                                }
                            }
                        }
                        state.migration_report = Some(report);
                        state.migration_phase = MigrationPhase::Done;
                        state.migration_done_at = Some(Instant::now());
                    }
                    Err(e) => {
                        state.migration_error = Some(e);
                        state.migration_phase = MigrationPhase::Done;
                        state.migration_done_at = Some(Instant::now());
                    }
                }
            }
        }

        // ── Migration auto-advance 1.5s after Done ──
        if state.step == Step::Migration && state.migration_phase == MigrationPhase::Done {
            if let Some(done_at) = state.migration_done_at {
                if done_at.elapsed() >= Duration::from_millis(1500) {
                    state.advance_to_provider();
                }
            }
        }

        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            // Resize falls through to the next loop iteration so the next
            // `terminal.draw(...)` picks up the new size automatically (#3638).
            let read = event::read();
            // Handle bracketed paste — pasting an API key during Step::ApiKey
            // must arrive as a single string, not synthesized Key events.
            if let Ok(CtEvent::Paste(text)) = &read {
                if state.step == Step::ApiKey && state.key_test == KeyTestState::Idle {
                    // Strip newlines/tabs/control chars; API keys never contain them
                    // and terminals occasionally append a stray \r.
                    for c in text.chars() {
                        if !c.is_control() {
                            state.api_key_input.push(c);
                        }
                    }
                }
                continue;
            }
            if let Ok(CtEvent::Key(key)) = read {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if key.code == KeyCode::Char('c')
                    && key
                        .modifiers
                        .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
                {
                    break InitResult::Cancelled;
                }

                match state.step {
                    Step::Welcome => match key.code {
                        KeyCode::Enter => {
                            state.migration_phase = MigrationPhase::Detecting;
                            state.step = Step::Migration;
                        }
                        KeyCode::Esc => break InitResult::Cancelled,
                        _ => {}
                    },

                    Step::Migration => handle_migration_key(&mut state, key.code, &migrate_tx),

                    Step::Provider => match key.code {
                        KeyCode::Esc => break InitResult::Cancelled,
                        KeyCode::Up | KeyCode::Char('k') => {
                            let i = state.provider_list.selected().unwrap_or(0);
                            let next = if i == 0 {
                                state.provider_order.len() - 1
                            } else {
                                i - 1
                            };
                            state.provider_list.select(Some(next));
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let i = state.provider_list.selected().unwrap_or(0);
                            let next = (i + 1) % state.provider_order.len();
                            state.provider_list.select(Some(next));
                        }
                        KeyCode::Enter => {
                            if let Some(list_idx) = state.provider_list.selected() {
                                let prov_idx = state.provider_order[list_idx];
                                state.selected_provider = Some(prov_idx);
                                let p = &PROVIDERS[prov_idx];

                                if !p.needs_key {
                                    state.api_key_from_env = false;
                                    state.load_models_for_provider();
                                    state.step = Step::Model;
                                } else if state.is_provider_detected(prov_idx) {
                                    state.api_key_from_env = true;
                                    state.load_models_for_provider();
                                    state.step = Step::Model;
                                } else {
                                    state.api_key_from_env = false;
                                    state.api_key_input.clear();
                                    state.key_test = KeyTestState::Idle;
                                    state.step = Step::ApiKey;
                                }
                            }
                        }
                        _ => {}
                    },

                    Step::ApiKey => {
                        if matches!(state.key_test, KeyTestState::Ok | KeyTestState::Warn) {
                            continue;
                        }

                        // #3629 follow-up: from SaveFailed the user has an
                        // already-validated key in `api_key_input` — Enter
                        // should retry ONLY the disk write (no re-validate, no
                        // rate-limit hit), and Esc should preserve the key
                        // text so the user is not forced to retype it after a
                        // transient disk error. Other input (typing /
                        // backspace) stays disabled to avoid mutating the
                        // already-verified value.
                        if let KeyTestState::SaveFailed(_) = &state.key_test {
                            match key.code {
                                KeyCode::Enter => {
                                    let mut new_err: Option<String> = None;
                                    if let Some(p) = state.provider() {
                                        if !p.env_var.is_empty() {
                                            if let Err(e) = dotenv::save_env_key(
                                                p.env_var,
                                                &state.api_key_input,
                                            ) {
                                                tracing::error!(
                                                    provider = ?p.name,
                                                    error = %e,
                                                    "init wizard: retry of save_env_key failed"
                                                );
                                                new_err = Some(e);
                                            }
                                        }
                                    }
                                    state.key_test = match new_err {
                                        Some(e) => KeyTestState::SaveFailed(e),
                                        None => KeyTestState::Ok,
                                    };
                                    state.key_test_started = Some(Instant::now());
                                }
                                KeyCode::Esc => {
                                    // Keep `api_key_input` so the user can
                                    // retry without re-typing or re-validating.
                                    state.key_test = KeyTestState::Idle;
                                }
                                _ => {}
                            }
                            continue;
                        }

                        match key.code {
                            KeyCode::Esc => {
                                state.key_test = KeyTestState::Idle;
                                state.step = Step::Provider;
                            }
                            KeyCode::Enter
                                if !state.api_key_input.is_empty()
                                    && state.key_test == KeyTestState::Idle =>
                            {
                                // Validate the API key with the provider BEFORE writing it to
                                // disk. The dotenv write happens only after the test result
                                // arrives (see the test_rx polling block above). This prevents
                                // a partially-written .env file if the wizard is cancelled
                                // while the test is in flight (fixes #3629).
                                state.key_test = KeyTestState::Testing;
                                let provider_name = state
                                    .provider()
                                    .map(|p| p.name.to_string())
                                    .unwrap_or_default();
                                let key_value = state.api_key_input.clone();
                                let tx = test_tx.clone();
                                std::thread::spawn(move || {
                                    let ok = crate::test_api_key(&provider_name, &key_value);
                                    let _ = tx.send(ok);
                                });
                            }
                            KeyCode::Char(c) if state.key_test == KeyTestState::Idle => {
                                state.api_key_input.push(c);
                            }
                            KeyCode::Backspace if state.key_test == KeyTestState::Idle => {
                                state.api_key_input.pop();
                            }
                            _ => {}
                        }
                    }

                    Step::Model => match key.code {
                        KeyCode::Esc => {
                            if let Some(p) = state.provider() {
                                if p.needs_key && !state.api_key_from_env {
                                    state.key_test = KeyTestState::Idle;
                                    state.step = Step::ApiKey;
                                } else {
                                    state.step = Step::Provider;
                                }
                            } else {
                                state.step = Step::Provider;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            let len = state.model_entries.len().max(1);
                            let i = state.model_list.selected().unwrap_or(0);
                            let next = if i == 0 { len - 1 } else { i - 1 };
                            state.model_list.select(Some(next));
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let len = state.model_entries.len().max(1);
                            let i = state.model_list.selected().unwrap_or(0);
                            let next = (i + 1) % len;
                            state.model_list.select(Some(next));
                        }
                        KeyCode::Enter => {
                            state.model_input = state.selected_model_id();
                            // Prepare routing step
                            state.routing_phase = RoutingPhase::Choice;
                            state.routing_choice_list.select(Some(0));
                            // Only offer routing if provider has 2+ models
                            if state.model_entries.len() < 2 {
                                // Skip routing — not enough models
                                state.routing_enabled = false;
                                save_config(&mut state);
                                state.step = Step::Complete;
                            } else {
                                state.step = Step::Routing;
                            }
                        }
                        _ => {}
                    },

                    Step::Routing => handle_routing_key(&mut state, key.code),

                    Step::Complete => match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            let i = state.complete_list.selected().unwrap_or(0);
                            let next = if i == 0 { 2 } else { i - 1 };
                            state.complete_list.select(Some(next));
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let i = state.complete_list.selected().unwrap_or(0);
                            let next = (i + 1) % 3;
                            state.complete_list.select(Some(next));
                        }
                        // Number shortcuts: 1=Desktop, 2=Dashboard, 3=Chat
                        KeyCode::Char('1') => {
                            state.complete_list.select(Some(0));
                        }
                        KeyCode::Char('2') => {
                            state.complete_list.select(Some(1));
                        }
                        KeyCode::Char('3') => {
                            state.complete_list.select(Some(2));
                        }
                        KeyCode::Enter => {
                            let choice = match state.complete_list.selected() {
                                Some(0) => LaunchChoice::Desktop,
                                Some(1) => LaunchChoice::Dashboard,
                                _ => LaunchChoice::Chat,
                            };
                            break InitResult::Completed {
                                provider: state
                                    .provider()
                                    .map(|p| p.name.to_string())
                                    .unwrap_or_default(),
                                model: state.model_input.clone(),
                                daemon_started: state.daemon_started,
                                launch: choice,
                            };
                        }
                        KeyCode::Esc => {
                            break InitResult::Completed {
                                provider: state
                                    .provider()
                                    .map(|p| p.name.to_string())
                                    .unwrap_or_default(),
                                model: state.model_input.clone(),
                                daemon_started: state.daemon_started,
                                launch: LaunchChoice::Chat,
                            };
                        }
                        _ => {}
                    },
                }
            }
        } else {
            state.tick = state.tick.wrapping_add(1);
        }
    };

    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableBracketedPaste
    );
    ratatui::restore();
    result
}

// ── Migration step key handler ─────────────────────────────────────────────

fn handle_migration_key(
    state: &mut State,
    code: KeyCode,
    migrate_tx: &std::sync::mpsc::Sender<
        Result<librefang_migrate::report::MigrationReport, String>,
    >,
) {
    match state.migration_phase {
        MigrationPhase::Detecting => {} // auto-resolves, no keys
        MigrationPhase::Offer => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = state.migration_choice_list.selected().unwrap_or(0);
                state
                    .migration_choice_list
                    .select(Some(if i == 0 { 1 } else { 0 }));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = state.migration_choice_list.selected().unwrap_or(0);
                state
                    .migration_choice_list
                    .select(Some(if i == 0 { 1 } else { 0 }));
            }
            KeyCode::Esc => {
                state.advance_to_provider();
            }
            KeyCode::Enter => {
                let yes = state.migration_choice_list.selected() == Some(0);
                if yes {
                    state.migration_phase = MigrationPhase::Running;
                    let migrate_source = state
                        .migrate_source
                        .unwrap_or(librefang_migrate::MigrateSource::OpenClaw);
                    let source_dir = match migrate_source {
                        librefang_migrate::MigrateSource::OpenFang => {
                            state.openfang_path.clone().unwrap_or_default()
                        }
                        _ => state.openclaw_path.clone().unwrap_or_default(),
                    };
                    let target_dir = if let Ok(h) = std::env::var("LIBREFANG_HOME") {
                        PathBuf::from(h)
                    } else {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".librefang")
                    };
                    let tx = migrate_tx.clone();
                    std::thread::spawn(move || {
                        let options = librefang_migrate::MigrateOptions {
                            source: migrate_source,
                            source_dir,
                            target_dir,
                            dry_run: false,
                        };
                        let result =
                            librefang_migrate::run_migration(&options).map_err(|e| format!("{e}"));
                        let _ = tx.send(result);
                    });
                } else {
                    state.advance_to_provider();
                }
            }
            _ => {}
        },
        MigrationPhase::Running => {} // ignore keys while running
        MigrationPhase::Done => {
            if code == KeyCode::Enter {
                state.advance_to_provider();
            }
        }
    }
}

// ── Routing step key handler ───────────────────────────────────────────────

fn handle_routing_key(state: &mut State, code: KeyCode) {
    match state.routing_phase {
        RoutingPhase::Choice => match code {
            KeyCode::Esc => {
                state.step = Step::Model;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = state.routing_choice_list.selected().unwrap_or(0);
                state
                    .routing_choice_list
                    .select(Some(if i == 0 { 1 } else { 0 }));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = state.routing_choice_list.selected().unwrap_or(0);
                state
                    .routing_choice_list
                    .select(Some(if i == 0 { 1 } else { 0 }));
            }
            KeyCode::Enter => {
                let yes = state.routing_choice_list.selected() == Some(0);
                if yes {
                    state.routing_enabled = true;
                    state.auto_select_routing_models();
                    state.routing_phase = RoutingPhase::PickTier(0);
                    state.select_routing_tier_model(0);
                } else {
                    state.routing_enabled = false;
                    save_config(state);
                    state.step = Step::Complete;
                }
            }
            _ => {}
        },
        RoutingPhase::PickTier(tier) => match code {
            KeyCode::Esc => {
                if tier == 0 {
                    state.routing_phase = RoutingPhase::Choice;
                } else {
                    state.routing_phase = RoutingPhase::PickTier(tier - 1);
                    state.select_routing_tier_model(tier - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = state.model_entries.len().max(1);
                let i = state.routing_tier_list.selected().unwrap_or(0);
                let next = if i == 0 { len - 1 } else { i - 1 };
                state.routing_tier_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = state.model_entries.len().max(1);
                let i = state.routing_tier_list.selected().unwrap_or(0);
                let next = (i + 1) % len;
                state.routing_tier_list.select(Some(next));
            }
            KeyCode::Enter => {
                // Save selected model for this tier
                if let Some(idx) = state.routing_tier_list.selected() {
                    if let Some(entry) = state.model_entries.get(idx) {
                        state.routing_models[tier] = entry.id.clone();
                    }
                }

                if tier < 2 {
                    // Advance to next tier
                    let next_tier = tier + 1;
                    state.routing_phase = RoutingPhase::PickTier(next_tier);
                    state.select_routing_tier_model(next_tier);
                } else {
                    // All 3 tiers picked — save and advance
                    save_config(state);
                    state.step = Step::Complete;
                }
            }
            _ => {}
        },
    }
}

// ── Config save ────────────────────────────────────────────────────────────

fn save_config(state: &mut State) {
    let p = match state.provider() {
        Some(p) => p,
        None => {
            state.save_error = "No provider selected".to_string();
            return;
        }
    };

    let librefang_dir = if let Ok(h) = std::env::var("LIBREFANG_HOME") {
        PathBuf::from(h)
    } else {
        match dirs::home_dir() {
            Some(h) => h.join(".librefang"),
            None => {
                state.save_error = "Could not determine home directory".to_string();
                return;
            }
        }
    };
    let _ = std::fs::create_dir_all(librefang_dir.join("workspaces").join("agents"));
    let _ = std::fs::create_dir_all(librefang_dir.join("data"));
    crate::restrict_dir_permissions(&librefang_dir);

    let default_model = default_model_for_provider(p.name, &state.model_catalog);
    let model = if state.model_input.is_empty() {
        default_model.as_str()
    } else {
        &state.model_input
    };

    let routing_section = build_routing_section(state.routing_enabled, &state.routing_models);

    let config_path = librefang_dir.join("config.toml");
    let api_key_line = build_api_key_line(p.env_var);

    let config = render_init_wizard_config(p.name, model, &api_key_line, &routing_section);

    match std::fs::write(&config_path, &config) {
        Ok(()) => {
            crate::restrict_file_permissions(&config_path);
        }
        Err(e) => {
            state.save_error = format!("Failed to write config: {e}");
            return;
        }
    }

    // Write config.example.toml with the full annotated template for reference
    let example_path = librefang_dir.join("config.example.toml");
    if !example_path.exists() {
        let example_content = include_str!("../../../templates/init_default_config.toml");
        let _ = std::fs::write(&example_path, example_content);
    }

    state.saving_done = true;

    // Auto-start the daemon so all launch options work immediately.
    match crate::start_daemon_background() {
        Ok(url) => {
            state.daemon_started = true;
            state.daemon_url = url;
        }
        Err(e) => {
            state.daemon_error = format!("Daemon failed: {e}");
        }
    }
}

/// Check if the `librefang-desktop` binary exists next to the current exe.
fn find_desktop_binary() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;

    #[cfg(windows)]
    let name = "librefang-desktop.exe";
    #[cfg(not(windows))]
    let name = "librefang-desktop";

    let path = dir.join(name);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

// ── Drawing ────────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, area: Rect, state: &mut State) {
    // Fill background
    f.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(theme::BG_PRIMARY)),
        area,
    );

    // Left-aligned content area (no centered card)
    let content = if area.width < 10 || area.height < 5 {
        area
    } else {
        let margin = 3u16.min(area.width.saturating_sub(10));
        let w = 72u16.min(area.width.saturating_sub(margin));
        Rect {
            x: area.x.saturating_add(margin),
            y: area.y,
            width: w,
            height: area.height,
        }
    };

    let chunks = Layout::vertical([
        Constraint::Length(1), // top pad
        Constraint::Length(1), // header
        Constraint::Length(1), // progress bar
        Constraint::Length(1), // separator
        Constraint::Min(1),    // step content
    ])
    .split(content);

    // Header: "LibreFang Init  Step X of 7"
    let header = Line::from(vec![
        Span::styled(
            "LibreFang",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Init", Style::default().fg(theme::TEXT_PRIMARY)),
        Span::styled(format!("  {}", state.step_label()), theme::dim_style()),
    ]);
    f.render_widget(Paragraph::new(header), chunks[1]);

    // Progress bar: ●──●──●──○──○──○──○
    let step_idx = state.step_index();
    let mut progress_spans: Vec<Span> = Vec::new();
    for i in 0..7 {
        if i > 0 {
            let line_style = if i <= step_idx {
                Style::default().fg(theme::ACCENT)
            } else {
                Style::default().fg(theme::BORDER)
            };
            progress_spans.push(Span::styled("\u{2500}\u{2500}", line_style));
        }
        if i < step_idx {
            progress_spans.push(Span::styled("\u{25cf}", Style::default().fg(theme::ACCENT)));
        } else if i == step_idx {
            progress_spans.push(Span::styled(
                "\u{25cf}",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            progress_spans.push(Span::styled("\u{25cb}", Style::default().fg(theme::BORDER)));
        }
    }
    f.render_widget(Paragraph::new(Line::from(progress_spans)), chunks[2]);

    // Separator
    f.render_widget(widgets::separator(content.width.min(60)), chunks[3]);

    // Step content (full remaining area)
    match state.step {
        Step::Welcome => draw_welcome(f, chunks[4]),
        Step::Migration => draw_migration(f, chunks[4], state),
        Step::Provider => draw_provider(f, chunks[4], state),
        Step::ApiKey => draw_api_key(f, chunks[4], state),
        Step::Model => draw_model(f, chunks[4], state),
        Step::Routing => draw_routing(f, chunks[4], state),
        Step::Complete => draw_complete(f, chunks[4], state),
    }
}

fn draw_welcome(f: &mut Frame, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let logo = Paragraph::new(Line::from(vec![Span::styled(
        "L I B R E F A N G",
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    )]))
    .alignment(Alignment::Center);
    f.render_widget(logo, chunks[1]);

    let tagline = Paragraph::new(Line::from(vec![Span::styled(
        "Agent Operating System",
        theme::dim_style(),
    )]))
    .alignment(Alignment::Center);
    f.render_widget(tagline, chunks[2]);

    f.render_widget(widgets::separator(area.width.saturating_sub(2)), chunks[3]);

    let sec1 = Paragraph::new(Line::from(vec![
        Span::styled("  \u{1f6e1} ", Style::default().fg(theme::GREEN)),
        Span::raw("Sandboxed execution, WASM isolation, SSRF protection"),
    ]));
    f.render_widget(sec1, chunks[5]);

    let sec2 = Paragraph::new(Line::from(vec![
        Span::styled("  \u{1f512} ", Style::default().fg(theme::GREEN)),
        Span::raw("Signed manifests, audit trail, taint tracking"),
    ]));
    f.render_widget(sec2, chunks[6]);

    let sec3 = Paragraph::new(Line::from(vec![
        Span::styled("  \u{1f50d} ", Style::default().fg(theme::GREEN)),
        Span::raw("RBAC, capability checks, secret zeroization"),
    ]));
    f.render_widget(sec3, chunks[7]);

    let sec4 = Paragraph::new(Line::from(vec![
        Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
        Span::raw("API keys never logged, 0600 file permissions"),
    ]));
    f.render_widget(sec4, chunks[8]);

    f.render_widget(widgets::separator(area.width.saturating_sub(2)), chunks[10]);

    let resp1 = Paragraph::new(Line::from(vec![Span::styled(
        "  Agents can execute code, access the network, and act",
        Style::default().fg(theme::TEXT_SECONDARY),
    )]));
    f.render_widget(resp1, chunks[12]);

    let resp2 = Paragraph::new(Line::from(vec![
        Span::styled(
            "  on your behalf. ",
            Style::default().fg(theme::TEXT_SECONDARY),
        ),
        Span::styled(
            "You are responsible for what they do.",
            Style::default().fg(theme::YELLOW),
        ),
    ]));
    f.render_widget(resp2, chunks[13]);

    f.render_widget(
        widgets::hint_bar("  [Enter] I understand    [Esc] Cancel"),
        chunks[15],
    );
}

fn draw_migration(f: &mut Frame, area: Rect, state: &mut State) {
    match state.migration_phase {
        MigrationPhase::Detecting => draw_migration_detecting(f, area, state),
        MigrationPhase::Offer => draw_migration_offer(f, area, state),
        MigrationPhase::Running => draw_migration_running(f, area, state),
        MigrationPhase::Done => draw_migration_done(f, area, state),
    }
}

fn draw_migration_detecting(f: &mut Frame, area: Rect, state: &State) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area);

    let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(spinner, Style::default().fg(theme::ACCENT)),
            Span::raw(" Checking for existing installations..."),
        ])),
        chunks[1],
    );
}

fn draw_migration_offer(f: &mut Frame, area: Rect, state: &mut State) {
    let is_openfang = matches!(
        state.migrate_source,
        Some(librefang_migrate::MigrateSource::OpenFang)
    );

    // For OpenClaw we need the scan; for OpenFang we just need the path
    if !is_openfang && state.openclaw_scan.is_none() {
        return;
    }

    let path_display = if is_openfang {
        state
            .openfang_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    } else {
        state
            .openclaw_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };

    // Count content lines to determine layout
    let mut content_lines: Vec<Line> = Vec::new();

    if is_openfang {
        // OpenFang uses the same format — just show a simple summary
        content_lines.push(Line::from(vec![
            Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
            Span::raw("OpenFang configuration and data"),
        ]));
    } else if let Some(scan) = &state.openclaw_scan {
        if !scan.agents.is_empty() {
            let names: Vec<&str> = scan.agents.iter().map(|a| a.name.as_str()).collect();
            let names_str = names.join(", ");
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw(format!("{} agents ({})", scan.agents.len(), names_str)),
            ]));
        } else {
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2500} ", theme::dim_style()),
                Span::styled("No agents", theme::dim_style()),
            ]));
        }

        if !scan.channels.is_empty() {
            let chan_str = scan.channels.join(", ");
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw(format!("{} channels ({})", scan.channels.len(), chan_str)),
            ]));
        } else {
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2500} ", theme::dim_style()),
                Span::styled("No channels", theme::dim_style()),
            ]));
        }

        if !scan.skills.is_empty() {
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw(format!("{} skills", scan.skills.len())),
            ]));
        } else {
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2500} ", theme::dim_style()),
                Span::styled("No skills", theme::dim_style()),
            ]));
        }

        if scan.has_memory {
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw("Memory files"),
            ]));
        } else {
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2500} ", theme::dim_style()),
                Span::styled("No memory files", theme::dim_style()),
            ]));
        }

        if scan.has_config {
            content_lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw("Configuration"),
            ]));
        }
    }

    let chunks = Layout::vertical([
        Constraint::Length(1),                          // 0: title
        Constraint::Length(1),                          // 1: path
        Constraint::Length(1),                          // 2: separator
        Constraint::Length(content_lines.len() as u16), // 3: scan items
        Constraint::Length(1),                          // 4: separator
        Constraint::Length(1),                          // 5: spacer
        Constraint::Length(1),                          // 6: option yes
        Constraint::Length(1),                          // 7: option no
        Constraint::Min(0),                             // 8: flex
        Constraint::Length(1),                          // 9: hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            match state.migrate_source {
                Some(librefang_migrate::MigrateSource::OpenFang) => {
                    "  OpenFang Installation Detected"
                }
                _ => "  OpenClaw Installation Detected",
            },
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )])),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {}", path_display),
            theme::dim_style(),
        )])),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  ".to_string() + &"\u{2500}".repeat(area.width.saturating_sub(6) as usize),
            Style::default().fg(theme::BORDER),
        )])),
        chunks[2],
    );

    // Render scan items
    for (i, line) in content_lines.iter().enumerate() {
        if i < chunks[3].height as usize {
            let line_area = Rect {
                x: chunks[3].x,
                y: chunks[3].y + i as u16,
                width: chunks[3].width,
                height: 1,
            };
            f.render_widget(Paragraph::new(line.clone()), line_area);
        }
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  ".to_string() + &"\u{2500}".repeat(area.width.saturating_sub(6) as usize),
            Style::default().fg(theme::BORDER),
        )])),
        chunks[4],
    );

    // Yes / No options
    let options = [("Yes", "migrate settings and data"), ("No", "start fresh")];

    for (i, (label, desc)) in options.iter().enumerate() {
        let selected = state.migration_choice_list.selected() == Some(i);
        let arrow = if selected {
            Span::styled("  \u{25b8} ", Style::default().fg(theme::ACCENT))
        } else {
            Span::raw("    ")
        };
        let label_style = if selected {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT_PRIMARY)
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                arrow,
                Span::styled(format!("{:<6}", label), label_style),
                Span::styled(*desc, theme::dim_style()),
            ])),
            chunks[6 + i],
        );
    }

    f.render_widget(
        widgets::hint_bar("  [\u{2191}\u{2193}] Navigate  [Enter] Select  [Esc] Skip"),
        chunks[9],
    );
}

fn draw_migration_running(f: &mut Frame, area: Rect, state: &State) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area);

    let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(spinner, Style::default().fg(theme::ACCENT)),
            Span::raw(match state.migrate_source {
                Some(librefang_migrate::MigrateSource::OpenFang) => " Migrating from OpenFang...",
                _ => " Migrating from OpenClaw...",
            }),
        ])),
        chunks[1],
    );
}

fn draw_migration_done(f: &mut Frame, area: Rect, state: &State) {
    let mut lines: Vec<Line> = Vec::new();

    if let Some(ref error) = state.migration_error {
        lines.push(Line::from(vec![
            Span::styled("  \u{2718} ", Style::default().fg(theme::RED)),
            Span::raw(format!("Migration failed: {}", error)),
        ]));
    } else if let Some(ref report) = state.migration_report {
        // Group imported items by kind
        use librefang_migrate::report::ItemKind;
        let config_count = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Config)
            .count();
        let agent_items: Vec<&str> = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Agent)
            .map(|i| i.name.as_str())
            .collect();
        let channel_items: Vec<&str> = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Channel)
            .map(|i| i.name.as_str())
            .collect();
        let memory_count = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Memory)
            .count();
        let skill_count = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Skill)
            .count();
        let session_count = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Session)
            .count();

        if config_count > 0 {
            lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw("Config migrated"),
            ]));
        }

        if !agent_items.is_empty() {
            let names = agent_items.join(", ");
            lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw(format!("{} agents imported ({})", agent_items.len(), names)),
            ]));
        }

        if !channel_items.is_empty() {
            let names = channel_items.join(", ");
            lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw(format!("{} channels ({})", channel_items.len(), names)),
            ]));
        }

        if memory_count > 0 {
            lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw("Memory files copied"),
            ]));
        }

        if skill_count > 0 {
            lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw(format!("{} skills imported", skill_count)),
            ]));
        }

        if session_count > 0 {
            lines.push(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::raw(format!("{} sessions imported", session_count)),
            ]));
        }

        for skipped in &report.skipped {
            lines.push(Line::from(vec![
                Span::styled("  \u{26a0} ", Style::default().fg(theme::YELLOW)),
                Span::raw(format!("{} skipped ({})", skipped.name, skipped.reason)),
            ]));
        }

        for warning in &report.warnings {
            lines.push(Line::from(vec![
                Span::styled("  \u{26a0} ", Style::default().fg(theme::YELLOW)),
                Span::raw(warning.clone()),
            ]));
        }

        // Summary line
        lines.push(Line::from(vec![Span::styled(
            "  ".to_string() + &"\u{2500}".repeat(area.width.saturating_sub(6) as usize),
            Style::default().fg(theme::BORDER),
        )]));
        lines.push(Line::from(vec![Span::raw(format!(
            "  {} imported, {} skipped, {} warnings",
            report.imported.len(),
            report.skipped.len(),
            report.warnings.len(),
        ))]));
    }

    let content_height = lines.len() as u16;

    let chunks = Layout::vertical([
        Constraint::Length(1),              // 0: spacer
        Constraint::Length(content_height), // 1: results
        Constraint::Length(1),              // 2: spacer
        Constraint::Min(0),                 // 3: flex
        Constraint::Length(1),              // 4: hints
    ])
    .split(area);

    // Render result lines
    for (i, line) in lines.iter().enumerate() {
        if i < chunks[1].height as usize {
            let line_area = Rect {
                x: chunks[1].x,
                y: chunks[1].y + i as u16,
                width: chunks[1].width,
                height: 1,
            };
            f.render_widget(Paragraph::new(line.clone()), line_area);
        }
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  [Enter] Continue  ", theme::hint_style()),
            Span::styled("(auto-advancing...)", theme::dim_style()),
        ])),
        chunks[4],
    );
}

fn draw_provider(f: &mut Frame, area: Rect, state: &mut State) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    let prompt = Paragraph::new(Line::from(vec![Span::raw("  Choose your LLM provider:")]));
    f.render_widget(prompt, chunks[0]);

    let items: Vec<ListItem> = state
        .provider_order
        .iter()
        .map(|&idx| {
            let p = &PROVIDERS[idx];
            let detected = state.is_provider_detected(idx);
            let icon = if detected {
                Span::styled("\u{25cf} ", Style::default().fg(theme::GREEN))
            } else if !p.needs_key {
                Span::styled("\u{25cb} ", Style::default().fg(theme::BLUE))
            } else {
                Span::styled("  ", Style::default())
            };
            let name_span = Span::raw(format!("{:<14}", p.display));
            let hint_text = if p.name == "claude-code" {
                if detected {
                    "CLI detected".to_string()
                } else {
                    "no API key needed".to_string()
                }
            } else if detected {
                format!("{} detected", p.env_var)
            } else if !p.needs_key {
                "local, no key needed".to_string()
            } else if !p.hint.is_empty() {
                format!("requires {} ({})", p.env_var, p.hint)
            } else {
                format!("requires {}", p.env_var)
            };
            ListItem::new(Line::from(vec![
                icon,
                name_span,
                Span::styled(hint_text, theme::dim_style()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(theme::selected_style())
        .highlight_symbol("\u{25b8} ");
    f.render_stateful_widget(list, chunks[1], &mut state.provider_list);

    f.render_widget(
        widgets::hint_bar("  [\u{2191}\u{2193}/jk] Navigate  [Enter] Select  [Esc] Cancel"),
        chunks[2],
    );
}

fn draw_api_key(f: &mut Frame, area: Rect, state: &mut State) {
    let p = match state.provider() {
        Some(p) => p,
        None => return,
    };

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let prompt = Paragraph::new(Line::from(vec![Span::raw(format!(
        "  Enter your {} API key:",
        p.display
    ))]));
    f.render_widget(prompt, chunks[0]);

    match &state.key_test {
        KeyTestState::Idle => {
            let masked: String = "\u{2022}".repeat(state.api_key_input.len());
            let input = Paragraph::new(Line::from(vec![
                Span::raw("  \u{25b8} "),
                Span::styled(&masked, theme::input_style()),
                Span::styled(
                    "\u{2588}",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ]));
            f.render_widget(input, chunks[1]);
            let env_hint = Paragraph::new(Line::from(vec![Span::styled(
                format!("    Or set {} environment variable", p.env_var),
                theme::dim_style(),
            )]));
            f.render_widget(env_hint, chunks[3]);
        }
        KeyTestState::Testing => {
            let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
            let input = Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(spinner, Style::default().fg(theme::ACCENT)),
                Span::raw(" Testing API key..."),
            ]));
            f.render_widget(input, chunks[1]);
        }
        KeyTestState::Ok => {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                    Span::raw("API key verified"),
                ])),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    "    Saved to ~/.librefang/.env",
                    theme::dim_style(),
                )])),
                chunks[3],
            );
        }
        KeyTestState::Warn => {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("  \u{26a0} ", Style::default().fg(theme::YELLOW)),
                    Span::raw("Could not verify (may still work)"),
                ])),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    "    Saved to ~/.librefang/.env",
                    theme::dim_style(),
                )])),
                chunks[3],
            );
        }
        KeyTestState::SaveFailed(err) => {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("  \u{2716} ", Style::default().fg(theme::YELLOW)),
                    Span::raw("Verified, but saving to .env failed"),
                ])),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    format!("    {err}"),
                    theme::dim_style(),
                )])),
                chunks[3],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    "    [Enter] retry save  ·  [Esc] edit key  (key already verified — nothing on disk)",
                    theme::dim_style(),
                )])),
                chunks[4],
            );
        }
    }

    f.render_widget(
        widgets::hint_bar("  [Enter] Confirm  [Esc] Back"),
        chunks[5],
    );
}

fn draw_model(f: &mut Frame, area: Rect, state: &mut State) {
    let p = match state.provider() {
        Some(p) => p,
        None => return,
    };

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::raw(format!(
            "  Choose default model for {}:",
            p.display
        ))])),
        chunks[0],
    );

    let default_model = default_model_for_provider(p.name, &state.model_catalog);
    let items = build_model_list_items(&state.model_entries, Some(default_model.as_str()));
    let list = List::new(items)
        .highlight_style(theme::selected_style())
        .highlight_symbol("\u{25b8} ");
    f.render_stateful_widget(list, chunks[1], &mut state.model_list);

    f.render_widget(
        widgets::hint_bar(
            "  [\u{2191}\u{2193}/jk] Navigate  [Enter] Select  [Esc] Back    * = default",
        ),
        chunks[2],
    );
}

fn draw_routing(f: &mut Frame, area: Rect, state: &mut State) {
    match state.routing_phase {
        RoutingPhase::Choice => draw_routing_choice(f, area, state),
        RoutingPhase::PickTier(tier) => draw_routing_pick(f, area, state, tier),
    }
}

fn draw_routing_choice(f: &mut Frame, area: Rect, state: &mut State) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Length(1), // description 1
        Constraint::Length(1), // description 2
        Constraint::Length(1), // description 3
        Constraint::Length(1), // spacer
        Constraint::Length(1), // separator
        Constraint::Length(1), // spacer
        Constraint::Length(1), // option yes
        Constraint::Length(1), // option no
        Constraint::Min(0),
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  Smart Model Routing",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )])),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  Automatically picks the right model per task complexity.",
            theme::dim_style(),
        )])),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  Simple tasks use cheap/fast models, complex tasks use",
            theme::dim_style(),
        )])),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  frontier models. Saves cost without sacrificing quality.",
            theme::dim_style(),
        )])),
        chunks[3],
    );

    f.render_widget(widgets::separator(area.width.saturating_sub(2)), chunks[5]);

    let options = [
        ("Yes", "pick 3 models (fast / balanced / frontier)"),
        ("No", "use one model for everything"),
    ];

    for (i, (label, desc)) in options.iter().enumerate() {
        let selected = state.routing_choice_list.selected() == Some(i);
        let arrow = if selected {
            Span::styled("  \u{25b8} ", Style::default().fg(theme::ACCENT))
        } else {
            Span::raw("    ")
        };
        let label_style = if selected {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT_PRIMARY)
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                arrow,
                Span::styled(format!("{:<6}", label), label_style),
                Span::styled(*desc, theme::dim_style()),
            ])),
            chunks[7 + i],
        );
    }

    f.render_widget(
        widgets::hint_bar("  [\u{2191}\u{2193}] Navigate  [Enter] Select  [Esc] Back"),
        chunks[10],
    );
}

fn draw_routing_pick(f: &mut Frame, area: Rect, state: &mut State, tier: usize) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // tier label
        Constraint::Length(1), // tier description
        Constraint::Length(1), // spacer + current selections
        Constraint::Min(3),    // model list
        Constraint::Length(1), // hints
    ])
    .split(area);

    // Tier header with colored label
    let tier_color = match tier {
        0 => theme::GREEN,
        1 => theme::YELLOW,
        _ => theme::PURPLE,
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  Pick "),
            Span::styled(
                ROUTING_TIER_NAMES[tier],
                Style::default().fg(tier_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" model ({}/3):", tier + 1)),
        ])),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {}", ROUTING_TIER_DESC[tier]),
            theme::dim_style(),
        )])),
        chunks[1],
    );

    // Show already-picked tiers as summary
    let tier_colors = [theme::GREEN, theme::YELLOW, theme::PURPLE];
    let mut summary_spans: Vec<Span> = vec![Span::raw("  ")];
    for (t, (name, c)) in ROUTING_TIER_NAMES
        .iter()
        .zip(tier_colors.iter())
        .enumerate()
    {
        if t == tier {
            summary_spans.push(Span::styled(
                format!("[{name}]"),
                Style::default().fg(*c).add_modifier(Modifier::BOLD),
            ));
        } else if t < tier {
            // Already picked — show short model name
            let short = state.routing_models[t]
                .split('/')
                .next_back()
                .unwrap_or(&state.routing_models[t]);
            let display = librefang_types::truncate_str(short, 14);
            summary_spans.push(Span::styled(
                format!("{name}:{display}"),
                Style::default().fg(*c),
            ));
        } else {
            summary_spans.push(Span::styled(*name, theme::dim_style()));
        }
        if t < 2 {
            summary_spans.push(Span::raw("  "));
        }
    }
    f.render_widget(Paragraph::new(Line::from(summary_spans)), chunks[2]);

    // Reuse the same model list as Model step
    let items = build_model_list_items(&state.model_entries, None);
    let list = List::new(items)
        .highlight_style(theme::selected_style())
        .highlight_symbol("\u{25b8} ");
    f.render_stateful_widget(list, chunks[3], &mut state.routing_tier_list);

    f.render_widget(
        widgets::hint_bar("  [\u{2191}\u{2193}/jk] Navigate  [Enter] Select  [Esc] Back"),
        chunks[4],
    );
}

/// Build list items for the model picker (shared between Model and Routing steps).
fn build_model_list_items<'a>(
    entries: &'a [ModelEntry],
    default_id: Option<&str>,
) -> Vec<ListItem<'a>> {
    entries
        .iter()
        .map(|entry| {
            let is_default = default_id.is_some_and(|d| entry.id == d);
            let default_marker = if is_default {
                Span::styled(" *", Style::default().fg(theme::GREEN))
            } else {
                Span::raw("  ")
            };

            let tier_style = match entry.tier {
                "frontier" => Style::default().fg(theme::PURPLE),
                "smart" => Style::default().fg(theme::BLUE),
                "balanced" => Style::default().fg(theme::YELLOW),
                "fast" => Style::default().fg(theme::GREEN),
                "local" => Style::default().fg(theme::TEXT_SECONDARY),
                _ => theme::dim_style(),
            };

            let cost_text = if entry.cost.is_empty() {
                String::new()
            } else {
                format!("  {}", entry.cost)
            };

            ListItem::new(Line::from(vec![
                Span::raw(format!("  {:<32}", entry.display_name)),
                Span::styled(entry.tier, tier_style),
                Span::styled(cost_text, theme::dim_style()),
                default_marker,
            ]))
        })
        .collect()
}

fn draw_complete(f: &mut Frame, area: Rect, state: &mut State) {
    let p = match state.provider() {
        Some(p) => p,
        None => return,
    };

    let default_model = default_model_for_provider(p.name, &state.model_catalog);
    let model = if state.model_input.is_empty() {
        default_model.as_str()
    } else {
        &state.model_input
    };

    let has_desktop = find_desktop_binary().is_some();

    let chunks = Layout::vertical([
        Constraint::Length(1), // 0: spacer
        Constraint::Length(1), // 1: status line
        Constraint::Length(1), // 2: spacer
        Constraint::Length(1), // 3: provider
        Constraint::Length(1), // 4: model
        Constraint::Length(1), // 5: daemon
        Constraint::Length(1), // 6: spacer
        Constraint::Length(1), // 7: separator
        Constraint::Length(1), // 8: spacer
        Constraint::Length(1), // 9: question
        Constraint::Length(1), // 10: spacer
        Constraint::Length(1), // 11: option 1 — Desktop
        Constraint::Length(1), // 12: option 2 — Dashboard
        Constraint::Length(1), // 13: option 3 — Chat
        Constraint::Min(0),    // 14: flex
        Constraint::Length(1), // 15: hints
    ])
    .split(area);

    // ── Status line ──
    if !state.save_error.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{2718} ", Style::default().fg(theme::RED)),
                Span::raw(&state.save_error),
            ])),
            chunks[1],
        );
    } else if state.daemon_started {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::styled(
                    "Setup complete \u{2014} daemon running",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            chunks[1],
        );
    } else if !state.daemon_error.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{26a0} ", Style::default().fg(theme::YELLOW)),
                Span::styled(
                    "Setup complete \u{2014} ",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(&state.daemon_error, Style::default().fg(theme::YELLOW)),
            ])),
            chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(theme::GREEN)),
                Span::styled(
                    "Setup complete!",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            chunks[1],
        );
    }

    // ── Summary KVs ──
    let kv_style = theme::dim_style();
    let val_style = Style::default().fg(theme::TEXT_PRIMARY);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Provider:    ", kv_style),
            Span::styled(p.display, val_style),
        ])),
        chunks[3],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Model:       ", kv_style),
            Span::styled(model, val_style),
        ])),
        chunks[4],
    );

    let daemon_text = if state.daemon_started {
        format!("running at {}", state.daemon_url)
    } else if !state.daemon_error.is_empty() {
        "not running".to_string()
    } else {
        "pending".to_string()
    };
    let daemon_color = if state.daemon_started {
        theme::GREEN
    } else {
        theme::YELLOW
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Daemon:      ", kv_style),
            Span::styled(daemon_text, Style::default().fg(daemon_color)),
        ])),
        chunks[5],
    );

    // ── Separator ──
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  ".to_string() + &"\u{2500}".repeat(area.width.saturating_sub(6) as usize),
            Style::default().fg(theme::BORDER),
        )])),
        chunks[7],
    );

    // ── Question ──
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  How do you want to use LibreFang?",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )])),
        chunks[9],
    );

    // ── Options ──
    let desktop_hint = if has_desktop {
        "native window with system tray"
    } else {
        "not installed"
    };

    let options: [(&str, &str, &str); 3] = [
        ("Desktop app", "(recommended)", desktop_hint),
        ("Web dashboard", "", "opens in your default browser"),
        ("Terminal chat", "", "interactive chat right here"),
    ];

    for (i, (label, badge, desc)) in options.iter().enumerate() {
        let selected = state.complete_list.selected() == Some(i);
        let num = format!("[{}]", i + 1);

        let arrow = if selected {
            Span::styled("  \u{25b8} ", Style::default().fg(theme::ACCENT))
        } else {
            Span::raw("    ")
        };

        let num_style = if selected {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            theme::dim_style()
        };

        let label_style = if i == 0 && !has_desktop {
            // Grey out desktop option if binary not found
            theme::dim_style()
        } else if selected {
            Style::default()
                .fg(theme::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT_PRIMARY)
        };

        let badge_span = if badge.is_empty() {
            Span::raw("")
        } else {
            Span::styled(format!(" {badge}"), Style::default().fg(theme::GREEN))
        };

        let desc_span = if i == 0 && !has_desktop {
            Span::styled(format!("  {desc}"), Style::default().fg(theme::YELLOW))
        } else {
            Span::styled(format!("  {desc}"), theme::dim_style())
        };

        f.render_widget(
            Paragraph::new(Line::from(vec![
                arrow,
                Span::styled(num, num_style),
                Span::raw(" "),
                Span::styled(*label, label_style),
                badge_span,
                desc_span,
            ])),
            chunks[11 + i],
        );
    }

    // ── Bottom hints ──
    f.render_widget(
        widgets::hint_bar("  [\u{2191}\u{2193}/jk] Navigate  [Enter] Launch  [1/2/3] Quick select"),
        chunks[15],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #3629: SaveFailed must not be treated as a successful key state. Pre-fix,
    /// a failed .env write was logged-then-ignored and the wizard auto-advanced
    /// while the key was never on disk.
    #[test]
    fn save_failed_does_not_auto_advance() {
        let s = KeyTestState::SaveFailed("disk full".to_string());
        assert!(
            !matches!(s, KeyTestState::Ok | KeyTestState::Warn),
            "SaveFailed must NOT match the auto-advance arm"
        );
    }

    // ── #3582: pure-helper coverage for the wizard's config-emission path ──

    fn models(fast: &str, balanced: &str, frontier: &str) -> [String; 3] {
        [fast.to_string(), balanced.to_string(), frontier.to_string()]
    }

    #[test]
    fn routing_section_disabled_is_empty() {
        // When the user picks "No" in the routing prompt the wizard must emit
        // no `[default_routing]` block at all — an empty section, not a stub.
        let out = build_routing_section(false, &models("a", "b", "c"));
        assert!(out.is_empty(), "expected empty section, got {out:?}");
    }

    #[test]
    fn routing_section_uses_default_routing_key_not_legacy_routing() {
        // Regression for issue #4466: the wizard previously emitted `[routing]`
        // which is NOT a recognised KernelConfig field, so the user's Smart
        // Router selection was silently ignored. The kernel reads
        // `[default_routing]` as the agent-fallback Smart Router config.
        let out = build_routing_section(true, &models("a", "b", "c"));
        assert!(
            out.contains("[default_routing]"),
            "expected `[default_routing]` header, got {out:?}"
        );
        assert!(
            !out.contains("[routing]"),
            "wizard must not emit the dead `[routing]` key, got {out:?}"
        );
    }

    #[test]
    fn routing_section_enabled_contains_all_three_tiers() {
        let out =
            build_routing_section(true, &models("llama-3-8b", "llama-3-70b", "claude-opus-4"));
        assert!(out.contains("simple_model = \"llama-3-8b\""));
        assert!(out.contains("medium_model = \"llama-3-70b\""));
        assert!(out.contains("complex_model = \"claude-opus-4\""));
    }

    #[test]
    fn routing_section_includes_threshold_defaults() {
        // Threshold values are wizard policy, not user-tunable in the TUI;
        // a regression that drops them would leave routing inert.
        let out = build_routing_section(true, &models("f", "b", "x"));
        assert!(out.contains("simple_threshold = 100"));
        assert!(out.contains("complex_threshold = 500"));
    }

    #[test]
    fn routing_section_starts_with_blank_line_for_template_concat() {
        // The section is concatenated directly after the rendered base config;
        // a leading newline keeps `[routing]` on its own line.
        let out = build_routing_section(true, &models("a", "b", "c"));
        assert!(
            out.starts_with('\n'),
            "expected leading newline, got {out:?}"
        );
    }

    #[test]
    fn api_key_line_empty_for_keyless_provider() {
        // claude-code / local providers have no env_var; the rendered template
        // must not contain a dangling `api_key_env = ""`.
        assert_eq!(build_api_key_line(""), "");
    }

    #[test]
    fn api_key_line_quotes_env_var_name() {
        assert_eq!(
            build_api_key_line("GROQ_API_KEY"),
            "api_key_env = \"GROQ_API_KEY\""
        );
    }

    #[test]
    fn render_init_wizard_config_substitutes_all_placeholders() {
        // If a placeholder survives substitution the user's config.toml is
        // unparseable and the daemon refuses to start. Guard against drift
        // between the template file and the renderer.
        let rendered = render_init_wizard_config(
            "groq",
            "llama-3-70b",
            &build_api_key_line("GROQ_API_KEY"),
            &build_routing_section(false, &models("", "", "")),
        );
        assert!(!rendered.contains("{{provider}}"));
        assert!(!rendered.contains("{{model}}"));
        assert!(!rendered.contains("{{api_key_line}}"));
        assert!(!rendered.contains("{{routing_section}}"));
        assert!(rendered.contains("groq"));
        assert!(rendered.contains("llama-3-70b"));
    }

    #[test]
    fn render_init_wizard_config_inlines_enabled_routing_section() {
        let rendered = render_init_wizard_config(
            "anthropic",
            "claude-opus-4",
            &build_api_key_line("ANTHROPIC_API_KEY"),
            &build_routing_section(true, &models("haiku", "sonnet", "opus")),
        );
        assert!(rendered.contains("[default_routing]"));
        assert!(rendered.contains("simple_model = \"haiku\""));
        assert!(rendered.contains("complex_model = \"opus\""));
    }

    #[test]
    fn default_model_for_unknown_provider_falls_back_to_local() {
        // The catalog lookup may return None for providers that ship no
        // bundled defaults; the wizard must still produce a non-empty model
        // string so the rendered template is valid TOML.
        let catalog = ModelCatalog::default();
        let m = default_model_for_provider("definitely-not-a-provider", &catalog);
        assert!(!m.is_empty());
    }

    #[test]
    fn tier_label_is_total_over_known_tiers() {
        // tier_label is called from the model-list renderer for every catalog
        // entry; a missing arm would surface as "unknown" badges next to real
        // models. Lock the mapping for the tiers the wizard actually displays.
        assert_eq!(tier_label(ModelTier::Frontier), "frontier");
        assert_eq!(tier_label(ModelTier::Smart), "smart");
        assert_eq!(tier_label(ModelTier::Balanced), "balanced");
        assert_eq!(tier_label(ModelTier::Fast), "fast");
        assert_eq!(tier_label(ModelTier::Local), "local");
        assert_eq!(tier_label(ModelTier::Custom), "custom");
    }

    // ── #3582: reducer-style coverage for handle_routing_key ────────────────
    //
    // The maintainer asked for an `Event → State → State` table-test on the
    // wizard's reducer. `handle_routing_key` is the closest the codebase has
    // to a pure reducer today (the migration handler has thread-spawning side
    // effects, the api-key handler does file I/O). We exercise navigation
    // arms only: `Enter` arms call `save_config`, which writes
    // `~/.librefang/config.toml` and would touch the user's filesystem
    // outside a tempdir, so they're deliberately out of scope here.

    /// Construct a minimal `State` for routing-step navigation tests.
    /// Pre-loads three fake model entries so `PickTier` arrow-key cycling
    /// has something to wrap around.
    fn routing_state_with_models(entries: usize) -> State {
        let mut s = State::new();
        s.step = Step::Routing;
        s.routing_phase = RoutingPhase::Choice;
        s.routing_choice_list.select(Some(0));
        s.routing_tier_list.select(Some(0));
        s.model_entries.clear();
        for i in 0..entries {
            s.model_entries.push(ModelEntry {
                id: format!("model-{i}"),
                display_name: format!("Model {i}"),
                tier: "fast",
                cost: String::new(),
            });
        }
        s
    }

    #[test]
    fn routing_choice_down_toggles_to_no() {
        let mut s = routing_state_with_models(3);
        // Selection starts at 0 (Yes); Down/j flips to 1 (No).
        handle_routing_key(&mut s, KeyCode::Down);
        assert_eq!(s.routing_choice_list.selected(), Some(1));
    }

    #[test]
    fn routing_choice_up_toggles_back_to_yes() {
        let mut s = routing_state_with_models(3);
        s.routing_choice_list.select(Some(1));
        handle_routing_key(&mut s, KeyCode::Up);
        assert_eq!(s.routing_choice_list.selected(), Some(0));
    }

    #[test]
    fn routing_choice_j_and_k_match_arrow_keys() {
        // Vim-style bindings must mirror the arrow keys exactly — a regression
        // here would create a confusing two-modes-of-input UX.
        let mut s = routing_state_with_models(3);
        s.routing_choice_list.select(Some(0));
        handle_routing_key(&mut s, KeyCode::Char('j'));
        assert_eq!(s.routing_choice_list.selected(), Some(1));
        handle_routing_key(&mut s, KeyCode::Char('k'));
        assert_eq!(s.routing_choice_list.selected(), Some(0));
    }

    #[test]
    fn routing_choice_esc_returns_to_model_step() {
        // Esc on the first routing screen should let the user back out into
        // the model-picker step, not silently swallow the input.
        let mut s = routing_state_with_models(3);
        s.step = Step::Routing;
        handle_routing_key(&mut s, KeyCode::Esc);
        assert!(
            matches!(s.step, Step::Model),
            "Esc on routing/Choice must go back to Model step"
        );
    }

    #[test]
    fn routing_choice_ignores_unrelated_keys() {
        // Random keys must NOT auto-advance or reset selection.
        let mut s = routing_state_with_models(3);
        s.routing_choice_list.select(Some(1));
        handle_routing_key(&mut s, KeyCode::Char('z'));
        handle_routing_key(&mut s, KeyCode::Tab);
        assert_eq!(s.routing_choice_list.selected(), Some(1));
        assert!(matches!(s.step, Step::Routing));
        assert!(matches!(s.routing_phase, RoutingPhase::Choice));
    }

    #[test]
    fn routing_pick_tier0_esc_returns_to_choice() {
        // Esc on the first tier rolls all the way back to the Yes/No screen.
        let mut s = routing_state_with_models(3);
        s.routing_phase = RoutingPhase::PickTier(0);
        handle_routing_key(&mut s, KeyCode::Esc);
        assert!(matches!(s.routing_phase, RoutingPhase::Choice));
    }

    #[test]
    fn routing_pick_tier1_esc_returns_to_tier0() {
        let mut s = routing_state_with_models(3);
        s.routing_phase = RoutingPhase::PickTier(1);
        handle_routing_key(&mut s, KeyCode::Esc);
        assert!(matches!(s.routing_phase, RoutingPhase::PickTier(0)));
    }

    #[test]
    fn routing_pick_tier2_esc_returns_to_tier1() {
        let mut s = routing_state_with_models(3);
        s.routing_phase = RoutingPhase::PickTier(2);
        handle_routing_key(&mut s, KeyCode::Esc);
        assert!(matches!(s.routing_phase, RoutingPhase::PickTier(1)));
    }

    #[test]
    fn routing_pick_tier_down_cycles_through_models() {
        // Down/j must wrap from the last entry back to 0, otherwise users on
        // small model lists get stuck at the bottom with no way around.
        let mut s = routing_state_with_models(3);
        s.routing_phase = RoutingPhase::PickTier(0);
        s.routing_tier_list.select(Some(0));

        handle_routing_key(&mut s, KeyCode::Down);
        assert_eq!(s.routing_tier_list.selected(), Some(1));
        handle_routing_key(&mut s, KeyCode::Down);
        assert_eq!(s.routing_tier_list.selected(), Some(2));
        handle_routing_key(&mut s, KeyCode::Down);
        assert_eq!(s.routing_tier_list.selected(), Some(0), "must wrap");
    }

    #[test]
    fn routing_pick_tier_up_wraps_from_zero_to_last() {
        let mut s = routing_state_with_models(3);
        s.routing_phase = RoutingPhase::PickTier(0);
        s.routing_tier_list.select(Some(0));

        handle_routing_key(&mut s, KeyCode::Up);
        assert_eq!(
            s.routing_tier_list.selected(),
            Some(2),
            "up at index 0 must wrap to the last entry"
        );
    }

    // ── #3582: step_label / step_index lock-in ──────────────────────────────

    #[test]
    fn step_label_matches_step_index_progression() {
        // The header shows "N of 7"; index and label must stay aligned. A
        // mismatch means the user sees a wrong-step indicator without any
        // compile-time signal.
        let pairs: [(Step, &str, usize, &str); 7] = [
            (Step::Welcome, "1 of 7", 0, "Welcome"),
            (Step::Migration, "2 of 7", 1, "Migration"),
            (Step::Provider, "3 of 7", 2, "Provider"),
            (Step::ApiKey, "4 of 7", 3, "ApiKey"),
            (Step::Model, "5 of 7", 4, "Model"),
            (Step::Routing, "6 of 7", 5, "Routing"),
            (Step::Complete, "7 of 7", 6, "Complete"),
        ];
        for (step, label, idx, name) in pairs {
            let mut s = State::new();
            s.step = step;
            assert_eq!(s.step_label(), label, "label drift for {name}");
            assert_eq!(s.step_index(), idx, "index drift for {name}");
        }
    }

    #[test]
    fn routing_tier_constants_are_three_wide() {
        // The PickTier sub-state is indexed 0..=2; the parallel display
        // arrays must match that or `draw_routing_pick` panics on indexing.
        assert_eq!(ROUTING_TIER_NAMES.len(), 3);
        assert_eq!(ROUTING_TIER_DESC.len(), 3);
    }
}
