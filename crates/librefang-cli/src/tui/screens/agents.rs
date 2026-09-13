//! Agent selection + creation: list running agents, template picker, custom builder.
//! Overhauled with search/filter, state badges, detail view, and new actions.

use crate::templates::{self, AgentTemplate};
use crate::tui::theme;
use crate::tui::widgets;
use librefang_kernel::AgentSubsystemApi;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

/// Available built-in tools for the custom agent builder.
const TOOL_OPTIONS: &[(&str, &str)] = &[
    ("file_read", "tui-agents-tool-file-read-desc"),
    ("file_write", "tui-agents-tool-file-write-desc"),
    ("file_list", "tui-agents-tool-file-list-desc"),
    ("memory_store", "tui-agents-tool-memory-store-desc"),
    ("memory_recall", "tui-agents-tool-memory-recall-desc"),
    ("memory_list", "tui-agents-tool-memory-list-desc"),
    ("web_fetch", "tui-agents-tool-web-fetch-desc"),
    ("shell_exec", "tui-agents-tool-shell-exec-desc"),
    ("agent_send", "tui-agents-tool-agent-send-desc"),
    ("agent_list", "tui-agents-tool-agent-list-desc"),
];

const DEFAULT_TOOLS: &[bool] = &[true, false, true, true, true, true, false, false, false];

#[derive(Clone, PartialEq, Eq)]
pub enum AgentSubScreen {
    /// Pick an existing agent or "create new"
    AgentList,
    /// View agent detail
    AgentDetail,
    /// Pick creation method: template or custom
    CreateMethod,
    /// Pick a template
    TemplatePicker,
    /// Custom builder: name
    CustomName,
    /// Custom builder: description
    CustomDesc,
    /// Custom builder: system prompt
    CustomPrompt,
    /// Custom builder: tool selection
    CustomTools,
    /// Custom builder: skill selection
    CustomSkills,
    /// Custom builder: MCP server selection
    CustomMcpServers,
    /// Edit skills for existing agent
    EditSkills,
    /// Edit MCP servers for existing agent
    EditMcpServers,
    /// Edit the channel allowlist for an existing agent
    EditChannels,
    /// Shared folders editor (`[workspaces]`).
    EditWorkspaces,
    /// Edit model routing (mode, profile allowlist, cost budget) for existing agent
    EditModelRouting,
    /// Edit the inference parameters (temperature, ladders, limits) for an existing agent
    EditModelParams,
    /// Read-only timeline of this agent's recorded manifest snapshots
    ManifestHistory,
    /// Spawning agent (waiting for result)
    Spawning,
}

pub struct AgentSelectState {
    pub sub: AgentSubScreen,
    pub list: ListState,

    // Daemon mode
    pub daemon_agents: Vec<DaemonAgent>,

    // In-process mode
    pub inprocess_agents: Vec<InProcessAgent>,

    // Search/filter
    pub search_active: bool,
    pub search_query: String,
    filtered_indices: Vec<usize>, // indices into combined agent list

    // Detail view
    pub detail: Option<AgentDetail>,

    // Create method
    pub create_method_list: ListState,

    // Template picker
    pub templates: Vec<AgentTemplate>,
    pub template_list: ListState,

    // Custom builder
    pub custom_name: String,
    pub custom_desc: String,
    pub custom_prompt: String,
    pub tool_checks: Vec<bool>,
    pub tool_cursor: usize,

    // Skill/MCP editor (shared by creation wizard + detail editor)
    pub available_skills: Vec<(String, bool)>,
    pub skill_cursor: usize,
    // Shared folders editor: (name, path, mode).
    pub workspaces: Vec<(String, String, String)>,
    pub ws_cursor: usize,
    /// `(row, field)` while a cell is being typed into; 0 = name, 1 = path, 2 = mode.
    pub ws_editing: Option<(usize, u8)>,
    pub ws_buf: String,
    /// Set once `AgentWorkspacesLoaded` has populated `workspaces` for the
    /// agent currently open. While false the editor blocks every key but
    /// `Esc`, so a slow or failed fetch can't be saved over the wrong
    /// agent's manifest (or an empty one).
    pub ws_loaded: bool,
    pub available_mcp: Vec<(String, bool)>,
    pub mcp_cursor: usize,
    // Channel allowlist editor. Detail-only: agent creation writes no `channels`
    // key, which the kernel reads as "every channel", the same default a new
    // agent has always had.
    pub available_channels: Vec<(String, bool)>,
    pub channel_cursor: usize,

    pub token_usage: Option<AgentTokenUsage>,
    // Model routing editor
    /// `"fixed"` or `"flexible"`.
    pub model_mode: String,
    /// The resolved profile catalog with this agent's allowlist applied:
    /// `(profile name, allowed)`. All-unchecked means "any profile".
    pub router_profiles: Vec<(String, bool)>,
    pub router_profile_cursor: usize,
    /// Index into [`COST_BUDGET_OPTIONS`].
    pub cost_budget_idx: usize,
    /// The fallback profile loaded from the agent's stored routing settings.
    /// Not editable from this screen — carried through unchanged on save so
    /// it is not silently cleared (#7781 review).
    pub router_default_profile: Option<String>,
    /// The per-agent router bypass loaded from the agent's stored routing
    /// settings. Not editable from this screen — carried through unchanged
    /// on save for the same reason as `router_default_profile` (#7781
    /// review).
    pub router_fixed: bool,
    /// Set only by `AgentModelRoutingLoaded`. `Enter` in the routing editor
    /// is a no-op while this is `false` — a fetch that failed after `r` was
    /// pressed must not let a save write the reset placeholder values (or,
    /// before the reset, a previous agent's stale ones) onto this agent
    /// (#7781 review).
    pub routing_loaded: bool,

    // Inference-parameter editor (detail view)
    pub model_params: super::model_params::ModelParamsEditor,

    // Manifest version history (detail view, read-only)
    pub manifest_history: Vec<ManifestVersion>,
    pub manifest_history_list: ListState,
    /// A fetch is in flight. Distinguishes "still loading" from "this agent has
    /// no recorded history", which would otherwise render the same empty pane.
    pub manifest_history_loading: bool,
    /// Why the last history fetch produced nothing, when it failed.
    ///
    /// Its own field rather than the shared `status_msg`: that one collects every
    /// agent-tab message, so a skills or channels error arriving while a history
    /// fetch is outstanding would otherwise be rendered as this fetch's reason.
    pub manifest_history_error: Option<String>,

    // Result
    pub spawned_toml: Option<String>,
    pub status_msg: String,
}

#[derive(Clone)]
pub struct DaemonAgent {
    pub id: String,
    pub name: String,
    pub state: String,
    pub provider: String,
    pub model: String,
}

#[derive(Clone)]
pub struct InProcessAgent {
    pub id: librefang_types::agent::AgentId,
    pub name: String,
    pub state: String,
    pub provider: String,
    pub model: String,
}

/// One recorded manifest snapshot, as `GET /api/agents/{id}/manifest-history`
/// returns it. The endpoint is read-only, so there is nothing here to write back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestVersion {
    /// SQLite `datetime('now')` shape: `YYYY-MM-DD HH:MM:SS`, UTC, no offset.
    pub timestamp: String,
    pub change_source: String,
    pub manifest_toml: String,
}

#[derive(Clone, Default)]
pub struct AgentDetail {
    pub id: String,
    pub name: String,
    pub state: String,
    pub model: String,
    pub provider: String,
    pub created: String,
    pub last_active: String,
    pub tags: Vec<String>,
    pub capabilities: Vec<String>,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub skills: Vec<String>,
    pub skills_mode: String,
    pub mcp_servers: Vec<String>,
    pub mcp_servers_mode: String,
    pub channels: Vec<String>,
    pub channels_mode: String,
}

#[derive(Clone, Debug, Default)]
pub struct AgentTokenUsage {
    pub total_tokens: u64,
    pub recent: Vec<(String, u64, u64, f64)>,
}

/// What the agent screen decided.
#[derive(Debug)]
pub enum AgentAction {
    /// Load the agent's `[workspaces]` table for the shared-folders editor.
    FetchAgentWorkspaces(String),
    /// Write the edited shared folders back to the agent's manifest.
    UpdateWorkspaces {
        id: String,
        workspaces: Vec<(String, String, String)>,
    },
    /// No action yet, keep rendering.
    Continue,
    /// User created a new agent manifest (TOML).
    CreatedManifest(String),
    /// User pressed Esc from the top-level list.
    Back,
    /// User wants to chat with a specific agent (from detail view).
    ChatWithAgent {
        id: String,
        name: String,
    },
    /// User wants to kill an agent (from detail view).
    KillAgent(String),
    /// Update skills for an agent.
    UpdateSkills {
        id: String,
        skills: Vec<String>,
    },
    /// Update MCP servers for an agent.
    UpdateMcpServers {
        id: String,
        servers: Vec<String>,
    },
    /// Update the channel allowlist for an agent.
    UpdateChannels {
        id: String,
        channels: Vec<String>,
    },
    /// Fetch skills/mcp data for an agent.
    FetchAgentSkills(String),
    /// Fetch MCP data for an agent.
    FetchAgentMcpServers(String),
    /// Fetch channel allowlist data for an agent.
    FetchAgentChannels(String),
    /// Opened the detail pane for an agent — load the three allowlists it displays.
    ///
    /// Before #7742 the pane rendered `AgentDetail::skills` / `mcp_servers` straight out of
    /// `Default::default()`, because nothing ever wrote them: every agent read as "all skills"
    /// and "no MCP servers" no matter what its manifest said.
    LoadAgentDetail(String),
    FetchAgentTokenUsage(String),
    /// Update an agent's model routing mode and router override.
    UpdateModelRouting {
        id: String,
        /// `"fixed"` or `"flexible"`.
        mode: String,
        /// Empty means "any profile".
        allowed_profiles: Vec<String>,
        /// `None` means "no cap".
        cost_budget: Option<String>,
        /// Not editable from this screen — the value loaded from the
        /// agent's stored settings, carried through unchanged (#7781 review).
        default_profile: Option<String>,
        /// Not editable from this screen — the value loaded from the
        /// agent's stored settings, carried through unchanged (#7781 review).
        fixed: bool,
    },
    /// Fetch an agent's model routing settings and the profile catalog.
    FetchAgentModelRouting(String),
    /// Fetch the agent's current inference parameters before editing them.
    FetchAgentModelParams(String),
    /// Persist edited inference parameters. `None` in a pair clears the agent's
    /// own value so the per-model override supplies it again.
    UpdateModelParams {
        id: String,
        changes: Vec<(String, Option<f64>)>,
    },
    /// Load this agent's recorded manifest snapshots for the history pane.
    FetchManifestHistory(String),
}

/// Cost-budget choices in the model routing editor, cycled with `+` / `-`.
///
/// `(i18n key for the label, wire value)`. The first entry is the no-cap
/// choice, which has no `CostTier`; the rest map onto one. The label is a
/// translation key rather than the display text so the picker is localised.
pub const COST_BUDGET_OPTIONS: &[(&str, Option<&str>)] = &[
    ("tui-agents-label-routing-no-cap", None),
    ("tui-agents-label-routing-cheap", Some("cheap")),
    ("tui-agents-label-routing-medium", Some("medium")),
    ("tui-agents-label-routing-expensive", Some("expensive")),
];

impl AgentSelectState {
    pub fn new() -> Self {
        Self {
            sub: AgentSubScreen::AgentList,
            list: ListState::default(),
            daemon_agents: Vec::new(),
            inprocess_agents: Vec::new(),
            search_active: false,
            search_query: String::new(),
            filtered_indices: Vec::new(),
            detail: None,
            create_method_list: ListState::default(),
            templates: Vec::new(),
            template_list: ListState::default(),
            custom_name: String::new(),
            custom_desc: String::new(),
            custom_prompt: String::new(),
            tool_checks: DEFAULT_TOOLS.to_vec(),
            tool_cursor: 0,
            model_params: super::model_params::ModelParamsEditor::new(),
            manifest_history: Vec::new(),
            manifest_history_list: ListState::default(),
            manifest_history_loading: false,
            manifest_history_error: None,
            available_skills: Vec::new(),
            skill_cursor: 0,
            available_mcp: Vec::new(),
            available_channels: Vec::new(),
            channel_cursor: 0,
            mcp_cursor: 0,
            token_usage: None,
            model_mode: "fixed".to_string(),
            router_profiles: Vec::new(),
            router_profile_cursor: 0,
            cost_budget_idx: 0,
            router_default_profile: None,
            router_fixed: false,
            routing_loaded: false,
            spawned_toml: None,
            status_msg: String::new(),
            workspaces: Vec::new(),
            ws_cursor: 0,
            ws_editing: None,
            ws_buf: String::new(),
            ws_loaded: false,
        }
    }

    pub fn reset(&mut self) {
        self.sub = AgentSubScreen::AgentList;
        self.list.select(Some(0));
        self.create_method_list.select(Some(0));
        self.template_list.select(Some(0));
        self.custom_name.clear();
        self.custom_desc.clear();
        self.custom_prompt.clear();
        self.tool_checks = DEFAULT_TOOLS.to_vec();
        self.tool_cursor = 0;
        self.available_skills.clear();
        self.skill_cursor = 0;
        self.available_mcp.clear();
        self.mcp_cursor = 0;
        self.available_channels.clear();
        self.channel_cursor = 0;
        self.model_mode = "fixed".to_string();
        self.router_profiles.clear();
        self.router_profile_cursor = 0;
        self.cost_budget_idx = 0;
        self.router_default_profile = None;
        self.router_fixed = false;
        self.routing_loaded = false;
        self.manifest_history.clear();
        self.manifest_history_list.select(None);
        self.manifest_history_loading = false;
        self.manifest_history_error = None;
        self.spawned_toml = None;
        self.status_msg.clear();
        self.search_active = false;
        self.search_query.clear();
        self.filtered_indices.clear();
        self.detail = None;
        self.token_usage = None;
    }

    /// Fold a token-usage payload in, if it is still the one being looked at.
    ///
    /// The fetch is two sequential HTTP calls; a selection change in between
    /// leaves the answer describing an agent nobody is looking at any more,
    /// and the panel has nothing on it saying whose numbers these are.
    pub fn apply_token_usage(&mut self, agent_id: &str, usage: AgentTokenUsage) {
        if self.detail.as_ref().is_some_and(|d| d.id == agent_id) {
            self.token_usage = Some(usage);
        }
    }

    /// Load daemon agents from the daemon API.
    pub fn load_daemon_agents(&mut self, base_url: &str) {
        let client = crate::daemon_client();
        if let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                self.daemon_agents.clear();
                // Handle both old format (direct array) and new format ({ "items": [...] })
                let arr = if let Some(arr) = body.as_array() {
                    arr.clone()
                } else if let Some(items) = body.get("items").and_then(|v| v.as_array()) {
                    items.clone()
                } else {
                    Vec::new()
                };
                for a in arr {
                    self.daemon_agents.push(DaemonAgent {
                        id: a["id"].as_str().unwrap_or("?").to_string(),
                        name: a["name"].as_str().unwrap_or("?").to_string(),
                        state: a["state"].as_str().unwrap_or("?").to_string(),
                        provider: a["model_provider"].as_str().unwrap_or("?").to_string(),
                        model: a["model_name"].as_str().unwrap_or("?").to_string(),
                    });
                }
            }
        }
        self.rebuild_filter();
        self.list.select(Some(0));
    }

    /// Load in-process agents from the kernel.
    pub fn load_inprocess_agents(&mut self, kernel: &librefang_kernel::LibreFangKernel) {
        self.inprocess_agents.clear();
        for entry in kernel.agent_registry_ref().list() {
            self.inprocess_agents.push(InProcessAgent {
                id: entry.id,
                name: entry.name.clone(),
                state: format!("{:?}", entry.state),
                provider: entry.manifest.model.provider.clone(),
                model: entry.manifest.model.model.clone(),
            });
        }
        self.rebuild_filter();
        self.list.select(Some(0));
    }

    fn total_agents(&self) -> usize {
        self.daemon_agents.len() + self.inprocess_agents.len()
    }

    /// Visible items: filtered agents + "Create new" item.
    fn visible_count(&self) -> usize {
        if self.search_query.is_empty() {
            self.total_agents() + 1
        } else {
            self.filtered_indices.len() + 1
        }
    }

    fn rebuild_filter(&mut self) {
        self.filtered_indices.clear();
        if self.search_query.is_empty() {
            return;
        }
        let q = self.search_query.to_lowercase();
        let total = self.total_agents();
        for i in 0..total {
            let (name, model, tags) = self.agent_info_at(i);
            if name.to_lowercase().contains(&q)
                || model.to_lowercase().contains(&q)
                || tags.to_lowercase().contains(&q)
            {
                self.filtered_indices.push(i);
            }
        }
    }

    /// Get display info for the agent at combined index.
    fn agent_info_at(&self, combined_idx: usize) -> (String, String, String) {
        let daemon_count = self.daemon_agents.len();
        if combined_idx < daemon_count {
            let a = &self.daemon_agents[combined_idx];
            (
                a.name.clone(),
                format!("{}/{}", a.provider, a.model),
                String::new(),
            )
        } else {
            let local_idx = combined_idx - daemon_count;
            if local_idx < self.inprocess_agents.len() {
                let a = &self.inprocess_agents[local_idx];
                (
                    a.name.clone(),
                    format!("{}/{}", a.provider, a.model),
                    String::new(),
                )
            } else {
                (String::new(), String::new(), String::new())
            }
        }
    }

    /// Map a visible list index to a combined agent index.
    fn visible_to_combined(&self, visible_idx: usize) -> Option<usize> {
        if self.search_query.is_empty() {
            if visible_idx < self.total_agents() {
                Some(visible_idx)
            } else {
                None // "Create new"
            }
        } else if visible_idx < self.filtered_indices.len() {
            Some(self.filtered_indices[visible_idx])
        } else {
            None // "Create new"
        }
    }

    fn load_templates(&mut self) {
        if self.templates.is_empty() {
            self.templates = templates::load_all_templates();
        }
        self.template_list.select(Some(0));
    }

    /// Build detail from daemon agent.
    fn build_detail_daemon(&self, idx: usize) -> AgentDetail {
        let a = &self.daemon_agents[idx];
        AgentDetail {
            id: a.id.clone(),
            name: a.name.clone(),
            state: a.state.clone(),
            model: a.model.clone(),
            provider: a.provider.clone(),
            ..Default::default()
        }
    }

    /// Build detail from in-process agent.
    fn build_detail_inprocess(&self, idx: usize) -> AgentDetail {
        let a = &self.inprocess_agents[idx];
        AgentDetail {
            id: format!("{}", a.id),
            name: a.name.clone(),
            state: a.state.clone(),
            model: a.model.clone(),
            provider: a.provider.clone(),
            ..Default::default()
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AgentAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return AgentAction::Back;
        }

        match self.sub {
            AgentSubScreen::AgentList => self.handle_agent_list(key),
            AgentSubScreen::AgentDetail => self.handle_detail(key),
            AgentSubScreen::EditModelParams => self.handle_edit_model_params(key),
            AgentSubScreen::ManifestHistory => self.handle_manifest_history(key),
            AgentSubScreen::CreateMethod => self.handle_create_method(key),
            AgentSubScreen::TemplatePicker => self.handle_template_picker(key),
            AgentSubScreen::CustomName => self.handle_custom_name(key),
            AgentSubScreen::CustomDesc => self.handle_custom_desc(key),
            AgentSubScreen::CustomPrompt => self.handle_custom_prompt(key),
            AgentSubScreen::CustomTools => self.handle_custom_tools(key),
            AgentSubScreen::CustomSkills => self.handle_custom_skills(key),
            AgentSubScreen::CustomMcpServers => self.handle_custom_mcp_servers(key),
            AgentSubScreen::EditSkills => self.handle_edit_skills(key),
            AgentSubScreen::EditMcpServers => self.handle_edit_mcp_servers(key),
            AgentSubScreen::EditChannels => self.handle_edit_channels(key),
            AgentSubScreen::EditWorkspaces => self.handle_edit_workspaces(key),
            AgentSubScreen::EditModelRouting => self.handle_edit_model_routing(key),
            AgentSubScreen::Spawning => AgentAction::Continue,
        }
    }

    fn handle_agent_list(&mut self, key: KeyEvent) -> AgentAction {
        // Search mode input
        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.rebuild_filter();
                    self.list.select(Some(0));
                    return AgentAction::Continue;
                }
                KeyCode::Enter => {
                    self.search_active = false;
                    return AgentAction::Continue;
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.rebuild_filter();
                    self.list.select(Some(0));
                    return AgentAction::Continue;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.rebuild_filter();
                    self.list.select(Some(0));
                    return AgentAction::Continue;
                }
                _ => return AgentAction::Continue,
            }
        }

        let total = self.visible_count();
        if total == 0 {
            return AgentAction::Continue;
        }

        match key.code {
            KeyCode::Esc => return AgentAction::Back,
            KeyCode::Char('/') => {
                self.search_active = true;
                self.search_query.clear();
                return AgentAction::Continue;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.list.selected().unwrap_or(0);
                let next = if i == 0 { total - 1 } else { i - 1 };
                self.list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.list.selected().unwrap_or(0);
                let next = (i + 1) % total;
                self.list.select(Some(next));
            }
            KeyCode::Enter => {
                if let Some(vis_idx) = self.list.selected() {
                    match self.visible_to_combined(vis_idx) {
                        Some(combined) => {
                            // Open detail view
                            let daemon_count = self.daemon_agents.len();
                            if combined < daemon_count {
                                self.detail = Some(self.build_detail_daemon(combined));
                            } else {
                                let local = combined - daemon_count;
                                if local < self.inprocess_agents.len() {
                                    self.detail = Some(self.build_detail_inprocess(local));
                                }
                            }
                            // The figures on screen belong to the agent that
                            // was open; leaving them up shows one agent's
                            // token count and cost under another's name until
                            // this one's own fetch returns.
                            self.token_usage = None;
                            self.sub = AgentSubScreen::AgentDetail;
                            if let Some(ref detail) = self.detail {
                                return AgentAction::LoadAgentDetail(detail.id.clone());
                            }
                        }
                        None => {
                            // "Create new"
                            self.create_method_list.select(Some(0));
                            self.sub = AgentSubScreen::CreateMethod;
                        }
                    }
                }
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_detail(&mut self, key: KeyEvent) -> AgentAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentList;
            }
            KeyCode::Char('c') => {
                // Chat with this agent
                if let Some(ref detail) = self.detail {
                    return AgentAction::ChatWithAgent {
                        id: detail.id.clone(),
                        name: detail.name.clone(),
                    };
                }
            }
            KeyCode::Char('k') => {
                // Kill this agent
                if let Some(ref detail) = self.detail {
                    return AgentAction::KillAgent(detail.id.clone());
                }
            }
            KeyCode::Char('s') => {
                // Edit skills for this agent
                if let Some(ref detail) = self.detail {
                    let id = detail.id.clone();
                    self.sub = AgentSubScreen::EditSkills;
                    return AgentAction::FetchAgentSkills(id);
                }
            }
            KeyCode::Char('w') => {
                // Edit shared folders for this agent. Reset the editor's
                // state rather than opening it optimistically over
                // whatever the last agent (or edit) left behind — the fetch
                // is async, and a stale row, cursor, or in-progress edit
                // surviving into this agent's editor is how a save ends up
                // PATCHing the wrong manifest (#7835).
                if let Some(ref detail) = self.detail {
                    let id = detail.id.clone();
                    self.workspaces = Vec::new();
                    self.ws_cursor = 0;
                    self.ws_editing = None;
                    self.ws_buf.clear();
                    self.ws_loaded = false;
                    self.sub = AgentSubScreen::EditWorkspaces;
                    return AgentAction::FetchAgentWorkspaces(id);
                }
            }
            KeyCode::Char('m') => {
                // Edit MCP servers for this agent
                if let Some(ref detail) = self.detail {
                    let id = detail.id.clone();
                    self.sub = AgentSubScreen::EditMcpServers;
                    return AgentAction::FetchAgentMcpServers(id);
                }
            }
            KeyCode::Char('n') => {
                // Edit the channel allowlist for this agent
                if let Some(ref detail) = self.detail {
                    let id = detail.id.clone();
                    self.sub = AgentSubScreen::EditChannels;
                    return AgentAction::FetchAgentChannels(id);
                }
            }
            KeyCode::Char('$') => {
                if let Some(ref detail) = self.detail {
                    return AgentAction::FetchAgentTokenUsage(detail.id.clone());
                }
            }
            KeyCode::Char('r') => {
                // Edit model routing for this agent. Reset the editor state
                // up front: if the fetch below fails (FetchError instead of
                // AgentModelRoutingLoaded), a stale value left over from
                // whatever agent was edited last must not get written onto
                // this one on Enter (#7781 review).
                if let Some(ref detail) = self.detail {
                    let id = detail.id.clone();
                    self.model_mode = "fixed".to_string();
                    self.router_profiles.clear();
                    self.router_profile_cursor = 0;
                    self.cost_budget_idx = 0;
                    self.router_default_profile = None;
                    self.router_fixed = false;
                    self.routing_loaded = false;
                    self.sub = AgentSubScreen::EditModelRouting;
                    return AgentAction::FetchAgentModelRouting(id);
                }
            }
            KeyCode::Char('p') => {
                // Edit this agent's inference parameters
                if let Some(ref detail) = self.detail {
                    let id = detail.id.clone();
                    self.sub = AgentSubScreen::EditModelParams;
                    return AgentAction::FetchAgentModelParams(id);
                }
            }
            KeyCode::Char('h') => {
                // Read-only manifest version history for this agent
                if let Some(ref detail) = self.detail {
                    let id = detail.id.clone();
                    self.manifest_history.clear();
                    self.manifest_history_list.select(None);
                    self.manifest_history_loading = true;
                    // Cleared so any reason the pane shows afterwards belongs to
                    // this fetch and not to an earlier one.
                    self.manifest_history_error = None;
                    self.sub = AgentSubScreen::ManifestHistory;
                    return AgentAction::FetchManifestHistory(id);
                }
            }
            _ => {}
        }
        AgentAction::Continue
    }

    /// Key handling for the manifest history pane.
    ///
    /// Navigation only — the endpoint records snapshots and offers no restore,
    /// so there is nothing here that writes.
    fn handle_manifest_history(&mut self, key: KeyEvent) -> AgentAction {
        let len = self.manifest_history.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentDetail;
            }
            // An empty history has no cursor to move, and `% 0` would panic.
            KeyCode::Up | KeyCode::Char('k') if len > 0 => {
                let i = self.manifest_history_list.selected().unwrap_or(0);
                let next = if i == 0 { len - 1 } else { i - 1 };
                self.manifest_history_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 => {
                let i = self.manifest_history_list.selected().unwrap_or(0);
                self.manifest_history_list.select(Some((i + 1) % len));
            }
            _ => {}
        }
        AgentAction::Continue
    }

    /// Record the snapshots a fetch returned and put the cursor on the newest.
    pub fn set_manifest_history(&mut self, versions: Vec<ManifestVersion>) {
        self.manifest_history_loading = false;
        self.manifest_history_error = None;
        self.manifest_history_list
            .select((!versions.is_empty()).then_some(0));
        self.manifest_history = versions;
    }

    /// Record why a history fetch produced nothing, so the pane says that rather
    /// than reporting the agent has no recorded history.
    pub fn set_manifest_history_error(&mut self, message: String) {
        self.manifest_history_loading = false;
        self.manifest_history_error = Some(message);
    }

    /// Whether a history response for `agent_id` belongs to the agent now open.
    ///
    /// A slow response for agent A can land after the operator has moved to agent
    /// B and asked for its history; this pane renders a whole `agent.toml`, so
    /// showing A's under B's header actively misleads rather than merely lagging.
    pub fn manifest_history_is_for(&self, agent_id: &str) -> bool {
        self.detail.as_ref().is_some_and(|d| d.id == agent_id)
    }

    /// Key handling for the inference-parameter editor.
    ///
    /// While a custom value is being typed the editor swallows navigation keys
    /// — otherwise `j` in "0.5j" would move the cursor instead of being
    /// rejected as a non-numeric character.
    fn handle_edit_model_params(&mut self, key: KeyEvent) -> AgentAction {
        if self.model_params.custom_buffer().is_some() {
            match key.code {
                KeyCode::Esc => self.model_params.cancel_custom(),
                KeyCode::Backspace => self.model_params.pop_custom_char(),
                KeyCode::Enter => match self.model_params.commit_custom() {
                    Ok(()) => self.model_params.status.clear(),
                    Err(e) => self.model_params.status = e,
                },
                KeyCode::Char(c) => self.model_params.push_custom_char(c),
                _ => {}
            }
            return AgentAction::Continue;
        }
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentDetail;
            }
            KeyCode::Up | KeyCode::Char('k') => self.model_params.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.model_params.move_cursor(1),
            KeyCode::Left | KeyCode::Char('h') => self.model_params.step(-1),
            KeyCode::Right | KeyCode::Char('l') => self.model_params.step(1),
            KeyCode::Char('i') => self.model_params.set_inherit(),
            KeyCode::Char('e') => self.model_params.begin_custom(),
            KeyCode::Enter => {
                let changes: Vec<(String, Option<f64>)> = self
                    .model_params
                    .changes()
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect();
                self.sub = AgentSubScreen::AgentDetail;
                if !changes.is_empty() {
                    if let Some(ref detail) = self.detail {
                        return AgentAction::UpdateModelParams {
                            id: detail.id.clone(),
                            changes,
                        };
                    }
                }
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_create_method(&mut self, key: KeyEvent) -> AgentAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentList;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.create_method_list.selected().unwrap_or(0);
                self.create_method_list
                    .select(Some(if i == 0 { 1 } else { 0 }));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.create_method_list.selected().unwrap_or(0);
                self.create_method_list
                    .select(Some(if i == 0 { 1 } else { 0 }));
            }
            KeyCode::Enter => {
                match self.create_method_list.selected() {
                    Some(0) => {
                        self.load_templates();
                        if self.templates.is_empty() {
                            // No templates, go straight to custom
                            self.custom_name.clear();
                            self.sub = AgentSubScreen::CustomName;
                        } else {
                            self.sub = AgentSubScreen::TemplatePicker;
                        }
                    }
                    Some(1) => {
                        self.custom_name.clear();
                        self.custom_desc.clear();
                        self.custom_prompt.clear();
                        self.tool_checks = DEFAULT_TOOLS.to_vec();
                        self.tool_cursor = 0;
                        self.sub = AgentSubScreen::CustomName;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_template_picker(&mut self, key: KeyEvent) -> AgentAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::CreateMethod;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.template_list.selected().unwrap_or(0);
                let total = self.templates.len();
                let next = if i == 0 {
                    total.saturating_sub(1)
                } else {
                    i - 1
                };
                self.template_list.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.template_list.selected().unwrap_or(0);
                let next = (i + 1) % self.templates.len().max(1);
                self.template_list.select(Some(next));
            }
            KeyCode::Enter => {
                if let Some(idx) = self.template_list.selected() {
                    if idx < self.templates.len() {
                        let toml = self.templates[idx].content.clone();
                        return AgentAction::CreatedManifest(toml);
                    }
                }
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_custom_name(&mut self, key: KeyEvent) -> AgentAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::CreateMethod;
            }
            KeyCode::Enter if !self.custom_name.is_empty() => {
                if self.custom_desc.is_empty() {
                    self.custom_desc = crate::i18n::t_args(
                        "tui-agents-default-desc",
                        &[("name", &self.custom_name)],
                    );
                }
                self.sub = AgentSubScreen::CustomDesc;
            }
            KeyCode::Char(c) => {
                self.custom_name.push(c);
            }
            KeyCode::Backspace => {
                self.custom_name.pop();
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_custom_desc(&mut self, key: KeyEvent) -> AgentAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::CustomName;
            }
            KeyCode::Enter => {
                if self.custom_prompt.is_empty() {
                    self.custom_prompt = crate::i18n::t_args(
                        "tui-agents-default-prompt",
                        &[("name", &self.custom_name)],
                    );
                }
                self.sub = AgentSubScreen::CustomPrompt;
            }
            KeyCode::Char(c) => {
                self.custom_desc.push(c);
            }
            KeyCode::Backspace => {
                self.custom_desc.pop();
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_custom_prompt(&mut self, key: KeyEvent) -> AgentAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::CustomDesc;
            }
            KeyCode::Enter => {
                self.sub = AgentSubScreen::CustomTools;
            }
            KeyCode::Char(c) => {
                self.custom_prompt.push(c);
            }
            KeyCode::Backspace => {
                self.custom_prompt.pop();
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_custom_tools(&mut self, key: KeyEvent) -> AgentAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::CustomPrompt;
            }
            KeyCode::Up | KeyCode::Char('k') if self.tool_cursor > 0 => {
                self.tool_cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if self.tool_cursor < TOOL_OPTIONS.len() - 1 => {
                self.tool_cursor += 1;
            }
            KeyCode::Char(' ') => {
                self.tool_checks[self.tool_cursor] = !self.tool_checks[self.tool_cursor];
            }
            KeyCode::Enter => {
                // Advance to skill selection (populate with all unchecked = "all skills" mode)
                if self.available_skills.is_empty() {
                    // Pre-populate on first entry (will be empty until backend fills it)
                    // Default: all unchecked = use all skills
                }
                self.skill_cursor = 0;
                self.sub = AgentSubScreen::CustomSkills;
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_custom_skills(&mut self, key: KeyEvent) -> AgentAction {
        let len = self.available_skills.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::CustomTools;
            }
            KeyCode::Up | KeyCode::Char('k') if self.skill_cursor > 0 => {
                self.skill_cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 && self.skill_cursor < len - 1 => {
                self.skill_cursor += 1;
            }
            KeyCode::Char(' ') if len > 0 => {
                let checked = &mut self.available_skills[self.skill_cursor].1;
                *checked = !*checked;
            }
            KeyCode::Enter => {
                // Advance to MCP server selection
                self.mcp_cursor = 0;
                self.sub = AgentSubScreen::CustomMcpServers;
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_custom_mcp_servers(&mut self, key: KeyEvent) -> AgentAction {
        let len = self.available_mcp.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::CustomSkills;
            }
            KeyCode::Up | KeyCode::Char('k') if self.mcp_cursor > 0 => {
                self.mcp_cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 && self.mcp_cursor < len - 1 => {
                self.mcp_cursor += 1;
            }
            KeyCode::Char(' ') if len > 0 => {
                let checked = &mut self.available_mcp[self.mcp_cursor].1;
                *checked = !*checked;
            }
            KeyCode::Enter => {
                let toml = self.build_custom_toml();
                return AgentAction::CreatedManifest(toml);
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_edit_skills(&mut self, key: KeyEvent) -> AgentAction {
        let len = self.available_skills.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentDetail;
            }
            KeyCode::Up | KeyCode::Char('k') if self.skill_cursor > 0 => {
                self.skill_cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 && self.skill_cursor < len - 1 => {
                self.skill_cursor += 1;
            }
            KeyCode::Char(' ') if len > 0 => {
                let checked = &mut self.available_skills[self.skill_cursor].1;
                *checked = !*checked;
            }
            KeyCode::Enter => {
                // Save — collect checked skill names (none checked = "all")
                if let Some(ref detail) = self.detail {
                    let skills: Vec<String> = self
                        .available_skills
                        .iter()
                        .filter(|(_, checked)| *checked)
                        .map(|(name, _)| name.clone())
                        .collect();
                    return AgentAction::UpdateSkills {
                        id: detail.id.clone(),
                        skills,
                    };
                }
                self.sub = AgentSubScreen::AgentDetail;
            }
            _ => {}
        }
        AgentAction::Continue
    }

    /// The value currently held by one of a row's three fields, used to
    /// seed `ws_buf` when opening it for editing so committing without
    /// retyping keeps the field instead of blanking it.
    fn workspace_field(entry: &(String, String, String), field: u8) -> String {
        match field {
            0 => entry.0.clone(),
            1 => entry.1.clone(),
            _ => entry.2.clone(),
        }
    }

    /// Recognizes both the canonical spelling `GET /api/agents/{id}/manifest`
    /// renders (`readonly` / `readwrite`) and the deserialize-only
    /// `WorkspaceMode` aliases (`r`, `rw`, ... — `librefang-types/src/agent.rs`)
    /// a user might type from habit. Returns `None` for anything else so the
    /// caller can keep the previous value instead of defaulting to the more
    /// permissive mode.
    fn parse_mode_input(v: &str) -> Option<&'static str> {
        match v {
            "r" | "read" | "read-only" | "readonly" => Some("readonly"),
            "rw" | "read-write" | "readwrite" => Some("readwrite"),
            _ => None,
        }
    }

    fn handle_edit_workspaces(&mut self, key: KeyEvent) -> AgentAction {
        if let Some((row, field)) = self.ws_editing {
            match key.code {
                KeyCode::Esc => {
                    self.ws_editing = None;
                    self.ws_buf.clear();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    // Commit the buffer to the field, then move on.
                    let v = self.ws_buf.clone();
                    if let Some(entry) = self.workspaces.get_mut(row) {
                        match field {
                            0 => entry.0 = v,
                            1 => entry.1 = v,
                            // Unrecognized input keeps the previous mode
                            // rather than escalating a read-only folder to
                            // read-write by default.
                            _ => {
                                if let Some(m) = Self::parse_mode_input(&v) {
                                    entry.2 = m.to_string();
                                }
                            }
                        }
                    }
                    self.ws_buf.clear();
                    if field == 2 {
                        self.ws_editing = None;
                    } else {
                        self.ws_editing = Some((row, field + 1));
                        self.ws_buf = self
                            .workspaces
                            .get(row)
                            .map(|e| Self::workspace_field(e, field + 1))
                            .unwrap_or_default();
                    }
                }
                KeyCode::Backspace => {
                    self.ws_buf.pop();
                }
                KeyCode::Char(c) => self.ws_buf.push(c),
                _ => {}
            }
            return AgentAction::Continue;
        }

        // The fetch this editor depends on hasn't landed yet (or failed —
        // `FetchError` never sets `ws_loaded`). Only `Esc` works: adding,
        // deleting or saving now would act on an editor `workspaces` never
        // populated for this agent.
        if !self.ws_loaded {
            if key.code == KeyCode::Esc {
                self.sub = AgentSubScreen::AgentDetail;
            }
            return AgentAction::Continue;
        }

        let len = self.workspaces.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentDetail;
            }
            KeyCode::Up | KeyCode::Char('k') if self.ws_cursor > 0 => self.ws_cursor -= 1,
            KeyCode::Down | KeyCode::Char('j') if len > 0 && self.ws_cursor < len - 1 => {
                self.ws_cursor += 1;
            }
            KeyCode::Char('a') => {
                self.workspaces
                    .push(("".into(), "".into(), "readwrite".into()));
                self.ws_cursor = self.workspaces.len() - 1;
                self.ws_editing = Some((self.ws_cursor, 0));
            }
            KeyCode::Char('d') if len > 0 => {
                self.workspaces.remove(self.ws_cursor);
                if self.ws_cursor >= self.workspaces.len() && self.ws_cursor > 0 {
                    self.ws_cursor -= 1;
                }
            }
            KeyCode::Enter if len > 0 => {
                self.ws_editing = Some((self.ws_cursor, 0));
                self.ws_buf = self
                    .workspaces
                    .get(self.ws_cursor)
                    .map(|e| Self::workspace_field(e, 0))
                    .unwrap_or_default();
            }
            KeyCode::Char('s') => {
                if let Some(ref detail) = self.detail {
                    // A row with an empty name or path is an abandoned edit,
                    // not a declaration: sending it would write a broken
                    // `[workspaces]` entry the kernel then fails to resolve.
                    let total = self.workspaces.len();
                    let entries: Vec<(String, String, String)> = self
                        .workspaces
                        .iter()
                        .filter(|(n, p, _)| !n.trim().is_empty() && !p.trim().is_empty())
                        .map(|(n, p, m)| (n.trim().to_string(), p.trim().to_string(), m.clone()))
                        .collect();
                    if entries.len() < total {
                        self.status_msg = crate::i18n::t("tui-agents-workspaces-row-dropped");
                    }
                    return AgentAction::UpdateWorkspaces {
                        id: detail.id.clone(),
                        workspaces: entries,
                    };
                }
                self.sub = AgentSubScreen::AgentDetail;
            }
            _ => {}
        }
        AgentAction::Continue
    }

    /// Model routing editor.
    ///
    /// `Tab` flips fixed <-> flexible, `Space` toggles a profile in the
    /// allowlist, `+` / `-` cycle the cost budget, `Enter` saves.
    ///
    /// Enter always emits [`AgentAction::UpdateModelRouting`] — including for
    /// `fixed`, which is how an operator turns routing back off. Returning to
    /// the detail screen without emitting would silently discard the edit.
    fn handle_edit_model_routing(&mut self, key: KeyEvent) -> AgentAction {
        let profile_count = self.router_profiles.len();
        let flexible = self.model_mode == "flexible";
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentDetail;
            }
            KeyCode::Tab => {
                self.model_mode = if flexible { "fixed" } else { "flexible" }.to_string();
            }
            KeyCode::Up | KeyCode::Char('k') if flexible && self.router_profile_cursor > 0 => {
                self.router_profile_cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j')
                if flexible
                    && profile_count > 0
                    && self.router_profile_cursor < profile_count - 1 =>
            {
                self.router_profile_cursor += 1;
            }
            KeyCode::Char(' ') if flexible && profile_count > 0 => {
                let checked = &mut self.router_profiles[self.router_profile_cursor].1;
                *checked = !*checked;
            }
            KeyCode::Char('+') | KeyCode::Char('=') if flexible => {
                self.cost_budget_idx = (self.cost_budget_idx + 1) % COST_BUDGET_OPTIONS.len();
            }
            KeyCode::Char('-') if flexible => {
                self.cost_budget_idx = if self.cost_budget_idx == 0 {
                    COST_BUDGET_OPTIONS.len() - 1
                } else {
                    self.cost_budget_idx - 1
                };
            }
            KeyCode::Enter => {
                // The fetch that populates this screen may still be in
                // flight or may have failed (`AppEvent::FetchError` only
                // writes a status message; it does not leave the editor).
                // Saving before `AgentModelRoutingLoaded` ever arrived would
                // write this screen's reset placeholder values over the
                // agent's real settings (#7781 review).
                // The two cases are indistinguishable from in here, and a
                // failed fetch cannot be retried from inside the editor —
                // so the message names both and points at the way out
                // (Esc, then `r`, which re-dispatches the fetch) instead of
                // repeating "still loading" forever at an operator whose
                // fetch is never coming back.
                if !self.routing_loaded {
                    self.status_msg = crate::i18n::t("tui-agents-model-routing-not-loaded");
                    return AgentAction::Continue;
                }
                if let Some(ref detail) = self.detail {
                    // In fixed mode the allowlist and budget describe a routing
                    // decision that will not happen, so they are not sent — the
                    // server clears the override wholesale.
                    let (allowed_profiles, cost_budget) = if flexible {
                        (
                            self.router_profiles
                                .iter()
                                .filter(|(_, checked)| *checked)
                                .map(|(name, _)| name.clone())
                                .collect(),
                            COST_BUDGET_OPTIONS[self.cost_budget_idx]
                                .1
                                .map(str::to_string),
                        )
                    } else {
                        (Vec::new(), None)
                    };
                    return AgentAction::UpdateModelRouting {
                        id: detail.id.clone(),
                        mode: self.model_mode.clone(),
                        allowed_profiles,
                        cost_budget,
                        default_profile: self.router_default_profile.clone(),
                        fixed: self.router_fixed,
                    };
                }
                self.sub = AgentSubScreen::AgentDetail;
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn handle_edit_mcp_servers(&mut self, key: KeyEvent) -> AgentAction {
        let len = self.available_mcp.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentDetail;
            }
            KeyCode::Up | KeyCode::Char('k') if self.mcp_cursor > 0 => {
                self.mcp_cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 && self.mcp_cursor < len - 1 => {
                self.mcp_cursor += 1;
            }
            KeyCode::Char(' ') if len > 0 => {
                let checked = &mut self.available_mcp[self.mcp_cursor].1;
                *checked = !*checked;
            }
            KeyCode::Enter => {
                // Save — collect checked server names (none checked = "all")
                if let Some(ref detail) = self.detail {
                    let servers: Vec<String> = self
                        .available_mcp
                        .iter()
                        .filter(|(_, checked)| *checked)
                        .map(|(name, _)| name.clone())
                        .collect();
                    return AgentAction::UpdateMcpServers {
                        id: detail.id.clone(),
                        servers,
                    };
                }
                self.sub = AgentSubScreen::AgentDetail;
            }
            _ => {}
        }
        AgentAction::Continue
    }

    /// Channel allowlist editor for an existing agent.
    ///
    /// Same shape as `handle_edit_mcp_servers`, with one difference that matters: an empty
    /// selection here means "every configured channel", not "none" (`AgentManifest::channels`
    /// documents empty as the backward-compatible all-channels default), so saving with nothing
    /// checked is a widening rather than a lockout.
    fn handle_edit_channels(&mut self, key: KeyEvent) -> AgentAction {
        let len = self.available_channels.len();
        match key.code {
            KeyCode::Esc => {
                self.sub = AgentSubScreen::AgentDetail;
            }
            KeyCode::Up | KeyCode::Char('k') if self.channel_cursor > 0 => {
                self.channel_cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 && self.channel_cursor < len - 1 => {
                self.channel_cursor += 1;
            }
            KeyCode::Char(' ') if len > 0 => {
                let checked = &mut self.available_channels[self.channel_cursor].1;
                *checked = !*checked;
            }
            KeyCode::Enter => {
                if let Some(ref detail) = self.detail {
                    let channels: Vec<String> = self
                        .available_channels
                        .iter()
                        .filter(|(_, checked)| *checked)
                        .map(|(name, _)| name.clone())
                        .collect();
                    return AgentAction::UpdateChannels {
                        id: detail.id.clone(),
                        channels,
                    };
                }
                self.sub = AgentSubScreen::AgentDetail;
            }
            _ => {}
        }
        AgentAction::Continue
    }

    fn build_custom_toml(&self) -> String {
        let tools_str: String = TOOL_OPTIONS
            .iter()
            .zip(self.tool_checks.iter())
            .filter(|(_, &checked)| checked)
            .map(|((name, _), _)| format!("\"{}\"", name))
            .collect::<Vec<_>>()
            .join(", ");

        let selected_skills: Vec<String> = self
            .available_skills
            .iter()
            .filter(|(_, checked)| *checked)
            .map(|(name, _)| format!("\"{}\"", name))
            .collect();
        let skills_str = selected_skills.join(", ");

        let selected_mcp: Vec<String> = self
            .available_mcp
            .iter()
            .filter(|(_, checked)| *checked)
            .map(|(name, _)| format!("\"{}\"", name))
            .collect();
        let mcp_str = selected_mcp.join(", ");

        format!(
            r#"name = "{name}"
version = "{version}"
description = "{desc}"
author = "user"
module = "builtin:chat"
tags = ["custom"]
skills = [{skills_str}]
mcp_servers = [{mcp_str}]

[model]
max_tokens = 8192
temperature = 0.5
system_prompt = """{prompt}"""

[resources]
# 0 = unlimited hourly LLM-token budget; set a positive value to rate-limit.
max_llm_tokens_per_hour = 0

[capabilities]
tools = [{tools_str}]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            name = self.custom_name,
            version = librefang_types::VERSION,
            desc = self.custom_desc,
            prompt = self.custom_prompt,
        )
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Render the agent screen.
pub fn draw(f: &mut Frame, area: Rect, state: &mut AgentSelectState) {
    // Clear background
    f.render_widget(Block::default(), area);

    match state.sub {
        AgentSubScreen::AgentDetail => {
            draw_detail(f, area, state);
            return;
        }
        AgentSubScreen::AgentList => {
            draw_agent_list_full(f, area, state);
            return;
        }
        AgentSubScreen::EditSkills
        | AgentSubScreen::EditMcpServers
        | AgentSubScreen::EditChannels => {
            draw_edit_allowlist(f, area, state);
            return;
        }
        AgentSubScreen::EditWorkspaces => {
            draw_edit_workspaces(f, area, state);
            return;
        }
        AgentSubScreen::EditModelRouting => {
            draw_edit_model_routing(f, area, state);
            return;
        }
        AgentSubScreen::EditModelParams => {
            draw_edit_model_params(f, area, state);
            return;
        }
        AgentSubScreen::ManifestHistory => {
            draw_manifest_history(f, area, state);
            return;
        }
        _ => {}
    }

    let sub_title = match state.sub {
        AgentSubScreen::AgentList
        | AgentSubScreen::AgentDetail
        | AgentSubScreen::EditSkills
        | AgentSubScreen::EditMcpServers
        | AgentSubScreen::EditChannels
        | AgentSubScreen::EditWorkspaces
        | AgentSubScreen::EditModelRouting
        | AgentSubScreen::EditModelParams
        | AgentSubScreen::ManifestHistory => unreachable!(),
        AgentSubScreen::CreateMethod => crate::i18n::t("tui-agents-title-create-method"),
        AgentSubScreen::TemplatePicker => crate::i18n::t("tui-agents-title-templates"),
        AgentSubScreen::CustomName => crate::i18n::t("tui-agents-title-custom-name"),
        AgentSubScreen::CustomDesc => crate::i18n::t("tui-agents-title-custom-desc"),
        AgentSubScreen::CustomPrompt => crate::i18n::t("tui-agents-title-custom-prompt"),
        AgentSubScreen::CustomTools => crate::i18n::t("tui-agents-title-custom-tools"),
        AgentSubScreen::CustomSkills => crate::i18n::t("tui-agents-title-custom-skills"),
        AgentSubScreen::CustomMcpServers => crate::i18n::t("tui-agents-title-custom-mcp"),
        AgentSubScreen::Spawning => crate::i18n::t("tui-agents-title-spawning"),
    };

    // Center a card
    let card_h = 18u16.min(area.height);
    let card_w = 64u16.min(area.width.saturating_sub(2));
    let [card_area] = Layout::horizontal([Constraint::Length(card_w)])
        .flex(Flex::Center)
        .areas(area);
    let [card_area] = Layout::vertical([Constraint::Length(card_h)])
        .flex(Flex::Center)
        .areas(card_area);

    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" {sub_title} "),
            theme::title_style(),
        )]))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));

    let inner = block.inner(card_area);
    f.render_widget(block, card_area);

    match state.sub {
        AgentSubScreen::CreateMethod => draw_create_method(f, inner, state),
        AgentSubScreen::TemplatePicker => draw_template_picker(f, inner, state),
        AgentSubScreen::CustomName => draw_text_input(
            f,
            inner,
            &crate::i18n::t("tui-agents-prompt-name"),
            &state.custom_name,
            &crate::i18n::t("tui-agents-placeholder-name"),
        ),
        AgentSubScreen::CustomDesc => draw_text_input(
            f,
            inner,
            &crate::i18n::t("tui-agents-prompt-desc"),
            &state.custom_desc,
            &crate::i18n::t("tui-agents-placeholder-desc"),
        ),
        AgentSubScreen::CustomPrompt => draw_text_input(
            f,
            inner,
            &crate::i18n::t("tui-agents-prompt-prompt"),
            &state.custom_prompt,
            &crate::i18n::t("tui-agents-placeholder-prompt"),
        ),
        AgentSubScreen::CustomTools => draw_tool_select(f, inner, state),
        AgentSubScreen::CustomSkills => draw_skill_select(f, inner, state),
        AgentSubScreen::CustomMcpServers => draw_mcp_select(f, inner, state),
        AgentSubScreen::Spawning => {
            let msg = Paragraph::new(Line::from(vec![Span::styled(
                crate::i18n::t("tui-agents-prompt-spawning"),
                theme::dim_style(),
            )]));
            f.render_widget(msg, inner);
        }
        _ => {}
    }
}

/// Full-area agent list with table layout and search bar.
fn draw_agent_list_full(f: &mut Frame, area: Rect, state: &mut AgentSelectState) {
    let inner = widgets::render_screen_block(f, area, &crate::i18n::t("tui-agents-title-screen"));

    let has_search = state.search_active || !state.search_query.is_empty();
    let search_height = if has_search { 1 } else { 0 };

    let chunks = Layout::vertical([
        Constraint::Length(search_height), // search bar
        Constraint::Length(2),             // table header
        Constraint::Min(3),                // list
        Constraint::Length(1),             // hints
    ])
    .split(inner);

    // ── Search bar ──────────────────────────────────────────────────────────
    if has_search {
        let cursor = if state.search_active { "\u{2588}" } else { "" };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  / ", Style::default().fg(theme::YELLOW)),
                Span::styled(&state.search_query, theme::input_style()),
                Span::styled(
                    cursor,
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ])),
            chunks[0],
        );
    }

    // ── Table header ────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<5} {:<18} {:<24} {}",
                crate::i18n::t("tui-agents-header-state"),
                crate::i18n::t("tui-agents-header-name"),
                crate::i18n::t("tui-agents-header-model"),
                crate::i18n::t("tui-agents-header-id")
            ),
            theme::table_header(),
        )])),
        chunks[1],
    );

    // ── Agent list ──────────────────────────────────────────────────────────
    let daemon_count = state.daemon_agents.len();
    let use_filter = !state.search_query.is_empty();

    let agent_indices: Vec<usize> = if use_filter {
        state.filtered_indices.clone()
    } else {
        (0..state.total_agents()).collect()
    };

    let mut items: Vec<ListItem> = agent_indices
        .iter()
        .map(|&combined| {
            if combined < daemon_count {
                let a = &state.daemon_agents[combined];
                let (badge, badge_style) = theme::state_badge(&a.state);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {:<5}", badge), badge_style),
                    Span::styled(
                        format!(" {:<18}", widgets::truncate(&a.name, 17)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(
                            " {:<24}",
                            widgets::truncate(&format!("{}/{}", a.provider, a.model), 23)
                        ),
                        Style::default().fg(theme::YELLOW),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&a.id, 12)),
                        theme::dim_style(),
                    ),
                ]))
            } else {
                let local = combined - daemon_count;
                let a = &state.inprocess_agents[local];
                let (badge, badge_style) = theme::state_badge(&a.state);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {:<5}", badge), badge_style),
                    Span::styled(
                        format!(" {:<18}", widgets::truncate(&a.name, 17)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(
                        format!(
                            " {:<24}",
                            widgets::truncate(&format!("{}/{}", a.provider, a.model), 23)
                        ),
                        Style::default().fg(theme::YELLOW),
                    ),
                    Span::styled(
                        format!(" {}", widgets::truncate(&format!("{}", a.id), 12)),
                        theme::dim_style(),
                    ),
                ]))
            }
        })
        .collect();

    items.push(ListItem::new(Line::from(vec![
        Span::styled("  \u{2795} ", Style::default().fg(theme::ACCENT)),
        Span::styled(
            crate::i18n::t("tui-agents-opt-create-new"),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
    ])));

    let list = widgets::themed_list(items);

    f.render_stateful_widget(list, chunks[2], &mut state.list);

    // ── Status message ──────────────────────────────────────────────────────
    if !state.status_msg.is_empty() {
        let msg_area = Rect {
            x: chunks[2].x,
            y: chunks[2].y + chunks[2].height.saturating_sub(1),
            width: chunks[2].width,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::YELLOW),
            )),
            msg_area,
        );
    }

    // ── Hints ───────────────────────────────────────────────────────────────
    let hints = if state.search_active {
        crate::i18n::t("tui-agents-hints-filter")
    } else {
        crate::i18n::t("tui-agents-hints-list")
    };
    f.render_widget(widgets::hint_bar(&hints), chunks[3]);
}

/// Draw agent detail view.
fn draw_detail(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    let inner = widgets::render_screen_block(f, area, &crate::i18n::t("tui-agents-title-detail"));

    let chunks = Layout::vertical([
        Constraint::Min(10),   // detail
        Constraint::Length(1), // hints
    ])
    .split(inner);

    match &state.detail {
        Some(detail) => {
            let (badge, badge_style) = theme::state_badge(&detail.state);
            let mut lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-id"), Style::default()),
                    Span::styled(&detail.id, theme::dim_style()),
                ]),
                Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-name"), Style::default()),
                    Span::styled(
                        &detail.name,
                        Style::default()
                            .fg(theme::CYAN)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-state"), Style::default()),
                    Span::styled(badge, badge_style),
                    Span::styled(format!(" ({})", detail.state), theme::dim_style()),
                ]),
                Line::from(vec![
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-provider"),
                        Style::default(),
                    ),
                    Span::styled(&detail.provider, Style::default().fg(theme::YELLOW)),
                ]),
                Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-model"), Style::default()),
                    Span::styled(&detail.model, Style::default().fg(theme::YELLOW)),
                ]),
            ];

            // Rendered right under the fixed header (not appended at the end) so it
            // cannot be pushed past the bottom of the pane by the variable-length
            // sections below it — the Paragraph here has no scroll offset.
            if let Some(usage) = &state.token_usage {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    crate::i18n::t("tui-agents-detail-tokens"),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {} {}",
                        crate::i18n::t("tui-agents-detail-tokens-injected"),
                        usage.total_tokens
                    ),
                    Style::default().fg(theme::TEXT_SECONDARY),
                )));
                for (model, input, output, cost) in usage.recent.iter().take(5) {
                    lines.push(Line::from(Span::styled(
                        format!("    {model:<20} {input}/{output}  ${cost:.4}"),
                        Style::default().fg(theme::TEXT_TERTIARY),
                    )));
                }
            }

            if !detail.created.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-created"),
                        Style::default(),
                    ),
                    Span::styled(&detail.created, theme::dim_style()),
                ]));
            }
            if !detail.last_active.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-active"), Style::default()),
                    Span::styled(&detail.last_active, theme::dim_style()),
                ]));
            }
            if !detail.tags.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-tags"), Style::default()),
                    Span::styled(detail.tags.join(", "), Style::default().fg(theme::CYAN)),
                ]));
            }
            if !detail.capabilities.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-caps"), Style::default()),
                    Span::styled(
                        detail.capabilities.join(", "),
                        Style::default().fg(theme::YELLOW),
                    ),
                ]));
            }
            if let Some(ref parent) = detail.parent {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-parent"), Style::default()),
                    Span::styled(parent, theme::dim_style()),
                ]));
            }
            if !detail.children.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-children"),
                        Style::default(),
                    ),
                    Span::styled(detail.children.join(", "), theme::dim_style()),
                ]));
            }

            // Skills section
            lines.push(Line::from(""));
            if detail.skills.is_empty() || detail.skills_mode == "all" {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-skills"), Style::default()),
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-all-skills"),
                        Style::default().fg(theme::GREEN),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-skills"), Style::default()),
                    Span::styled(detail.skills.join(", "), Style::default().fg(theme::CYAN)),
                ]));
            }

            // MCP section (#5855: empty allowlist = no servers, `["*"]` = all)
            if detail.mcp_servers.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-mcp"), Style::default()),
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-none"),
                        Style::default().fg(theme::DIM),
                    ),
                ]));
            } else if detail.mcp_servers_mode == "all" {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-mcp"), Style::default()),
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-all-servers"),
                        Style::default().fg(theme::GREEN),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(crate::i18n::t("tui-agents-detail-mcp"), Style::default()),
                    Span::styled(
                        detail.mcp_servers.join(", "),
                        Style::default().fg(theme::CYAN),
                    ),
                ]));
            }

            // Channel allowlist (#7742). Empty is the all-channels default, the
            // opposite of the MCP list directly above it.
            if detail.channels.is_empty() || detail.channels_mode == "all" {
                lines.push(Line::from(vec![
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-channels"),
                        Style::default(),
                    ),
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-all-channels"),
                        Style::default().fg(theme::GREEN),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        crate::i18n::t("tui-agents-detail-channels"),
                        Style::default(),
                    ),
                    Span::styled(detail.channels.join(", "), Style::default().fg(theme::CYAN)),
                ]));
            }

            f.render_widget(Paragraph::new(lines), chunks[0]);
        }
        None => {
            f.render_widget(
                widgets::empty_state(&crate::i18n::t("tui-agents-label-no-agent-selected")),
                chunks[0],
            );
        }
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-agents-hints-detail")),
        chunks[1],
    );
}

fn draw_create_method(f: &mut Frame, area: Rect, state: &mut AgentSelectState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    let prompt = Paragraph::new(crate::i18n::t("tui-agents-prompt-create-method"));
    f.render_widget(prompt, chunks[0]);

    let items = vec![
        ListItem::new(Line::from(vec![
            Span::styled(crate::i18n::t("tui-agents-opt-templates"), Style::default()),
            Span::styled(
                crate::i18n::t("tui-agents-opt-templates-desc"),
                theme::dim_style(),
            ),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled(crate::i18n::t("tui-agents-opt-custom"), Style::default()),
            Span::styled(
                crate::i18n::t("tui-agents-opt-custom-desc"),
                theme::dim_style(),
            ),
        ])),
    ];

    let list = widgets::themed_list(items);

    f.render_stateful_widget(list, chunks[1], &mut state.create_method_list);

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-agents-hints-navigate")),
        chunks[2],
    );
}

fn draw_template_picker(f: &mut Frame, area: Rect, state: &mut AgentSelectState) {
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);

    let items: Vec<ListItem> = state
        .templates
        .iter()
        .map(|t| {
            let hint = templates::template_display_hint(t);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {:<20}", t.name),
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(hint, theme::dim_style()),
            ]))
        })
        .collect();

    let list = widgets::themed_list(items);

    f.render_stateful_widget(list, chunks[0], &mut state.template_list);

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-agents-hints-navigate")),
        chunks[1],
    );
}

fn draw_text_input(f: &mut Frame, area: Rect, label: &str, value: &str, placeholder: &str) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let prompt = Paragraph::new(format!("  {label}"));
    f.render_widget(prompt, chunks[0]);

    let display = if value.is_empty() { placeholder } else { value };
    let style = if value.is_empty() {
        theme::dim_style()
    } else {
        theme::input_style()
    };

    let input = Paragraph::new(Line::from(vec![
        Span::raw("  > "),
        Span::styled(display, style),
        Span::styled(
            "\u{2588}",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]));
    f.render_widget(input, chunks[1]);

    if value.is_empty() {
        let hint_text = crate::i18n::t_args(
            "tui-agents-label-placeholder",
            &[("placeholder", placeholder)],
        );
        let hint = Paragraph::new(Line::from(vec![Span::styled(
            hint_text,
            theme::dim_style(),
        )]));
        f.render_widget(hint, chunks[2]);
    }

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-agents-hints-input")),
        chunks[4],
    );
}

fn draw_tool_select(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    let prompt = Paragraph::new(crate::i18n::t("tui-agents-prompt-tools"));
    f.render_widget(prompt, chunks[0]);

    let items: Vec<ListItem> = TOOL_OPTIONS
        .iter()
        .zip(state.tool_checks.iter())
        .enumerate()
        .map(|(i, ((name, desc), &checked))| {
            let check = if checked { "\u{25c9}" } else { "\u{25cb}" };
            let highlight = if i == state.tool_cursor {
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {}{:<16}", check, name), highlight),
                Span::styled(crate::i18n::t(desc), theme::dim_style()),
            ]))
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, chunks[1]);

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-agents-hints-tools")),
        chunks[2],
    );
}

fn draw_skill_select(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    draw_checkbox_list(
        f,
        area,
        &crate::i18n::t("tui-agents-prompt-skills"),
        &state.available_skills,
        state.skill_cursor,
        &crate::i18n::t("tui-agents-hints-skills"),
    );
}

fn draw_mcp_select(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    draw_checkbox_list(
        f,
        area,
        &crate::i18n::t("tui-agents-prompt-mcp"),
        &state.available_mcp,
        state.mcp_cursor,
        &crate::i18n::t("tui-agents-hints-mcp"),
    );
}

fn draw_edit_workspaces(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        crate::i18n::t("tui-agents-workspaces-help"),
        Style::default().fg(theme::TEXT_SECONDARY),
    ))];
    for (i, (name, path, mode)) in state.workspaces.iter().enumerate() {
        let marker = if i == state.ws_cursor { ">" } else { " " };
        let mut spans = vec![Span::styled(
            format!("{} {:<16} {:<28} {}", marker, name, path, mode),
            if i == state.ws_cursor {
                Style::default().fg(theme::ACCENT)
            } else {
                Style::default()
            },
        )];
        if let Some((row, field)) = state.ws_editing {
            if row == i {
                spans.push(Span::styled(
                    format!("  [{}]", ["name", "path", "mode"][field as usize]),
                    Style::default().fg(theme::ACCENT),
                ));
                spans.push(Span::styled(
                    format!(" {}", state.ws_buf),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    if state.workspaces.is_empty() {
        let key = if state.ws_loaded {
            "tui-agents-workspaces-empty"
        } else {
            "tui-agents-workspaces-loading"
        };
        lines.push(Line::from(crate::i18n::t(key)));
    }
    // A failed fetch, a rejected save (duplicate name), or a dropped
    // half-typed row all land here via `status_msg` — without this the
    // operator saw nothing distinguish a reject from a save (#7835).
    if !state.status_msg.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status_msg.clone(),
            Style::default().fg(theme::YELLOW),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_edit_allowlist(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    let (title, items, cursor) = match state.sub {
        AgentSubScreen::EditSkills => (
            crate::i18n::t("tui-agents-title-custom-skills"),
            &state.available_skills,
            state.skill_cursor,
        ),
        AgentSubScreen::EditMcpServers => (
            crate::i18n::t("tui-agents-title-custom-mcp"),
            &state.available_mcp,
            state.mcp_cursor,
        ),
        AgentSubScreen::EditChannels => (
            crate::i18n::t("tui-agents-title-edit-channels"),
            &state.available_channels,
            state.channel_cursor,
        ),
        _ => return,
    };

    // The channel editor gets its own prompt because its empty state inverts the other two:
    // checking nothing grants every channel instead of revoking them.
    let prompt = if state.sub == AgentSubScreen::EditChannels {
        crate::i18n::t("tui-agents-prompt-edit-channels")
    } else {
        crate::i18n::t("tui-agents-prompt-edit-skills")
    };

    let inner = widgets::render_screen_block(f, area, title.trim());

    draw_checkbox_list(
        f,
        inner,
        &prompt,
        items,
        cursor,
        &crate::i18n::t("tui-agents-hints-save"),
    );
}

/// Model routing editor: mode, profile allowlist, cost budget.
///
/// Labels are the human-readable names an operator recognises; the wire
/// values (`fixed` / `flexible`, `cheap` / `medium` / `expensive`) are shown
/// in the value column rather than as the label itself.
fn draw_edit_model_routing(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        crate::i18n::t("tui-agents-title-model-routing").trim(),
    );

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(inner);

    let flexible = state.model_mode == "flexible";
    let mode_label = if flexible {
        crate::i18n::t("tui-agents-label-routing-flexible")
    } else {
        crate::i18n::t("tui-agents-label-routing-fixed")
    };
    // Label and value are joined inside the Fluent message rather than by a
    // format literal here, so a locale can reorder or re-punctuate the line.
    let mode_line = crate::i18n::t_args("tui-agents-line-routing-mode", &[("mode", &mode_label)]);
    f.render_widget(
        Paragraph::new(format!(
            "{}\n{}",
            mode_line,
            crate::i18n::t("tui-agents-hint-routing-mode"),
        )),
        chunks[0],
    );

    if !flexible {
        // Nothing below applies while the agent is pinned to its own model;
        // showing a disabled picker would imply the values still matter.
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-agents-label-routing-fixed-explainer")),
            chunks[1],
        );
    } else if state.router_profiles.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-agents-label-no-router-profiles")),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .router_profiles
            .iter()
            .enumerate()
            .map(|(i, (name, checked))| {
                let check = if *checked { "\u{25c9}" } else { "\u{25cb}" };
                let style = if i == state.router_profile_cursor {
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(format!("  {check} {name}")).style(style)
            })
            .collect();
        f.render_widget(List::new(items), chunks[1]);
    }

    let budget_label = crate::i18n::t(COST_BUDGET_OPTIONS[state.cost_budget_idx].0);
    let allowed = state
        .router_profiles
        .iter()
        .filter(|(_, checked)| *checked)
        .count();
    let allowlist_summary = if !flexible {
        String::new()
    } else if allowed == 0 {
        crate::i18n::t("tui-agents-label-routing-any-profile")
    } else {
        format!("{allowed}")
    };
    f.render_widget(
        Paragraph::new(crate::i18n::t_args(
            "tui-agents-line-routing-summary",
            &[("budget", &budget_label), ("allowed", &allowlist_summary)],
        )),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(crate::i18n::t("tui-agents-hints-model-routing"))
            .style(Style::default().fg(theme::DIM)),
        chunks[3],
    );
}

/// Width of the label column in the inference-parameter editor, in cells.
const LABEL_COLUMN_WIDTH: usize = 22;

/// The {24d8} info glyph the TUI convention puts in front of a field hint.
const INFO_ICON: char = '\u{24d8}';

/// Render the inference-parameter editor.
///
/// Every row shows the resolved-looking value or the word `inherit`, so "this
/// agent has no opinion" reads as a state rather than as a blank. The ladder
/// fields also show which rungs exist, which is the whole reason for replacing
/// the free slider: the useful values are a short list, not a continuum.
fn draw_edit_model_params(f: &mut Frame, area: Rect, state: &AgentSelectState) {
    use super::model_params::FIELDS;

    let inner = widgets::render_screen_block(
        f,
        area,
        crate::i18n::t("tui-agents-title-model-params").trim(),
    );
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(format!(
            "  {}",
            crate::i18n::t("tui-agents-prompt-model-params")
        )),
        chunks[0],
    );

    let editor = &state.model_params;
    let rows: Vec<ListItem> = FIELDS
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let selected = i == editor.cursor();
            let value = match (selected, editor.custom_buffer()) {
                (true, Some(buf)) => format!("{buf}\u{2588}"),
                _ => editor.display(i),
            };
            let style = if selected {
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let marker = if editor.value(i).is_none() {
                "\u{25cb}"
            } else {
                "\u{25c9}"
            };
            // Assembled by pushes rather than a format literal: the layout
            // string would otherwise read as untranslated user-facing text to
            // the i18n scanner, which cannot tell padding from prose.
            let mut cell = String::from("  ");
            cell.push_str(marker);
            cell.push(' ');
            cell.push_str(&field.label());
            while cell.chars().count() < LABEL_COLUMN_WIDTH {
                cell.push(' ');
            }
            ListItem::new(Line::from(vec![
                Span::styled(cell, style),
                Span::styled(value, style),
            ]))
        })
        .collect();
    f.render_widget(List::new(rows), chunks[1]);

    let mut hint_line = String::from("  ");
    hint_line.push(INFO_ICON);
    hint_line.push(' ');
    hint_line.push_str(&FIELDS[editor.cursor()].hint());
    f.render_widget(
        Paragraph::new(hint_line).style(Style::default().fg(theme::DIM)),
        chunks[2],
    );

    let hints = if editor.custom_buffer().is_some() {
        crate::i18n::t("tui-agents-hints-model-params-custom")
    } else if editor.status.is_empty() {
        crate::i18n::t("tui-agents-hints-model-params")
    } else {
        editor.status.clone()
    };
    f.render_widget(widgets::hint_bar(&hints), chunks[3]);
}

/// Render `manifest_versions.timestamp` the way the dashboard does.
///
/// The column is defaulted to SQLite's `datetime('now')`, which stores
/// `YYYY-MM-DD HH:MM:SS` in UTC carrying no offset. Read as-is it would show a
/// UTC instant as if it were local, so it is parsed as UTC and converted; a
/// value in any other shape is shown verbatim rather than as an error string,
/// which is what `formatSqliteDateTime` in the dashboard also does.
fn format_manifest_timestamp(raw: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .map(|naive| {
            naive
                .and_utc()
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| raw.to_string())
}

/// Render the read-only manifest version timeline.
///
/// Left: one row per snapshot, newest first. Right: the selected snapshot's
/// full TOML. An agent that has never been persisted, and a fetch that is still
/// in flight, each get their own line — an empty pane on its own would not say
/// which of the two happened.
fn draw_manifest_history(f: &mut Frame, area: Rect, state: &mut AgentSelectState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        crate::i18n::t("tui-agents-title-manifest-history").trim(),
    );

    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(inner);

    if state.manifest_history.is_empty() {
        // A failed fetch writes `manifest_history_error` and clears the loading
        // flag, so the pane says why it is empty instead of claiming the agent has
        // no recorded history. Reading the shared `status_msg` here instead would
        // show whatever unrelated agent-tab message happened to arrive first.
        let message = if state.manifest_history_loading {
            crate::i18n::t("tui-agents-label-manifest-history-loading")
        } else if let Some(reason) = state.manifest_history_error.clone() {
            reason
        } else {
            crate::i18n::t("tui-agents-label-manifest-history-empty")
        };
        f.render_widget(widgets::empty_state(&message), chunks[0]);
        f.render_widget(
            widgets::hint_bar(&crate::i18n::t("tui-agents-hints-manifest-history")),
            chunks[1],
        );
        return;
    }

    let panes = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[0]);

    let items: Vec<ListItem> = state
        .manifest_history
        .iter()
        .map(|v| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {:<21}", format_manifest_timestamp(&v.timestamp)),
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(widgets::truncate(&v.change_source, 18), theme::dim_style()),
            ]))
        })
        .collect();
    f.render_stateful_widget(
        widgets::themed_list(items),
        panes[0],
        &mut state.manifest_history_list,
    );

    // `selected()` can outlive its row when a refresh returns fewer snapshots,
    // so the index is looked up rather than indexed into.
    let toml = state
        .manifest_history_list
        .selected()
        .and_then(|i| state.manifest_history.get(i))
        .map(|v| v.manifest_toml.as_str())
        .unwrap_or_default();
    f.render_widget(
        Paragraph::new(toml)
            .style(theme::dim_style())
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(theme::DIM))
                    .padding(Padding::horizontal(1)),
            ),
        panes[1],
    );

    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-agents-hints-manifest-history")),
        chunks[1],
    );
}

fn draw_checkbox_list(
    f: &mut Frame,
    area: Rect,
    prompt_text: &str,
    items: &[(String, bool)],
    cursor: usize,
    hints_text: &str,
) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    let prompt = Paragraph::new(format!("  {prompt_text}"));
    f.render_widget(prompt, chunks[0]);

    if items.is_empty() {
        f.render_widget(
            widgets::empty_state(&crate::i18n::t("tui-agents-label-none-available")),
            chunks[1],
        );
    } else {
        let list_items: Vec<ListItem> = items
            .iter()
            .enumerate()
            .map(|(i, (name, checked))| {
                let check = if *checked { "\u{25c9}" } else { "\u{25cb}" };
                let highlight = if i == cursor {
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![Span::styled(
                    format!("  {check} {name}"),
                    highlight,
                )]))
            })
            .collect();

        let list = List::new(list_items);
        f.render_widget(list, chunks[1]);
    }

    f.render_widget(widgets::hint_bar(hints_text), chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(timestamp: &str) -> ManifestVersion {
        ManifestVersion {
            timestamp: timestamp.to_string(),
            change_source: "api".to_string(),
            manifest_toml: format!("name = \"a\"\n# {timestamp}\n"),
        }
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Draw the screen into an off-screen buffer and return it as text, so a
    /// test can assert what the pane actually shows.
    fn render(state: &mut AgentSelectState) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).unwrap();
        terminal
            .draw(|f| draw(f, f.area(), state))
            .unwrap()
            .buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn history_key_opens_the_pane_and_asks_for_the_agents_versions() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::AgentDetail;
        state.detail = Some(AgentDetail {
            id: "11111111-2222-3333-4444-555555555555".to_string(),
            ..Default::default()
        });
        state.status_msg = "stale message".to_string();
        state.manifest_history_error = Some("an earlier fetch failed".to_string());

        let action = state.handle_key(press(KeyCode::Char('h')));

        assert!(state.sub == AgentSubScreen::ManifestHistory);
        assert!(state.manifest_history_loading);
        assert_eq!(
            state.manifest_history_error, None,
            "an earlier status must not be mistaken for this fetch's error"
        );
        // `status_msg` is deliberately left alone: it is the agent list's message
        // line, not this pane's, and the pane no longer reads it. Clearing it here
        // would silently drop an unrelated agent-tab message the operator has not
        // seen yet.
        assert_eq!(state.status_msg, "stale message");
        match action {
            AgentAction::FetchManifestHistory(id) => {
                assert_eq!(id, "11111111-2222-3333-4444-555555555555");
            }
            _ => panic!("expected a manifest-history fetch"),
        }
    }

    #[test]
    fn loaded_versions_populate_the_list_and_select_the_newest() {
        let mut state = AgentSelectState::new();
        state.manifest_history_loading = true;

        state.set_manifest_history(vec![
            version("2026-09-07 10:00:00"),
            version("2026-09-06 09:00:00"),
        ]);

        assert!(!state.manifest_history_loading);
        assert_eq!(state.manifest_history.len(), 2);
        assert_eq!(state.manifest_history_list.selected(), Some(0));
    }

    #[test]
    fn arrows_move_through_the_versions_and_wrap() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;
        state.set_manifest_history(vec![
            version("2026-09-07 10:00:00"),
            version("2026-09-06 09:00:00"),
        ]);

        state.handle_key(press(KeyCode::Down));
        assert_eq!(state.manifest_history_list.selected(), Some(1));
        state.handle_key(press(KeyCode::Down));
        assert_eq!(state.manifest_history_list.selected(), Some(0));
        state.handle_key(press(KeyCode::Up));
        assert_eq!(state.manifest_history_list.selected(), Some(1));
    }

    /// The endpoint answers an agent that was never persisted with `[]`, and
    /// `limit=0` clamps to 1 rather than erroring — either way the pane can be
    /// asked to navigate an empty list, where a wrapping `% len` would panic.
    #[test]
    fn navigating_an_empty_history_neither_panics_nor_selects() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;
        state.set_manifest_history(Vec::new());

        state.handle_key(press(KeyCode::Down));
        state.handle_key(press(KeyCode::Up));
        state.handle_key(press(KeyCode::Char('j')));
        state.handle_key(press(KeyCode::Char('k')));

        assert_eq!(state.manifest_history_list.selected(), None);
        assert!(state.sub == AgentSubScreen::ManifestHistory);
    }

    #[test]
    fn esc_returns_from_the_history_pane_to_the_detail_pane() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;

        state.handle_key(press(KeyCode::Esc));

        assert!(state.sub == AgentSubScreen::AgentDetail);
    }

    #[test]
    fn an_empty_history_renders_the_empty_state_not_a_blank_pane() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;
        state.set_manifest_history(Vec::new());

        let rendered = render(&mut state);

        assert!(
            rendered.contains(&crate::i18n::t("tui-agents-label-manifest-history-empty")),
            "empty history must say so:\n{rendered}"
        );
    }

    #[test]
    fn a_fetch_still_in_flight_says_loading_rather_than_no_history() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;
        state.manifest_history_loading = true;

        let rendered = render(&mut state);

        assert!(
            rendered.contains(&crate::i18n::t("tui-agents-label-manifest-history-loading")),
            "an in-flight fetch must not claim the agent has no history:\n{rendered}"
        );
    }

    /// A rejected id (400 for a non-UUID, 404 for one the registry does not
    /// know) arrives as a history failure; the pane must show it instead of the
    /// "no changes recorded" line, which would be a different and wrong answer.
    #[test]
    fn a_rejected_agent_id_shows_the_daemons_reason_not_the_empty_state() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;
        state.set_manifest_history_error("Agent not found".to_string());

        let rendered = render(&mut state);

        assert!(
            rendered.contains("Agent not found"),
            "the failure reason must reach the pane:\n{rendered}"
        );
        assert!(
            !rendered.contains(&crate::i18n::t("tui-agents-label-manifest-history-empty")),
            "a failed fetch must not read as an agent with no history:\n{rendered}"
        );
    }

    /// `status_msg` collects every agent-tab message. A skills or channels error
    /// arriving while a history fetch is outstanding used to be rendered here as
    /// this fetch's reason, so an operator read an unrelated failure where "no
    /// configuration changes recorded" belonged.
    #[test]
    fn an_unrelated_agent_tab_error_is_not_shown_as_the_history_fetchs_reason() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;
        state.set_manifest_history(Vec::new());
        state.status_msg = "Failed to save skills".to_string();

        let rendered = render(&mut state);

        assert!(
            !rendered.contains("Failed to save skills"),
            "an unrelated agent-tab error must not stand in for the history fetch's reason:\n{rendered}"
        );
        assert!(
            rendered.contains(&crate::i18n::t("tui-agents-label-manifest-history-empty")),
            "an agent with no recorded history must still say so:\n{rendered}"
        );
    }

    #[test]
    fn a_populated_history_renders_its_versions_and_the_selected_toml() {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::ManifestHistory;
        state.set_manifest_history(vec![version("2026-09-07 10:00:00")]);

        let rendered = render(&mut state);

        assert!(
            rendered.contains("api"),
            "the change source belongs on the row:\n{rendered}"
        );
        assert!(
            rendered.contains("2026-09-07"),
            "the version's timestamp belongs on the row:\n{rendered}"
        );
    }

    /// `manifest_versions.timestamp` is stored in SQLite's `datetime('now')`
    /// shape — UTC with no offset — which is the interpretation the dashboard's
    /// `formatSqliteDateTime` also applies.
    #[test]
    fn a_utc_timestamp_is_rendered_in_local_time() {
        use chrono::TimeZone;

        let formatted = format_manifest_timestamp("2026-09-07 10:00:00");
        let expected = chrono::Utc
            .with_ymd_and_hms(2026, 9, 7, 10, 0, 0)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        assert_eq!(formatted, expected);
    }

    #[test]
    fn an_unparseable_timestamp_is_shown_verbatim() {
        assert_eq!(format_manifest_timestamp("not a date"), "not a date");
        assert_eq!(format_manifest_timestamp(""), "");
    }

    #[test]
    fn custom_agent_template_has_unlimited_hourly_token_budget() {
        let toml = AgentSelectState::new().build_custom_toml();
        assert!(
            toml.contains("max_llm_tokens_per_hour = 0"),
            "template must emit an unlimited (0) hourly token budget:\n{toml}"
        );
        assert!(
            !toml.contains("max_llm_tokens_per_hour = 200000"),
            "template must not re-introduce the 200000 hourly cap"
        );
    }

    #[test]
    fn dollar_key_requests_the_footprint_for_the_open_agent_only() {
        let mut state = AgentSelectState::new();
        // No detail open: the key must not fire a request against nothing.
        assert!(matches!(
            state.handle_detail(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE)),
            AgentAction::Continue
        ));
        state.detail = Some(AgentDetail {
            id: "agent-7".to_string(),
            ..AgentDetail::default()
        });
        match state.handle_detail(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE)) {
            AgentAction::FetchAgentTokenUsage(id) => assert_eq!(id, "agent-7"),
            _ => panic!("the open agent's id must be the one fetched"),
        }
    }

    fn usage(total: u64) -> AgentTokenUsage {
        AgentTokenUsage {
            total_tokens: total,
            recent: Vec::new(),
        }
    }

    /// Two sequential HTTP calls back one `$`, so a selection change lands
    /// between the request and the answer whenever the daemon is slow.
    #[test]
    fn a_footprint_for_another_agent_is_ignored() {
        let mut state = AgentSelectState::new();
        state.detail = Some(AgentDetail {
            id: "agent-2".to_string(),
            ..AgentDetail::default()
        });

        state.apply_token_usage("agent-1", usage(9_999));

        assert!(
            state.token_usage.is_none(),
            "agent A's figures must not render under agent B's name"
        );

        state.apply_token_usage("agent-2", usage(42));

        assert_eq!(
            state.token_usage.as_ref().map(|u| u.total_tokens),
            Some(42),
            "the open agent's own answer must still land"
        );
    }

    /// Without this the previous agent's numbers stay on screen until this
    /// agent's own fetch returns — on every selection change, race or no race.
    #[test]
    fn opening_another_agent_drops_the_previous_footprint() {
        let mut state = AgentSelectState::new();
        state.daemon_agents = vec![
            DaemonAgent {
                id: "agent-1".to_string(),
                name: "a".to_string(),
                state: "running".to_string(),
                provider: String::new(),
                model: String::new(),
            },
            DaemonAgent {
                id: "agent-2".to_string(),
                name: "b".to_string(),
                state: "running".to_string(),
                provider: String::new(),
                model: String::new(),
            },
        ];
        state.list.select(Some(0));
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        state.apply_token_usage("agent-1", usage(1_234));
        assert!(state.token_usage.is_some());

        state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        state.list.select(Some(1));
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(
            state.detail.as_ref().map(|d| d.id.as_str()),
            Some("agent-2")
        );
        assert!(
            state.token_usage.is_none(),
            "the panel must not show agent A's figures for agent B"
        );
    }
}

#[cfg(test)]
mod workspaces_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Renders `draw_edit_workspaces` to an in-memory buffer and returns its
    /// text content, so a test can assert on what an operator would actually
    /// see rather than on internal state alone (#7835).
    fn rendered_edit_workspaces(state: &AgentSelectState) -> String {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw_edit_workspaces(f, f.area(), state))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn editing_state() -> AgentSelectState {
        let mut state = AgentSelectState::new();
        state.sub = AgentSubScreen::EditWorkspaces;
        state.ws_loaded = true;
        state.detail = Some(AgentDetail {
            id: "agent-1".to_string(),
            name: String::new(),
            state: String::new(),
            model: String::new(),
            provider: String::new(),
            created: String::new(),
            last_active: String::new(),
            tags: vec![],
            capabilities: vec![],
            parent: None,
            children: vec![],
            skills: vec![],
            skills_mode: String::new(),
            mcp_servers: vec![],
            mcp_servers_mode: String::new(),
            channels: vec![],
            channels_mode: String::new(),
        });
        state
    }

    #[test]
    fn adding_a_folder_focuses_the_name_field() {
        let mut state = editing_state();
        state.handle_key(key(KeyCode::Char('a')));
        assert_eq!(state.workspaces.len(), 1);
        assert!(matches!(state.ws_editing, Some((0, 0))));
    }

    #[test]
    fn typing_then_tab_fills_name_and_path() {
        let mut state = editing_state();
        state.handle_key(key(KeyCode::Char('a')));
        for c in "library".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Tab));
        assert_eq!(state.workspaces[0].0, "library");
        assert!(matches!(state.ws_editing, Some((0, 1))));
        for c in "shared/library".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Tab));
        assert_eq!(state.workspaces[0].1, "shared/library");
    }

    #[test]
    fn deleting_removes_the_selected_row() {
        let mut state = editing_state();
        state.workspaces.push(("a".into(), "p".into(), "rw".into()));
        state.workspaces.push(("b".into(), "q".into(), "r".into()));
        state.ws_cursor = 1;
        state.handle_key(key(KeyCode::Char('d')));
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].0, "a");
    }

    #[test]
    fn save_emits_only_complete_rows() {
        let mut state = editing_state();
        state
            .workspaces
            .push(("library".into(), "shared/library".into(), "rw".into()));
        state.workspaces.push(("".into(), "".into(), "rw".into()));
        match state.handle_key(key(KeyCode::Char('s'))) {
            AgentAction::UpdateWorkspaces { id, workspaces } => {
                assert_eq!(id, "agent-1");
                assert_eq!(workspaces.len(), 1);
                assert_eq!(workspaces[0].0, "library");
            }
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn save_trims_the_values_it_emits_and_reports_dropped_rows() {
        let mut state = editing_state();
        state.workspaces.push((
            "library ".into(),
            " shared/library".into(),
            "readwrite".into(),
        ));
        state
            .workspaces
            .push(("half".into(), "".into(), "readwrite".into()));
        match state.handle_key(key(KeyCode::Char('s'))) {
            AgentAction::UpdateWorkspaces { workspaces, .. } => {
                assert_eq!(workspaces.len(), 1);
                assert_eq!(
                    workspaces[0].0, "library",
                    "the emitted name must be trimmed"
                );
                assert_eq!(
                    workspaces[0].1, "shared/library",
                    "the emitted path must be trimmed"
                );
            }
            other => panic!("expected update, got {other:?}"),
        }
        assert!(
            !state.status_msg.is_empty(),
            "dropping a half-typed row must surface a message, not fail silently"
        );
        let rendered = rendered_edit_workspaces(&state);
        assert!(
            rendered.contains(state.status_msg.trim()),
            "the status message must actually be painted, not just set: {rendered:?}"
        );
    }

    #[test]
    fn entering_edit_seeds_the_buffer_so_committing_untouched_keeps_the_value() {
        let mut state = editing_state();
        state
            .workspaces
            .push(("library".into(), "shared/library".into(), "readonly".into()));
        state.handle_key(key(KeyCode::Enter)); // open the name field
        state.handle_key(key(KeyCode::Enter)); // commit name untouched, advance to path
        state.handle_key(key(KeyCode::Enter)); // commit path untouched, advance to mode
        state.handle_key(key(KeyCode::Enter)); // commit mode untouched, close the row

        assert_eq!(
            state.workspaces[0],
            (
                "library".to_string(),
                "shared/library".to_string(),
                "readonly".to_string()
            ),
            "committing every field without retyping must not blank any of them"
        );
    }

    #[test]
    fn retyping_the_displayed_readonly_value_does_not_escalate_to_readwrite() {
        let mut state = editing_state();
        state
            .workspaces
            .push(("library".into(), "shared/library".into(), "readonly".into()));
        state.handle_key(key(KeyCode::Enter)); // field 0 (name)
        state.handle_key(key(KeyCode::Tab)); // field 1 (path)
        state.handle_key(key(KeyCode::Tab)); // field 2 (mode), buf seeded "readonly"
        for _ in 0.."readonly".len() {
            state.handle_key(key(KeyCode::Backspace));
        }
        for c in "readonly".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Enter));

        assert_eq!(
            state.workspaces[0].2, "readonly",
            "retyping the exact value the row already displays must not grant read-write"
        );
    }

    #[test]
    fn unrecognized_mode_input_keeps_the_previous_value() {
        let mut state = editing_state();
        state
            .workspaces
            .push(("library".into(), "shared/library".into(), "readonly".into()));
        state.handle_key(key(KeyCode::Enter));
        state.handle_key(key(KeyCode::Tab));
        state.handle_key(key(KeyCode::Tab));
        for _ in 0.."readonly".len() {
            state.handle_key(key(KeyCode::Backspace));
        }
        for c in "garbage".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Enter));

        assert_eq!(
            state.workspaces[0].2, "readonly",
            "unrecognized mode input must not default to the more permissive read-write"
        );
    }

    #[test]
    fn editor_ignores_every_key_but_esc_until_loaded() {
        let mut state = editing_state();
        // A non-empty `workspaces` with `ws_loaded == false` isn't reachable
        // through the normal `w` reset, but proves the gate itself — not an
        // empty vector — is what blocks `Enter`/`'d'` below. With an empty
        // vector both are already no-ops regardless of the gate
        // (`if len > 0`), so that alone wouldn't distinguish the two.
        state.workspaces.push((
            "library".into(),
            "shared/library".into(),
            "readwrite".into(),
        ));
        state.ws_loaded = false;

        state.handle_key(key(KeyCode::Char('a')));
        assert_eq!(
            state.workspaces.len(),
            1,
            "adding a row before the fetch lands must not touch the table"
        );
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('s'))),
            AgentAction::Continue
        ));
        state.handle_key(key(KeyCode::Char('d')));
        assert_eq!(
            state.workspaces.len(),
            1,
            "'d' must not delete a row while the fetch hasn't landed"
        );
        state.handle_key(key(KeyCode::Enter));
        assert!(
            state.ws_editing.is_none(),
            "Enter must not open a field for editing while the fetch hasn't landed"
        );

        state.handle_key(key(KeyCode::Esc));
        assert!(matches!(state.sub, AgentSubScreen::AgentDetail));
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// #7781 review: `default_profile` is not editable from this screen, so
    /// saving a routing edit (Enter) must carry through whatever value was
    /// loaded rather than silently dropping it.
    #[test]
    fn saving_model_routing_preserves_the_loaded_default_profile() {
        let mut state = AgentSelectState::new();
        state.detail = Some(AgentDetail {
            id: "agent-1".to_string(),
            ..AgentDetail::default()
        });
        state.model_mode = "flexible".to_string();
        state.router_default_profile = Some("coder".to_string());
        state.routing_loaded = true;

        let action = state.handle_edit_model_routing(key(KeyCode::Enter));

        match action {
            AgentAction::UpdateModelRouting {
                default_profile, ..
            } => {
                assert_eq!(
                    default_profile,
                    Some("coder".to_string()),
                    "save must not clear the loaded default_profile"
                );
            }
            _ => panic!("Enter must emit UpdateModelRouting"),
        }
    }

    /// #7781 review: `fixed` — the per-agent router bypass — is not
    /// editable from this screen either, and the InProcess save path used
    /// to hardcode it to `false` unconditionally. Same contract as
    /// `default_profile` above.
    #[test]
    fn saving_model_routing_preserves_the_loaded_fixed_flag() {
        let mut state = AgentSelectState::new();
        state.detail = Some(AgentDetail {
            id: "agent-1".to_string(),
            ..AgentDetail::default()
        });
        state.model_mode = "flexible".to_string();
        state.router_fixed = true;
        state.routing_loaded = true;

        let action = state.handle_edit_model_routing(key(KeyCode::Enter));

        match action {
            AgentAction::UpdateModelRouting { fixed, .. } => {
                assert!(fixed, "save must not clear the loaded fixed flag");
            }
            _ => panic!("Enter must emit UpdateModelRouting"),
        }
    }

    /// #7781 review: opening the editor for a different agent (`r` from the
    /// detail pane) must not let the previous agent's routing values leak
    /// into a save for the new one if the fetch that follows fails.
    #[test]
    fn entering_the_routing_editor_resets_stale_values() {
        let mut state = AgentSelectState::new();
        state.detail = Some(AgentDetail {
            id: "agent-2".to_string(),
            ..AgentDetail::default()
        });
        // Simulate leftover state from a previously edited agent.
        state.model_mode = "flexible".to_string();
        state.router_profiles = vec![("coder".to_string(), true)];
        state.router_profile_cursor = 1;
        state.cost_budget_idx = 2;
        state.router_default_profile = Some("coder".to_string());
        state.router_fixed = true;
        state.routing_loaded = true;

        state.handle_detail(key(KeyCode::Char('r')));

        assert_eq!(state.model_mode, "fixed");
        assert!(state.router_profiles.is_empty());
        assert_eq!(state.router_profile_cursor, 0);
        assert_eq!(state.cost_budget_idx, 0);
        assert_eq!(state.router_default_profile, None);
        assert!(!state.router_fixed);
        assert!(
            !state.routing_loaded,
            "re-entering must require a fresh AgentModelRoutingLoaded before Enter can save"
        );
    }

    /// #7781 review: if the fetch after `r` fails (or has not returned yet),
    /// `Enter` must not save — it would write this screen's reset
    /// placeholder values over the agent's real settings. `FetchError` only
    /// sets a status message; it does not leave the editor, so this guard
    /// is the only thing standing between a failed fetch and a bad write.
    #[test]
    fn saving_before_routing_loaded_is_a_no_op() {
        let mut state = AgentSelectState::new();
        state.detail = Some(AgentDetail {
            id: "agent-3".to_string(),
            ..AgentDetail::default()
        });
        state.model_mode = "flexible".to_string();
        state.router_default_profile = Some("coder".to_string());
        state.router_fixed = true;
        // routing_loaded defaults to false and was not set here.

        let action = state.handle_edit_model_routing(key(KeyCode::Enter));

        assert!(
            matches!(action, AgentAction::Continue),
            "Enter must not emit a save before the real settings have loaded"
        );
        assert!(
            !state.status_msg.is_empty(),
            "the operator needs to know why Enter did nothing"
        );
    }
}
