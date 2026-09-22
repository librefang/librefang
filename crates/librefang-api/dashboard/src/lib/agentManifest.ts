// Structured representation of AgentManifest for the visual editor.
//
// The form covers nearly all fields the kernel understands; anything not
// represented here (tools.* tables, metadata, profile, extra_params,
// internal flags) lives in `extras` so a TOML-tab edit that adds them
// survives a round-trip back through the form.

import { parse, stringify, TomlError, type TomlTable } from "smol-toml";
// The parameter range table is the single source of truth for a model
// parameter's ceiling (#8332); the validator reads it rather than restating
// a number beside it, the same table the controls render from.
import { MODEL_PARAM_RANGES } from "../components/ui/ModelParamField";

let _nextUid = 1;
export const generateUid = (): string => String(_nextUid++);

// Numeric inputs are stored as raw strings so empty fields stay empty
// (instead of becoming 0 and silently overriding kernel defaults).
export interface ManifestFormState {
  name: string;
  description: string;
  version: string;
  author: string;
  module: string;
  priority: "Low" | "Normal" | "High" | "Critical";
  session_mode: "persistent" | "new";
  web_search_augmentation: "off" | "auto" | "always";
  // Tri-state: `""` inherits the kernel's `[task_board].assignee_wake`, and the
  // two strings are an explicit override. A plain boolean cannot say "this
  // agent has no opinion", and this key is precisely a per-agent override of a
  // global default — writing `false` where the operator meant "inherit" would
  // silently pin the agent against a later deployment-wide change.
  assignee_wake: "" | "true" | "false";
  // Plain booleans whose serde default is `true`, so the form carries the same
  // default and only writes the key when it differs. See `emptyManifestForm`.
  show_progress: boolean;
  cache_context: boolean;
  mcp_disabled: boolean;
  // Tri-state counts: `""` inherits, a number is an override. Same shape as
  // the sampling knobs — an absent key and a zero are different statements.
  max_history_messages: string;
  max_concurrent_invocations: string;
  // Enum overrides. The values are serde's renamed forms, not the Rust variant
  // names: `ToolProfile` and `OrphanPolicy` are `rename_all = "snake_case"` and
  // `BackendKind` is `rename_all = "lowercase"`, so `"Full"` or `"Docker"`
  // written here would be a manifest the daemon cannot deserialise.
  tool_exec_backend: "" | "local" | "docker" | "ssh" | "daytona";
  profile:
    | ""
    | "minimal"
    | "coding"
    | "research"
    | "messaging"
    | "automation"
    | "full"
    | "custom";
  reconcile_orphans: "keep" | "warn" | "delete";
  // Per-agent overrides of the kernel's `[proactive_memory]`. Every switch is
  // tri-state for the same reason as `assignee_wake`: `memory.rs` declares them
  // `Option<bool>` with `skip_serializing_if`, so "inherit the global value"
  // and "explicitly off" are different keys on disk, and writing `false` where
  // the operator meant inherit pins the agent against a later change to the
  // deployment's setting.
  proactive_memory: {
    enabled: "" | "true" | "false";
    auto_memorize: "" | "true" | "false";
    auto_retrieve: "" | "true" | "false";
    extraction_model: string;
    session_scoped_recall: "" | "true" | "false";
    min_similarity: string;
    allow_self_consolidation: "" | "true" | "false";
  };
  // Idle thresholds before an agent may dream. `Option<f64>` / `Option<u32>`,
  // so `""` inherits and a number overrides.
  auto_dream_min_hours: string;
  auto_dream_min_sessions: string;
  // A single `Option<bool>` on its own struct — tri-state for the same reason
  // as the rest: absent inherits the kernel's `[rl_export] enabled`.
  rl_export: "" | "true" | "false";
  // `[async_tasks]`: a timeout that is `Option<u64>`, and a bool that is not.
  // `notify_on_timeout`'s compiled default is `true` — `AsyncTasksConfig` has a manual `Default` impl saying so, and its doc explains why: a timeout is user-visible so the agent can react to it.
  // That makes `true` the value that must not be written, and `false` the decision worth recording.
  async_tasks: {
    default_timeout_secs: string;
    notify_on_timeout: boolean;
  };
  // `[skill_workshop]`. `enabled` and `auto_capture` are plain `bool`s, not
  // `Option<bool>`: the struct's `Default` supplies them, so "absent" and
  // "the default" are the same state and a plain boolean says it exactly.
  //
  // The defaults are read from `impl Default for SkillWorkshopConfig`, not
  // assumed. `auto_capture` is **true** — the workshop is off, but its capture
  // pass is on by default, so that turning the master switch on gives a
  // workshop that does something. Writing `false` for an agent that never
  // touched it would silently disable capture the moment anyone opened and
  // saved the agent.
  skill_workshop: {
    enabled: boolean;
    auto_capture: boolean;
    approval_policy: "pending" | "auto";
    review_mode: "heuristic" | "threshold_llm" | "none";
    max_pending: string;
    max_pending_age_days: string;
    evolution_mode: "free" | "controlled";
  };
  // `[channel_overrides]`: per-agent overrides of a channel's own config.
  //
  // Fields that are `Option<T>` hold `""` for "inherit the channel's value".
  // The rest are plain, because `ChannelOverrides` carries `#[serde(default)]`
  // and its own `Default` — and eight of those defaults come from named
  // functions rather than the type's zero, so they are listed here rather than
  // assumed. `thread_ownership_enabled` is the one that reads backwards:
  // `true`, not `false`.
  channel_overrides: {
    model: string;
    system_prompt: string;
    dm_policy: "" | "respond" | "allowed_only" | "ignore";
    group_policy: "" | "all" | "mention_only" | "commands_only" | "ignore";
    group_trigger_patterns: string[];
    reply_precheck: boolean;
    reply_precheck_model: string;
    rate_limit_per_minute: string;
    rate_limit_per_user: string;
    threading: boolean;
    output_format: "" | "markdown" | "telegram_html" | "slack_mrkdwn" | "plain_text";
    usage_footer: "" | "off" | "tokens" | "cost" | "full";
    typing_mode: "" | "instant" | "message" | "thinking" | "never";
    message_debounce_ms: string;
    message_debounce_max_ms: string;
    message_debounce_max_buffer: string;
    clear_done_reaction: boolean;
    disable_commands: boolean;
    allowed_commands: string[];
    blocked_commands: string[];
    auto_route: "off" | "explicit_only" | "sticky_ttl" | "sticky_heuristic";
    auto_route_ttl_minutes: string;
    auto_route_confidence_threshold: string;
    auto_route_sticky_bonus: string;
    auto_route_divergence_count: string;
    prefix_agent_name: "off" | "bracket" | "bold_bracket";
    thread_ownership_enabled: boolean;
    conversation_ownership_ttl_seconds: string;
    conversation_ownership_include_dms: boolean;
  };
  // Per-agent overrides of the kernel's `[compaction]`. Nine `Option<T>` with
  // `skip_serializing_if`, so `""` inherits and a value overrides — and the
  // table is not written at all when every field says nothing.
  compaction: {
    threshold_messages: string;
    keep_recent: string;
    max_summary_tokens: string;
    token_threshold_ratio: string;
    max_chunk_chars: string;
    max_retries: string;
    aggregate_developer_loops: "" | "true" | "false";
    max_loop_steps_before_aggregate: string;
    strip_reasoning_after_turns: string;
  };
  pinned_model: string;
  workspace: string;

  schedule:
    | { mode: "reactive" }
    | { mode: "periodic"; cron: string }
    | { mode: "proactive"; conditions: string[] }
    | { mode: "continuous"; check_interval_secs: string };

  // Every numeric field here is tri-state, and `""` is the third state.
  // It means "this agent has no opinion", which the kernel reads as inherit:
  // the per-model override supplies the value, and failing that the system default.
  // It is emphatically not zero, and it is not the same as a number that happens to match the default.
  model: {
    provider: string;
    model: string;
    system_prompt: string;
    temperature: string;
    max_tokens: string;
    top_p: string;
    frequency_penalty: string;
    presence_penalty: string;
    // Endpoint limits rather than sampling preferences: what the model can
    // read and emit, not how it should sound.
    context_window: string;
    max_output_tokens: string;
    api_key_env: string;
    base_url: string;
    // `[model] mode` and `[model] router_override` — the profile router's
    // per-agent settings, read and written inside the agent's own manifest
    // (#8424). They are manifest fields, so this form is where they are
    // edited; the routing panel that shared them was removed rather than
    // kept as a second writer.
    mode: "fixed" | "flexible";
    // `AgentRouterOverride` is `Option` in Rust: every field below is only
    // meaningful once at least one of them is set, and the table is left out
    // entirely when none is.
    router_fixed: boolean;
    /** Empty means "any profile is allowed", which is also what the daemon reads. */
    router_allowed_profiles: string[];
    router_cost_budget: "" | "cheap" | "medium" | "expensive";
    router_default_profile: string;
    /**
     * Keys inside `router_override` the form has no widget for.
     * `AgentRouterOverride` carries no `deny_unknown_fields`
     * (crates/librefang-types/src/model_profile.rs), so such a key is a legal
     * manifest member the daemon keeps on disk — the stash is what lets the
     * re-emitted inline table carry it back instead of dropping it.
     */
    router_override_preserved?: TomlTable;
  };

  /** Tri-state: `null` = key absent (inherit global fallback_providers),
   * `[]` = declared empty (disable all fallbacks for this agent), rows = the
   * list. Collapsing `[]` to absent silently re-enables global fallbacks on
   * an agent pinned to none (#7749 review). */
  fallback_models: Array<{
    _uid: string;
    provider: string;
    model: string;
    api_key_env: string;
    base_url: string;
    // FallbackModel uses #[serde(flatten)] for provider-specific
    // params (e.g. Qwen's enable_memory). Hold them here so round-trips
    // through the form don't strip provider customisations.
    extras: TomlTable;
  }> | null;

  resources: {
    max_llm_tokens_per_hour: string;
    max_tool_calls_per_minute: string;
    max_cost_per_hour_usd: string;
    max_cost_per_day_usd: string;
    max_cost_per_month_usd: string;
    max_memory_bytes: string;
    max_cpu_time_ms: string;
    max_network_bytes_per_hour: string;
    /**
     * `Option<f32>`, clamped to 0.01..=1.0 at enforcement time, not at write
     * time — so the form just carries what the operator wrote.
     * `""` is the absent key, which means the compiled default of 0.2 applies.
     */
    burst_ratio: string;
  };

  capabilities: {
    network: string[];
    shell: string[];
    tools: string[];
    /** Tri-state (#7605): `null` = key absent (unrestricted), `[]` = declared and grants
     * nothing (deny), non-empty = allowlist. A plain `string[]` cannot carry the
     * distinction, and a save that collapses `[]` to absent silently flips a
     * deny to unrestricted (#7749 review). */
    memory_read: string[] | null;
    memory_write: string[] | null;
    agent_message: string[];
    ofp_connect: string[];
    agent_spawn: boolean;
    ofp_discover: boolean;
    /**
     * Media capability routing, flattened into `[capabilities]` exactly as the
     * kernel reads it. Each value is the `"provider/model"` shorthand, or a
     * bare provider id.
     *
     * An **empty string means inherit** the kernel-global `[capabilities]`
     * block — the key is then omitted from the emitted TOML entirely, which is
     * what inheritance looks like on disk. There is no separate "inherit"
     * sentinel precisely so that clearing the field and saving cannot leave a
     * value pinned behind.
     */
    image_understanding: string;
    speech_to_text: string;
    image_generation: string;
    text_to_speech: string;
    video_generation: string;
    music_generation: string;
  };

  thinking: {
    enabled: boolean;
    budget_tokens: string;
    stream_thinking: boolean;
  };

  autonomous: {
    enabled: boolean;
    max_iterations: string;
    max_restarts: string;
    heartbeat_interval_secs: string;
    heartbeat_timeout_secs: string;
    heartbeat_keep_recent: string;
    heartbeat_channel: string;
    quiet_hours: string;
  };

  routing: {
    enabled: boolean;
    simple_model: string;
    medium_model: string;
    complex_model: string;
    simple_threshold: string;
    complex_threshold: string;
  };

  context_injection: Array<{
    _uid: string;
    name: string;
    content: string;
    position: "system" | "before_user" | "after_reset";
    condition: string;
    /** Keys inside the row the form has no widget for. */
    preserved?: TomlTable;
  }>;

  response_format:
    | { mode: "text" }
    | { mode: "json"; preserved?: TomlTable }
    | {
        mode: "json_schema";
        name: string;
        schema: string;
        strict: boolean;
        /** Keys inside the table the form has no widget for. */
        preserved?: TomlTable;
      };

  // Only the shorthand string variants are exposed here. Full ExecPolicy
  // tables (`[exec_policy]` with mode/safe_bins/timeout_secs/…) stay in
  // extras so they're preserved without complicating the form.
  exec_policy_shorthand: "" | "allow" | "deny" | "full" | "allowlist";

  skills: string[];
  mcp_servers: string[];
  tags: string[];
  tool_allowlist: string[];
  tool_blocklist: string[];
  allowed_plugins: string[];

  enabled: boolean;
  skills_disabled: boolean;
  tools_disabled: boolean;
  inherit_parent_context: boolean;
  generate_identity_files: boolean;

  workspaces: Array<{
    _uid: string;
    name: string;
    path: string;
    mode: "rw" | "r";
    /** Keys inside the row the form has no widget for. */
    preserved?: TomlTable;
  }>;
}

export interface ManifestExtras {
  topLevel: TomlTable;
  model: TomlTable;
  resources: TomlTable;
  capabilities: TomlTable;
  // `[thinking]` keys the form has no widget for — `reasoning_mode` (#7946) is
  // the first one. Without this slot the form re-emits the section from its two
  // known fields alone, so opening an agent in the editor and saving it
  // silently deletes any newer key from that agent's agent.toml.
  thinking: TomlTable;
  // `[autonomous]` and `[routing]` are in `FORM_TOP_LEVEL_KEYS`, which means
  // their tables never reach `topLevel` — so without a slot of their own, every
  // key the form has no widget for is consumed on parse and never re-emitted.
  // `block_stall_degrade_after` (the loop-guard threshold) was being deleted
  // that way; `reasoning_mode` was the same bug in `[thinking]`, which is why
  // that slot exists above.
  autonomous: TomlTable;
  routing: TomlTable;
  // `[proactive_memory]` is about to join `FORM_TOP_LEVEL_KEYS`, which means
  // its table never reaches `topLevel` — so without a slot of its own, every
  // key the form has no widget for is consumed on parse and never re-emitted.
  // Same reasoning as the `thinking` and `autonomous` slots above; this is the
  // bug that was deleting keys before those existed.
  proactive_memory: TomlTable;
  async_tasks: TomlTable;
  // `[rl_export]` is in `FORM_TOP_LEVEL_KEYS` too, so without this slot every
  // key of the table but `enabled` is consumed on parse and never re-emitted.
  // It needs no "body is empty" trigger to lose data, which is what made it the
  // worse half of the pair: any agent carrying an `[rl_export]` key the form
  // does not render lost it on the next save, unconditionally.
  rl_export: TomlTable;
  // The chosen `[schedule.<variant>]` table's keys the form does not render.
  // Shaped `{ <variant>: { … } }`, because that is the depth the form owns: the
  // `[schedule]` root is a closed enum, so only a variant's contents can carry
  // a key this editor has no widget for.
  schedule: TomlTable;
  compaction: TomlTable;
  skill_workshop: TomlTable;
  channel_overrides: TomlTable;
}

export const emptyManifestExtras = (): ManifestExtras => ({
  topLevel: {},
  model: {},
  resources: {},
  capabilities: {},
  thinking: {},
  autonomous: {},
  routing: {},
  proactive_memory: {},
  async_tasks: {},
  rl_export: {},
  schedule: {},
  compaction: {},
  skill_workshop: {},
  channel_overrides: {},
});

/**
 * The form's starting state, and the one place where a default can be got
 * wrong in a way nothing else catches.
 *
 * **Read the Rust `Default`; do not infer the value from the field's name.**
 * Three fields in this editor default to the opposite of what their name
 * suggests, and all three were found by reading the source rather than by
 * reasoning about them:
 *
 * - `show_progress` — `#[serde(default = "default_true")]`, so an absent key
 *   means the agent *does* stream progress. Defaulting it to `false` here
 *   would turn the indicator off for every agent opened and saved.
 * - `auto_capture` — `impl Default for SkillWorkshopConfig` sets it `true`
 *   while `enabled` is `false`: the workshop is off, its capture pass is on,
 *   so the master switch has something to switch on.
 * - `notify_on_timeout` — a plain `#[serde(default)]` on a field whose
 *   sibling is an `Option`, so `false` is the value that must *not* be
 *   written on an agent that never chose.
 *
 * The shape of the failure is the same in all three: the form writes a
 * decision the operator never made, into a file nobody reads, and the symptom
 * surfaces later as behaviour that changed on its own. A wrong default here
 * is not caught by any test unless a test was written for that field — which
 * is why the rule is to read, not to reason.
 */
export const emptyManifestForm = (): ManifestFormState => ({
  name: "",
  description: "",
  version: "1.0.0",
  author: "",
  module: "builtin:chat",
  priority: "Normal",
  session_mode: "persistent",
  web_search_augmentation: "auto",
  assignee_wake: "",
  show_progress: true,
  cache_context: false,
  mcp_disabled: false,
  max_history_messages: "",
  max_concurrent_invocations: "",
  tool_exec_backend: "",
  profile: "",
  reconcile_orphans: "keep",
  proactive_memory: {
    enabled: "",
    auto_memorize: "",
    auto_retrieve: "",
    extraction_model: "",
    session_scoped_recall: "",
    min_similarity: "",
    allow_self_consolidation: "",
  },
  auto_dream_min_hours: "",
  auto_dream_min_sessions: "",
  rl_export: "",
  async_tasks: {
    default_timeout_secs: "",
    // The daemon's own default, not "off": see the interface comment above.
    notify_on_timeout: true,
  },
  channel_overrides: {
    model: "",
    system_prompt: "",
    dm_policy: "",
    group_policy: "",
    group_trigger_patterns: [],
    reply_precheck: false,
    reply_precheck_model: "",
    rate_limit_per_minute: "",
    rate_limit_per_user: "",
    threading: false,
    output_format: "",
    usage_footer: "",
    typing_mode: "",
    message_debounce_ms: "",
    message_debounce_max_ms: "",
    message_debounce_max_buffer: "",
    clear_done_reaction: false,
    disable_commands: false,
    allowed_commands: [],
    blocked_commands: [],
    auto_route: "off",
    auto_route_ttl_minutes: "",
    auto_route_confidence_threshold: "",
    auto_route_sticky_bonus: "",
    auto_route_divergence_count: "",
    prefix_agent_name: "off",
    // `default_thread_ownership_enabled` returns `true`.
    thread_ownership_enabled: true,
    conversation_ownership_ttl_seconds: "",
    conversation_ownership_include_dms: false,
  },
  skill_workshop: {
    enabled: false,
    auto_capture: true,
    approval_policy: "pending",
    review_mode: "heuristic",
    max_pending: "",
    max_pending_age_days: "",
    evolution_mode: "free",
  },
  compaction: {
    threshold_messages: "",
    keep_recent: "",
    max_summary_tokens: "",
    token_threshold_ratio: "",
    max_chunk_chars: "",
    max_retries: "",
    aggregate_developer_loops: "",
    max_loop_steps_before_aggregate: "",
    strip_reasoning_after_turns: "",
  },
  pinned_model: "",
  workspace: "",
  schedule: { mode: "reactive" },
  model: {
    provider: "",
    model: "",
    system_prompt: "",
    temperature: "",
    max_tokens: "",
    top_p: "",
    frequency_penalty: "",
    presence_penalty: "",
    context_window: "",
    max_output_tokens: "",
    api_key_env: "",
    base_url: "",
    // `ModelMode::Fixed` is the `#[default]` variant, and it is also what every
    // manifest written before the profile router existed means.
    mode: "fixed",
    router_fixed: false,
    router_allowed_profiles: [],
    router_cost_budget: "",
    router_default_profile: "",
  },
  fallback_models: null,
  resources: {
    max_llm_tokens_per_hour: "",
    max_tool_calls_per_minute: "",
    max_cost_per_hour_usd: "",
    max_cost_per_day_usd: "",
    max_cost_per_month_usd: "",
    max_memory_bytes: "",
    max_cpu_time_ms: "",
    max_network_bytes_per_hour: "",
    // "" is the absent key: the compiled default of 0.2 applies (see the
    // field's doc in ManifestFormState.resources).
    burst_ratio: "",
  },
  capabilities: {
    network: [],
    shell: [],
    tools: [],
    memory_read: null,
    memory_write: null,
    agent_message: [],
    ofp_connect: [],
    agent_spawn: false,
    ofp_discover: false,
    image_understanding: "",
    speech_to_text: "",
    image_generation: "",
    text_to_speech: "",
    video_generation: "",
    music_generation: "",
  },
  thinking: { enabled: false, budget_tokens: "", stream_thinking: false },
  autonomous: {
    enabled: false,
    max_iterations: "",
    max_restarts: "",
    heartbeat_interval_secs: "",
    heartbeat_timeout_secs: "",
    heartbeat_keep_recent: "",
    heartbeat_channel: "",
    quiet_hours: "",
  },
  routing: {
    enabled: false,
    simple_model: "",
    medium_model: "",
    complex_model: "",
    simple_threshold: "",
    complex_threshold: "",
  },
  context_injection: [],
  response_format: { mode: "text" },
  exec_policy_shorthand: "",
  skills: [],
  mcp_servers: [],
  tags: [],
  tool_allowlist: [],
  tool_blocklist: [],
  allowed_plugins: [],
  enabled: true,
  skills_disabled: false,
  tools_disabled: false,
  inherit_parent_context: true,
  generate_identity_files: true,
  workspaces: [],
});

/**
 * The media capabilities the form renders, in the order they appear in the
 * "Capabilities" section. Understanding first, then generation — that is the
 * order an operator reasons about them in, and the first two are the ones that
 * change how an inbound message is handled.
 */
export const CAPABILITY_ROUTING_KEYS = [
  "image_understanding",
  "speech_to_text",
  "image_generation",
  "text_to_speech",
  "video_generation",
  "music_generation",
] as const;

export type CapabilityRoutingKey = (typeof CAPABILITY_ROUTING_KEYS)[number];

/** Kernel-side aliases → the canonical key the form stores. */
const CAPABILITY_ROUTING_ALIASES: Record<string, CapabilityRoutingKey> = {
  vision: "image_understanding",
  transcription: "speech_to_text",
  speech: "text_to_speech",
};

/**
 * Read one routing value out of a parsed `[capabilities]` table.
 *
 * Accepts both spellings the kernel accepts — the `"provider/model"` string
 * and the `{ provider, model }` table — and normalises the table form back to
 * the shorthand, because the form edits a single text field. A table carrying
 * only `model` round-trips as `"/model"`, which the kernel parses back to an
 * empty provider plus that model, preserving the inherit-the-provider case.
 */
function readCapabilityRouting(capTable: TomlTable, key: CapabilityRoutingKey): string {
  const aliasKey = Object.keys(CAPABILITY_ROUTING_ALIASES).find(
    (a) => CAPABILITY_ROUTING_ALIASES[a] === key,
  );
  const raw = capTable[key] ?? (aliasKey ? capTable[aliasKey] : undefined);
  if (typeof raw === "string") return raw.trim();
  if (isTomlTable(raw)) {
    const provider = typeof raw.provider === "string" ? raw.provider.trim() : "";
    const model = typeof raw.model === "string" ? raw.model.trim() : "";
    if (!provider && !model) return "";
    return model ? `${provider}/${model}` : provider;
  }
  return "";
}

// Keys the form fully owns within each scope. Anything else is preserved
// as `extras` and re-emitted on serialize.
// Exported so the sweep test can assert that every table the form claims is
// actually swept for unknown-key loss, rather than trusting a hand-kept list.
export const FORM_TOP_LEVEL_KEYS = new Set([
  "name",
  "version",
  "description",
  "author",
  "module",
  "enabled",
  "priority",
  "session_mode",
  "web_search_augmentation",
  "assignee_wake",
  "show_progress",
  "cache_context",
  "mcp_disabled",
  "max_history_messages",
  "max_concurrent_invocations",
  "tool_exec_backend",
  "profile",
  "reconcile_orphans",
  "proactive_memory",
  "auto_dream_min_hours",
  "auto_dream_min_sessions",
  "rl_export",
  "async_tasks",
  "compaction",
  "skill_workshop",
  "channel_overrides",
  "pinned_model",
  "workspace",
  "skills_disabled",
  "tools_disabled",
  "inherit_parent_context",
  "generate_identity_files",
  "tags",
  "skills",
  "mcp_servers",
  "tool_allowlist",
  "tool_blocklist",
  "allowed_plugins",
  "schedule",
  "model",
  "resources",
  "capabilities",
  "fallback_models",
  "thinking",
  "autonomous",
  "routing",
  "context_injection",
  "response_format",
  "exec_policy",
  "workspaces",
]);
const FORM_MODEL_KEYS = new Set([
  "provider",
  "model",
  "system_prompt",
  "temperature",
  "max_tokens",
  "top_p",
  "frequency_penalty",
  "presence_penalty",
  "context_window",
  "max_output_tokens",
  "api_key_env",
  "base_url",
  // Both now have widgets, so both are the form's to emit: leaving them out of
  // this set would put them in the extras as well and emit each one twice.
  "mode",
  "router_override",
]);
const FORM_RESOURCE_KEYS = new Set([
  "max_llm_tokens_per_hour",
  "max_tool_calls_per_minute",
  "max_cost_per_hour_usd",
  "max_cost_per_day_usd",
  "max_cost_per_month_usd",
  "max_memory_bytes",
  "max_cpu_time_ms",
  "max_network_bytes_per_hour",
  // Has a widget too, so it is the form's to emit: leaving it out of this set
  // would put it in the extras as well and emit it twice.
  "burst_ratio",
]);
const FALLBACK_MODEL_KEYS = new Set([
  "provider",
  "model",
  "api_key_env",
  "base_url",
]);
const FORM_CAPABILITY_KEYS = new Set([
  "network",
  "shell",
  "tools",
  "memory_read",
  "memory_write",
  "agent_message",
  "ofp_connect",
  "agent_spawn",
  "ofp_discover",
  // Media capability routing (flattened into `[capabilities]` kernel-side).
  ...CAPABILITY_ROUTING_KEYS,
  // Aliases the kernel accepts. Listed so a manifest that spells the key the
  // other way is loaded into the form field rather than silently preserved as
  // an "extra" — which would then be re-emitted alongside the form's own key
  // and give the block two spellings of the same setting.
  "vision",
  "transcription",
  "speech",
]);
const FORM_THINKING_KEYS = new Set(["budget_tokens", "stream_thinking"]);
const FORM_CHANNEL_OVERRIDE_KEYS = new Set([
  "model",
  "system_prompt",
  "dm_policy",
  "group_policy",
  "group_trigger_patterns",
  "reply_precheck",
  "reply_precheck_model",
  "rate_limit_per_minute",
  "rate_limit_per_user",
  "threading",
  "output_format",
  "usage_footer",
  "typing_mode",
  "message_debounce_ms",
  "message_debounce_max_ms",
  "message_debounce_max_buffer",
  "clear_done_reaction",
  "disable_commands",
  "allowed_commands",
  "blocked_commands",
  "auto_route",
  "auto_route_ttl_minutes",
  "auto_route_confidence_threshold",
  "auto_route_sticky_bonus",
  "auto_route_divergence_count",
  "prefix_agent_name",
  "thread_ownership_enabled",
  "conversation_ownership_ttl_seconds",
  "conversation_ownership_include_dms",
]);

const FORM_SKILL_WORKSHOP_KEYS = new Set([
  "enabled",
  "auto_capture",
  "approval_policy",
  "review_mode",
  "max_pending",
  "max_pending_age_days",
  "evolution_mode",
]);

const FORM_COMPACTION_KEYS = new Set([
  "threshold_messages",
  "keep_recent",
  "max_summary_tokens",
  "token_threshold_ratio",
  "max_chunk_chars",
  "max_retries",
  "aggregate_developer_loops",
  "max_loop_steps_before_aggregate",
  "strip_reasoning_after_turns",
]);

const FORM_PROACTIVE_MEMORY_KEYS = new Set([
  "enabled",
  "auto_memorize",
  "auto_retrieve",
  "extraction_model",
  "session_scoped_recall",
  "min_similarity",
  "allow_self_consolidation",
]);

const FORM_AUTONOMOUS_KEYS = new Set([
  "max_iterations",
  "max_restarts",
  "heartbeat_interval_secs",
  "heartbeat_timeout_secs",
  "heartbeat_keep_recent",
  "heartbeat_channel",
  "quiet_hours",
]);
const FORM_ROUTING_KEYS = new Set([
  "simple_model",
  "medium_model",
  "complex_model",
  "simple_threshold",
  "complex_threshold",
]);

const SCHEDULE_DEFAULT_INTERVAL = "300";

/**
 * The keys the form renders inside each `[schedule.<variant>]` table, so the
 * ones it does not render can be carried through untouched.
 *
 * Keyed by the same snake_case spellings `ScheduleMode` serializes to
 * (crates/librefang-types/src/agent.rs), which is also what the form's `mode`
 * holds — one vocabulary, so a variant cannot be spelled one way in the parse
 * and another in the emit.
 */
const SCHEDULE_VARIANT_KEYS: Record<string, Set<string>> = {
  periodic: new Set(["cron"]),
  proactive: new Set(["conditions"]),
  continuous: new Set(["check_interval_secs"]),
};
const PRIORITIES = ["Low", "Normal", "High", "Critical"] as const;
const SESSION_MODES = ["persistent", "new"] as const;
const WEB_SEARCH_MODES = ["off", "auto", "always"] as const;
const INJECTION_POSITIONS = ["system", "before_user", "after_reset"] as const;
/** `ModelMode`'s `#[serde(rename_all = "snake_case")]` spellings. */
const MODEL_MODES = ["fixed", "flexible"] as const;
/** `CostTier`'s spellings, which the router API echoes byte for byte. */
const COST_TIERS = ["cheap", "medium", "expensive"] as const;
/** The members of `AgentRouterOverride` the form renders. */
const ROUTER_OVERRIDE_KEYS = new Set([
  "fixed",
  "allowed_profiles",
  "cost_budget",
  "default_profile",
]);
/** The members of a [[context_injection]] row the form renders. */
const CONTEXT_INJECTION_KEYS = new Set(["name", "content", "position", "condition"]);
/** The members of a [workspaces] path-form row the form renders. */
const WORKSPACE_ROW_KEYS = new Set(["path", "mode"]);
const EXEC_SHORTHANDS = ["allow", "deny", "full", "allowlist"] as const;
// These three mirror `rename_all` on the Rust enums, not the variant names:
// `ToolProfile` and `OrphanPolicy` are `snake_case`, `BackendKind` is
// `lowercase`. The form has to speak the serialised form because that is what
// lands in the TOML — `"Full"` would deserialise to the default instead.
export const TOOL_EXEC_BACKENDS = ["local", "docker", "ssh", "daytona"] as const;
export const TOOL_PROFILES = [
  "minimal",
  "coding",
  "research",
  "messaging",
  "automation",
  "full",
  "custom",
] as const;
export const ORPHAN_POLICIES = ["keep", "warn", "delete"] as const;
// `ApprovalPolicy` and `EvolutionMode` are `rename_all = "lowercase"`,
// `ReviewMode` is `"snake_case"`. The form speaks the serialised spelling
// because that is what lands in the TOML.
export const SKILL_APPROVAL_POLICIES = ["pending", "auto"] as const;
// All seven of these are `rename_all = "snake_case"` on the Rust enum.
export const DM_POLICIES = ["respond", "allowed_only", "ignore"] as const;
export const GROUP_POLICIES = ["all", "mention_only", "commands_only", "ignore"] as const;
export const OUTPUT_FORMATS = ["markdown", "telegram_html", "slack_mrkdwn", "plain_text"] as const;
export const USAGE_FOOTERS = ["off", "tokens", "cost", "full"] as const;
export const TYPING_MODES = ["instant", "message", "thinking", "never"] as const;
export const AUTO_ROUTE_STRATEGIES = ["off", "explicit_only", "sticky_ttl", "sticky_heuristic"] as const;
export const PREFIX_STYLES = ["off", "bracket", "bold_bracket"] as const;

// Keyed by the manifest field, so the parse side can look one up by name and
// the guard test can compare each against the Rust enum it came from.
export const CHANNEL_ENUMS = {
  dm_policy: DM_POLICIES,
  group_policy: GROUP_POLICIES,
  output_format: OUTPUT_FORMATS,
  usage_footer: USAGE_FOOTERS,
  typing_mode: TYPING_MODES,
  auto_route: AUTO_ROUTE_STRATEGIES,
  prefix_agent_name: PREFIX_STYLES,
} as const;
export const SKILL_REVIEW_MODES = ["heuristic", "threshold_llm", "none"] as const;
export const SKILL_EVOLUTION_MODES = ["free", "controlled"] as const;

const escapeTomlString = (value: string): string => {
  let escaped = "";
  for (const character of value) {
    switch (character) {
      case "\\":
        escaped += "\\\\";
        break;
      case '"':
        escaped += '\\"';
        break;
      case "\b":
        escaped += "\\b";
        break;
      case "\t":
        escaped += "\\t";
        break;
      case "\n":
        escaped += "\\n";
        break;
      case "\f":
        escaped += "\\f";
        break;
      case "\r":
        escaped += "\\r";
        break;
      default: {
        const codeUnit = character.charCodeAt(0);
        if (character.length === 1 && codeUnit >= 0xd800 && codeUnit <= 0xdfff) {
          escaped += "\ufffd";
        } else if (codeUnit <= 0x1f || character === "\u007f") {
          escaped += `\\u${codeUnit.toString(16).padStart(4, "0")}`;
        } else {
          escaped += character;
        }
        break;
      }
    }
  }
  return `"${escaped}"`;
};

const tomlArray = (values: string[]): string =>
  `[${values.map(escapeTomlString).join(", ")}]`;

// All integer manifest fields the form touches map to unsigned Rust
// types (u32/u64). Reject negatives and anything beyond JS's safe-integer
// range — the latter would silently lose precision before reaching the
// kernel, the former is rejected server-side anyway. For form UX, "garbage
// in → field omitted" is friendlier than a server-side error after submit.
const parseInteger = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n) || !Number.isInteger(n)) return null;
  if (n < 0 || n > Number.MAX_SAFE_INTEGER) return null;
  return n;
};

const TOML_INTEGER_MAX = 9_223_372_036_854_775_807n;

// Resource byte/time/token quotas are u64 in Rust, but TOML integers are
// signed 64-bit values. Keep their decimal form as a string so values beyond
// JavaScript's safe integer range survive the visual-editor round-trip.
const parseUnsignedTomlInteger = (raw: string): string | null => {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const value = BigInt(trimmed);
  if (value > TOML_INTEGER_MAX) return null;
  return value.toString();
};

const isPositiveUnsignedTomlInteger = (raw: string): boolean => {
  const value = parseUnsignedTomlInteger(raw);
  return value !== null && BigInt(value) > 0n;
};

/**
 * Blank means "this agent makes no override", which every optional count allows.
 *
 * Anything else has to be a TOML integer: the daemon deserializes these into
 * `Option<usize>`, and one that is not a whole number rejects the whole
 * document. Unlike `isPositiveUnsignedTomlInteger`, zero is not an error here —
 * what a zero means for a concurrency cap is the kernel's business, and this
 * check exists to keep the form from producing a document the server it feeds
 * will refuse.
 */
const isBlankOrUnsignedTomlInteger = (raw: string): boolean =>
  raw.trim() === "" || parseUnsignedTomlInteger(raw) !== null;

/** `u32::MAX` — the largest value an `Option<u32>` manifest count can carry. */
const U32_TOML_MAX = 4294967295n;

/**
 * The blank-or-unsigned check at an explicit ceiling.
 *
 * The whole-number check above accepts anything up to `TOML_INTEGER_MAX`
 * (2^63-1), which is the `usize`/`u64` half of the manifest's counts. A field
 * whose Rust side is `Option<u32>` stops one power of two lower: serde
 * rejects `4294967296` with the same 400 the whole-number check was written
 * to close, so the ceiling belongs to the field's type, not to the shared
 * parser. Where a field carries a ceiling in `MODEL_PARAM_RANGES` (#8332),
 * the table's number is the one used — one source of truth.
 */
const isBlankOrUnsignedTomlIntegerAtMost = (raw: string, ceiling: bigint): boolean => {
  const value = parseUnsignedTomlInteger(raw);
  return raw.trim() === "" || (value !== null && BigInt(value) <= ceiling);
};

/** Every `Option<u32>` count the form edits (`crates/librefang-types/src/agent.rs`). */
const isBlankOrU32TomlInteger = (raw: string): boolean =>
  isBlankOrUnsignedTomlIntegerAtMost(raw, U32_TOML_MAX);

/**
 * max_tokens's ceiling comes from the range table (#8332), not a second
 * number beside this one. The table's max for the parameter IS `u32::MAX`,
 * so table and type agree today — the fallback only names what still applies
 * if the table entry ever loses its max: the Rust field is `Option<u32>`
 * (agent.rs:949), and that bound outlives any table edit.
 */
const MODEL_MAX_TOKENS_CEILING =
  MODEL_PARAM_RANGES.max_tokens.max !== undefined
    ? BigInt(MODEL_PARAM_RANGES.max_tokens.max)
    : U32_TOML_MAX;

/**
 * Parse a float that may legitimately be negative.
 *
 * `parseFloatish` refuses negatives because every field it was written for is a
 * cost or a quota. `frequency_penalty` and `presence_penalty` range -2.0..2.0,
 * so routing them through it silently dropped every negative value the operator
 * typed — the field accepted the input and the TOML came out without the key.
 */
const parseSignedFloat = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  return Number.isFinite(n) ? n : null;
};

const parseFloatish = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n)) return null;
  if (n < 0) return null; // all our float fields are cost/quota — never negative
  return n;
};

// The number inputs declare min/max, but min/max does not stop pasted or
// programmatic values on a non-submitted form. `PATCH /api/agents/{id}/model`
// rejects an out-of-range sampling value with an explicit 400 rather than
// clamping it; `validateManifestForm` (below) mirrors those same ranges so
// this editor reports the same conflict instead of silently rewriting the
// number to one the operator never chose (#8112 review).
const isInRange = (raw: string, min: number, max: number): boolean => {
  const v = parseSignedFloat(raw);
  return v === null || (v >= min && v <= max);
};

const writeStringScalar = (lines: string[], key: string, value: string): void => {
  if (!value) return;
  lines.push(`${key} = ${escapeTomlString(value)}`);
};
// `system_prompt` is not tri-state like the sampling knobs above it: a blank
// value here means "this agent has no system prompt", not "no opinion, fall
// back to the canned default" — that was the removed flat editor's documented
// contract (`system_prompt_hint`: "Left blank, the agent type stores a blank
// prompt — nothing is substituted for you"). Routing it through the
// skip-if-empty `writeStringScalar` drops the key on an intentionally blank
// prompt, and `ModelConfig`'s container-level `#[serde(default)]` then fills
// the missing key with "You are a helpful AI agent." on the very next save.
const writeSystemPrompt = (lines: string[], value: string): void => {
  lines.push(`system_prompt = ${escapeTomlString(value)}`);
};
const writeNumberScalar = (lines: string[], key: string, value: number | null): void => {
  if (value === null) return;
  lines.push(`${key} = ${value}`);
};
const writeIntegerScalar = (lines: string[], key: string, value: string | null): void => {
  if (value === null) return;
  lines.push(`${key} = ${value}`);
};
/**
 * Writes a tri-state boolean, and writes nothing for `""`.
 *
 * The absent case is the point: `""` means "inherit the global value", and
 * emitting `= false` for it would turn a decision the operator did not make
 * into one they did, pinned in the file where nobody looks for it.
 */
const writeTriStateBool = (
  lines: string[],
  key: string,
  value: "" | "true" | "false",
): void => {
  if (value !== "") writeBoolScalar(lines, key, value === "true");
};

const writeBoolScalar = (lines: string[], key: string, value: boolean): void => {
  lines.push(`${key} = ${value}`);
};

// Render the form (and any preserved extras) as TOML.
export const serializeManifestForm = (
  form: ManifestFormState,
  extras: ManifestExtras = emptyManifestExtras(),
): string => {
  const lines: string[] = [];

  writeStringScalar(lines, "name", form.name.trim());
  writeStringScalar(lines, "version", form.version.trim());
  writeStringScalar(lines, "description", form.description.trim());
  writeStringScalar(lines, "author", form.author.trim());
  writeStringScalar(lines, "module", form.module.trim());
  if (!form.enabled) writeBoolScalar(lines, "enabled", false);
  if (form.priority !== "Normal") writeStringScalar(lines, "priority", form.priority);
  if (form.session_mode !== "persistent") {
    writeStringScalar(lines, "session_mode", form.session_mode);
  }
  if (form.web_search_augmentation !== "auto") {
    writeStringScalar(lines, "web_search_augmentation", form.web_search_augmentation);
  }
  if (!form.show_progress) writeBoolScalar(lines, "show_progress", false);
  if (form.cache_context) writeBoolScalar(lines, "cache_context", true);
  if (form.mcp_disabled) writeBoolScalar(lines, "mcp_disabled", true);
  // Counts are `Option<usize>` / `Option<u32>`: `""` omits the key rather than
  // writing a zero, which would be a limit of nothing.
  // Parsed like the other numeric fields rather than interpolated raw: `-5`,
  // `1.5` and `1e3` are all reachable from the ladder's custom box, and each one
  // made `PATCH /api/agents/{id}` reject the whole document with a 400.
  writeIntegerScalar(
    lines,
    "max_history_messages",
    parseUnsignedTomlInteger(form.max_history_messages),
  );
  writeIntegerScalar(
    lines,
    "max_concurrent_invocations",
    parseUnsignedTomlInteger(form.max_concurrent_invocations),
  );
  if (form.tool_exec_backend !== "") {
    writeStringScalar(lines, "tool_exec_backend", form.tool_exec_backend);
  }
  if (form.profile !== "") writeStringScalar(lines, "profile", form.profile);
  if (form.reconcile_orphans !== "keep") {
    writeStringScalar(lines, "reconcile_orphans", form.reconcile_orphans);
  }
  // Top-level, so they belong here: emitted after the first `[section]` header
  // they would be scoped into that table and silently dropped by the daemon.
  if (form.auto_dream_min_hours.trim() !== "") {
    writeNumberScalar(
      lines,
      "auto_dream_min_hours",
      parseFloatish(form.auto_dream_min_hours),
    );
  }
  if (form.auto_dream_min_sessions.trim() !== "") {
    writeNumberScalar(
      lines,
      "auto_dream_min_sessions",
      parseInteger(form.auto_dream_min_sessions),
    );
  }
  if (form.assignee_wake !== "") {
    writeBoolScalar(lines, "assignee_wake", form.assignee_wake === "true");
  }
  writeStringScalar(lines, "pinned_model", form.pinned_model.trim());
  writeStringScalar(lines, "workspace", form.workspace.trim());
  if (form.skills_disabled) writeBoolScalar(lines, "skills_disabled", true);
  if (form.tools_disabled) writeBoolScalar(lines, "tools_disabled", true);
  if (!form.inherit_parent_context) {
    writeBoolScalar(lines, "inherit_parent_context", false);
  }
  if (!form.generate_identity_files) {
    writeBoolScalar(lines, "generate_identity_files", false);
  }

  if (form.tags.length) lines.push(`tags = ${tomlArray(form.tags)}`);
  if (form.skills.length) lines.push(`skills = ${tomlArray(form.skills)}`);
  if (form.mcp_servers.length) lines.push(`mcp_servers = ${tomlArray(form.mcp_servers)}`);
  if (form.tool_allowlist.length) lines.push(`tool_allowlist = ${tomlArray(form.tool_allowlist)}`);
  if (form.tool_blocklist.length) lines.push(`tool_blocklist = ${tomlArray(form.tool_blocklist)}`);
  if (form.allowed_plugins.length) {
    lines.push(`allowed_plugins = ${tomlArray(form.allowed_plugins)}`);
  }
  // `fallback_models = []` is a TOP-LEVEL key, so it has to be emitted here,
  // with the other top-level arrays and before the first `[section]` header.
  // `null` omits it (inherit the global fallback_providers); a declared `[]`
  // must still emit — it is the disable-all statement, and dropping it would
  // re-enable global fallbacks on an agent pinned to none (#7749).
  // Emitting it after the section headers instead put the bare key inside
  // whichever table was appended last (`[model]` on any manifest the daemon
  // renders, since `toml::to_string_pretty` always writes a `[model]` table).
  // Neither `AgentManifest` nor `ModelConfig` declares `deny_unknown_fields`,
  // so the kernel dropped that `model.fallback_models` silently and the agent
  // went back to inheriting the deployment-wide chain with no error surfaced.
  if (form.fallback_models !== null && form.fallback_models.length === 0) {
    lines.push("fallback_models = []");
  }

  // Schedule — Reactive is the default and emits nothing; tagged variants
  // serialize as the externally-tagged TOML form `schedule = { variant = { … } }`.
  const scheduleLine = renderSchedule(form.schedule, extras.schedule);
  if (scheduleLine) lines.push(scheduleLine);

  if (form.exec_policy_shorthand) {
    writeStringScalar(lines, "exec_policy", form.exec_policy_shorthand);
  }

  // response_format — only emit if non-default.
  const responseFormatLine = renderResponseFormat(form.response_format);
  if (responseFormatLine) lines.push(responseFormatLine);

  // Top-level extras — split scalars (BEFORE table headers) and tables (AFTER).
  // Drop any key the form is about to emit itself, otherwise we'd produce
  // a duplicate key that smol-toml (and the kernel) rejects:
  //   - exec_policy: form emits the shorthand, extras may carry the full table
  //   - response_format: form emits text/json/json_schema, extras may carry
  //     an unmappable `type = "future_format"` table that survived parse
  let filteredTopExtras = extras.topLevel;
  if (form.exec_policy_shorthand) {
    filteredTopExtras = omitKey(filteredTopExtras, "exec_policy");
  }
  if (form.response_format.mode !== "text") {
    filteredTopExtras = omitKey(filteredTopExtras, "response_format");
  }

  const { inline: topInlineExtras, tables: topTableExtras } =
    splitTopLevelExtras(filteredTopExtras);
  for (const line of renderExtraScalars(topInlineExtras)) lines.push(line);

  const deferredSectionExtras: Record<string, TomlTable | TomlTable[]> = {};
  // Section-extras that contain nested tables must NOT be inlined inside
  // the [section] block — a stray `[name]` header would re-anchor TOML
  // scoping for everything that follows. Defer them and emit later with
  // their full dotted key path, e.g. `[model.exotic_subtable]`.
  // Preserved `[workspaces]` entries (mount-based declarations, or malformed
  // non-path rows) re-emit as `[workspaces.<name>]` sub-tables beside the
  // form's rows — routed here rather than through the trailer, which would
  // emit a duplicate `[workspaces]` header.
  if (isTomlTable(topTableExtras.workspaces)) {
    for (const [name, decl] of Object.entries(topTableExtras.workspaces)) {
      // Non-table garbage under a preserved entry is skipped rather than
      // emitted as a header it cannot legally have.
      if (!isTomlTable(decl)) continue;
      deferredSectionExtras[`workspaces.${name}`] = decl;
    }
    delete topTableExtras.workspaces;
  }

  const safeModelExtras = pluckSafeExtras(extras.model, deferredSectionExtras, "model");
  const safeResourceExtras = pluckSafeExtras(extras.resources, deferredSectionExtras, "resources");

  const safeCapabilityExtras = pluckSafeExtras(
    extras.capabilities,
    deferredSectionExtras,
    "capabilities",
  );
  // Only when the form is still emitting a `[thinking]` section: unticking
  // "enabled" is the user deleting the whole table, and the preserved keys go
  // with it rather than stranding a `[thinking]` block the form no longer owns.
  const safeThinkingExtras = form.thinking.enabled
    ? pluckSafeExtras(extras.thinking, deferredSectionExtras, "thinking")
    : {};
  // Same conditional shape as `thinking`: the toggle owns the whole table, so
  // switching it off is the user deleting the section and the preserved keys go
  // with it rather than stranding a block the form no longer writes.
  const safeAutonomousExtras = form.autonomous.enabled
    ? pluckSafeExtras(extras.autonomous, deferredSectionExtras, "autonomous")
    : {};
  const safeChannelOverrideExtras = pluckSafeExtras(
    extras.channel_overrides,
    deferredSectionExtras,
    "channel_overrides",
  );
  const safeSkillWorkshopExtras = pluckSafeExtras(
    extras.skill_workshop,
    deferredSectionExtras,
    "skill_workshop",
  );
  const safeCompactionExtras = pluckSafeExtras(
    extras.compaction,
    deferredSectionExtras,
    "compaction",
  );
  const safeAsyncTaskExtras = pluckSafeExtras(
    extras.async_tasks,
    deferredSectionExtras,
    "async_tasks",
  );
  const safeProactiveMemoryExtras = pluckSafeExtras(
    extras.proactive_memory,
    deferredSectionExtras,
    "proactive_memory",
  );
  const safeRlExportExtras = pluckSafeExtras(extras.rl_export, deferredSectionExtras, "rl_export");
  const safeRoutingExtras = form.routing.enabled
    ? pluckSafeExtras(extras.routing, deferredSectionExtras, "routing")
    : {};

  // [workspaces] — table header, so it is emitted here, after every
  // top-level scalar; a header inside the scalar block would scope the
  // remaining bare keys into the table and silently delete them.

  if (form.workspaces.length) {
    const wsBody: string[] = [];
    for (const ws of form.workspaces) {
      const n = ws.name.trim();
      const p = ws.path.trim();
      if (!n || !p) continue;
      const parts = [`path = ${escapeTomlString(p)}`];
      if (ws.mode === "r") parts.push(`mode = "r"`);
      for (const [key, value] of Object.entries(ws.preserved ?? {})) {
        if (value === null || value === undefined) continue;
        parts.push(`${tomlBareKeyOrQuoted(key)} = ${jsonValueToInlineToml(value)}`);
      }
      wsBody.push(`${tomlBareKeyOrQuoted(n)} = { ${parts.join(", ")} }`);
    }
    if (wsBody.length) lines.push("", "[workspaces]", ...wsBody);
  }

  // [model]
  const modelBody: string[] = [];
  writeStringScalar(modelBody, "provider", form.model.provider.trim());
  writeStringScalar(modelBody, "model", form.model.model.trim());
  writeSystemPrompt(modelBody, form.model.system_prompt);
  writeNumberScalar(modelBody, "temperature", parseSignedFloat(form.model.temperature));
  writeNumberScalar(modelBody, "max_tokens", parseInteger(form.model.max_tokens));
  writeNumberScalar(modelBody, "top_p", parseSignedFloat(form.model.top_p));
  writeNumberScalar(modelBody, "frequency_penalty", parseSignedFloat(form.model.frequency_penalty));
  writeNumberScalar(modelBody, "presence_penalty", parseSignedFloat(form.model.presence_penalty));
  // The two u64 token counts carry as strings, the way the resource quotas
  // already do: their type reaches past JavaScript's safe integer range
  // (TOML's signed-64-bit bound is u64's practical wire ceiling), and the
  // Number-based parseInteger dropped anything past 2^53-1 on the floor —
  // a value the validator now accepts because the file format can carry it.
  writeIntegerScalar(modelBody, "context_window", parseUnsignedTomlInteger(form.model.context_window));
  writeIntegerScalar(modelBody, "max_output_tokens", parseUnsignedTomlInteger(form.model.max_output_tokens));
  writeStringScalar(modelBody, "api_key_env", form.model.api_key_env.trim());
  writeStringScalar(modelBody, "base_url", form.model.base_url.trim());
  // `fixed` is `ModelMode`'s default, so it is the value that must not be
  // written — writing it would record a decision nobody made and pin the agent
  // if that default ever changes.
  if (form.model.mode !== "fixed") writeStringScalar(modelBody, "mode", form.model.mode);
  modelBody.push(...renderRouterOverride(form.model));
  const modelExtras = renderExtraScalars(safeModelExtras);
  if (modelBody.length || modelExtras.length) {
    lines.push("", "[model]", ...modelBody, ...modelExtras);
  }

  // [resources]
  const resourceBody: string[] = [];
  writeIntegerScalar(resourceBody, "max_llm_tokens_per_hour", parseUnsignedTomlInteger(form.resources.max_llm_tokens_per_hour));
  writeNumberScalar(resourceBody, "max_tool_calls_per_minute", parseInteger(form.resources.max_tool_calls_per_minute));
  writeNumberScalar(resourceBody, "max_cost_per_hour_usd", parseFloatish(form.resources.max_cost_per_hour_usd));
  writeNumberScalar(resourceBody, "max_cost_per_day_usd", parseFloatish(form.resources.max_cost_per_day_usd));
  writeNumberScalar(resourceBody, "max_cost_per_month_usd", parseFloatish(form.resources.max_cost_per_month_usd));
  writeIntegerScalar(resourceBody, "max_memory_bytes", parseUnsignedTomlInteger(form.resources.max_memory_bytes));
  writeIntegerScalar(resourceBody, "max_cpu_time_ms", parseUnsignedTomlInteger(form.resources.max_cpu_time_ms));
  writeIntegerScalar(resourceBody, "max_network_bytes_per_hour", parseUnsignedTomlInteger(form.resources.max_network_bytes_per_hour));
  // A fraction of the hourly budget one minute may spend. `parseFloatish`
  // returns null for the absent key and for garbage, so neither is written;
  // in-range values the form does not recognise as a rung pass through
  // unclamped — the runtime does the clamping, at enforcement time.
  writeNumberScalar(resourceBody, "burst_ratio", parseFloatish(form.resources.burst_ratio));
  const resourceExtras = renderExtraScalars(safeResourceExtras);
  if (resourceBody.length || resourceExtras.length) {
    lines.push("", "[resources]", ...resourceBody, ...resourceExtras);
  }

  // [capabilities]
  const capabilityBody: string[] = [];
  if (form.capabilities.network.length) capabilityBody.push(`network = ${tomlArray(form.capabilities.network)}`);
  if (form.capabilities.shell.length) capabilityBody.push(`shell = ${tomlArray(form.capabilities.shell)}`);
  if (form.capabilities.tools.length) capabilityBody.push(`tools = ${tomlArray(form.capabilities.tools)}`);
  // `null` omits the key (unrestricted); `[]` MUST still emit — the empty array is
  // the deny-all declaration, and dropping it would flip the field to unrestricted
  // on a save that never touched it (#7749 review).
  if (form.capabilities.memory_read !== null) capabilityBody.push(`memory_read = ${tomlArray(form.capabilities.memory_read)}`);
  if (form.capabilities.memory_write !== null) capabilityBody.push(`memory_write = ${tomlArray(form.capabilities.memory_write)}`);
  if (form.capabilities.agent_message.length) capabilityBody.push(`agent_message = ${tomlArray(form.capabilities.agent_message)}`);
  if (form.capabilities.ofp_connect.length) capabilityBody.push(`ofp_connect = ${tomlArray(form.capabilities.ofp_connect)}`);
  if (form.capabilities.agent_spawn) writeBoolScalar(capabilityBody, "agent_spawn", true);
  if (form.capabilities.ofp_discover) writeBoolScalar(capabilityBody, "ofp_discover", true);
  // Media capability routing. An empty field is *omitted*, not written as
  // `""` — omission is what the kernel reads as "inherit the global block",
  // and an empty string would pin an empty provider instead.
  for (const key of CAPABILITY_ROUTING_KEYS) {
    const spec = form.capabilities[key].trim();
    if (spec) capabilityBody.push(`${key} = ${escapeTomlString(spec)}`);
  }
  const capabilityExtras = renderExtraScalars(safeCapabilityExtras);
  if (capabilityBody.length || capabilityExtras.length) {
    lines.push("", "[capabilities]", ...capabilityBody, ...capabilityExtras);
  }

  // [thinking]
  if (form.thinking.enabled) {
    const body: string[] = [];
    writeNumberScalar(body, "budget_tokens", parseInteger(form.thinking.budget_tokens));
    writeBoolScalar(body, "stream_thinking", form.thinking.stream_thinking);
    lines.push("", "[thinking]", ...body, ...renderExtraScalars(safeThinkingExtras));
  }

  // [autonomous]
  if (form.autonomous.enabled) {
    const body: string[] = [];
    writeNumberScalar(body, "max_iterations", parseInteger(form.autonomous.max_iterations));
    writeNumberScalar(body, "max_restarts", parseInteger(form.autonomous.max_restarts));
    writeIntegerScalar(body, "heartbeat_interval_secs", parseUnsignedTomlInteger(form.autonomous.heartbeat_interval_secs));
    writeNumberScalar(body, "heartbeat_timeout_secs", parseInteger(form.autonomous.heartbeat_timeout_secs));
    writeIntegerScalar(body, "heartbeat_keep_recent", parseUnsignedTomlInteger(form.autonomous.heartbeat_keep_recent));
    writeStringScalar(body, "heartbeat_channel", form.autonomous.heartbeat_channel.trim());
    writeStringScalar(body, "quiet_hours", form.autonomous.quiet_hours.trim());
    lines.push("", "[autonomous]", ...body, ...renderExtraScalars(safeAutonomousExtras));
  }

  // [routing]
  if (form.routing.enabled) {
    const body: string[] = [];
    writeStringScalar(body, "simple_model", form.routing.simple_model.trim());
    writeStringScalar(body, "medium_model", form.routing.medium_model.trim());
    writeStringScalar(body, "complex_model", form.routing.complex_model.trim());
    writeNumberScalar(body, "simple_threshold", parseInteger(form.routing.simple_threshold));
    writeNumberScalar(body, "complex_threshold", parseInteger(form.routing.complex_threshold));
    lines.push("", "[routing]", ...body, ...renderExtraScalars(safeRoutingExtras));
  }

  // [proactive_memory]
  {
    const body: string[] = [];
    const pm = form.proactive_memory;
    writeTriStateBool(body, "enabled", pm.enabled);
    writeTriStateBool(body, "auto_memorize", pm.auto_memorize);
    writeTriStateBool(body, "auto_retrieve", pm.auto_retrieve);
    writeStringScalar(body, "extraction_model", pm.extraction_model.trim());
    writeTriStateBool(body, "session_scoped_recall", pm.session_scoped_recall);
    writeNumberScalar(body, "min_similarity", parseFloatish(pm.min_similarity));
    writeTriStateBool(body, "allow_self_consolidation", pm.allow_self_consolidation);
    // Emitted when it says something: an all-inherit table with nothing
    // preserved would be a `[proactive_memory]` header that overrides nothing,
    // which reads as a configured section and is not one.
    //
    // The guard covers the extras as well as the body, and that is the
    // invariant this whole family of sections shares: a section's preserved
    // extras are emitted whenever they are non-empty, whatever the form's own
    // body did. Guarding on the body alone silently deleted every key the form
    // has no widget for — the bug the `thinking` and `autonomous` slots were
    // added to fix, reintroduced here by the slot's own guard.
    const proactiveMemoryExtras = renderExtraScalars(safeProactiveMemoryExtras);
    if (body.length || proactiveMemoryExtras.length) {
      lines.push("", "[proactive_memory]", ...body, ...proactiveMemoryExtras);
    }
  }

  // [rl_export]
  {
    const body: string[] = [];
    writeTriStateBool(body, "enabled", form.rl_export);
    const rlExportExtras = renderExtraScalars(safeRlExportExtras);
    if (body.length || rlExportExtras.length) {
      lines.push("", "[rl_export]", ...body, ...rlExportExtras);
    }
  }

  // [async_tasks]
  {
    const body: string[] = [];
    writeNumberScalar(
      body,
      "default_timeout_secs",
      parseInteger(form.async_tasks.default_timeout_secs),
    );
    // The compiled default is `true`, so `true` is the value that must not be
    // written: emitting it would pin the agent against a later change to that
    // default, and would record a decision the operator never made. `false` is
    // a decision, and the only one worth the key.
    if (!form.async_tasks.notify_on_timeout) {
      writeBoolScalar(body, "notify_on_timeout", false);
    }
    const asyncTaskExtras = renderExtraScalars(safeAsyncTaskExtras);
    if (body.length || asyncTaskExtras.length) {
      lines.push("", "[async_tasks]", ...body, ...asyncTaskExtras);
    }
  }

  // [compaction]
  {
    const body: string[] = [];
    const c = form.compaction;
    writeNumberScalar(body, "threshold_messages", parseInteger(c.threshold_messages));
    writeNumberScalar(body, "keep_recent", parseInteger(c.keep_recent));
    writeNumberScalar(body, "max_summary_tokens", parseInteger(c.max_summary_tokens));
    writeNumberScalar(body, "token_threshold_ratio", parseFloatish(c.token_threshold_ratio));
    writeNumberScalar(body, "max_chunk_chars", parseInteger(c.max_chunk_chars));
    writeNumberScalar(body, "max_retries", parseInteger(c.max_retries));
    writeTriStateBool(body, "aggregate_developer_loops", c.aggregate_developer_loops);
    writeNumberScalar(
      body,
      "max_loop_steps_before_aggregate",
      parseInteger(c.max_loop_steps_before_aggregate),
    );
    writeNumberScalar(
      body,
      "strip_reasoning_after_turns",
      parseInteger(c.strip_reasoning_after_turns),
    );
    // The guard covers the extras as well as the body: a table whose keys the
    // form has no widget for — or whose only known key serializes to nothing —
    // would otherwise be dropped whole, preserved keys included. Same form as
    // the `[model]` and `[resources]` guards above.
    const compactionExtras = renderExtraScalars(safeCompactionExtras);
    if (body.length || compactionExtras.length) {
      lines.push("", "[compaction]", ...body, ...compactionExtras);
    }
  }

  // [channel_overrides]
  {
    const body: string[] = [];
    const c = form.channel_overrides;
    writeStringScalar(body, "model", c.model.trim());
    writeStringScalar(body, "system_prompt", c.system_prompt.trim());
    if (c.dm_policy !== "") writeStringScalar(body, "dm_policy", c.dm_policy);
    if (c.group_policy !== "") writeStringScalar(body, "group_policy", c.group_policy);
    if (c.group_trigger_patterns.length) body.push(`group_trigger_patterns = ${tomlArray(c.group_trigger_patterns)}`);
    if (c.reply_precheck) writeBoolScalar(body, "reply_precheck", true);
    writeStringScalar(body, "reply_precheck_model", c.reply_precheck_model.trim());
    writeNumberScalar(body, "rate_limit_per_minute", parseInteger(c.rate_limit_per_minute));
    writeNumberScalar(body, "rate_limit_per_user", parseInteger(c.rate_limit_per_user));
    if (c.threading) writeBoolScalar(body, "threading", true);
    if (c.output_format !== "") writeStringScalar(body, "output_format", c.output_format);
    if (c.usage_footer !== "") writeStringScalar(body, "usage_footer", c.usage_footer);
    if (c.typing_mode !== "") writeStringScalar(body, "typing_mode", c.typing_mode);
    writeNumberScalar(body, "message_debounce_ms", parseInteger(c.message_debounce_ms));
    writeNumberScalar(body, "message_debounce_max_ms", parseInteger(c.message_debounce_max_ms));
    writeNumberScalar(body, "message_debounce_max_buffer", parseInteger(c.message_debounce_max_buffer));
    if (c.clear_done_reaction) writeBoolScalar(body, "clear_done_reaction", true);
    if (c.disable_commands) writeBoolScalar(body, "disable_commands", true);
    if (c.allowed_commands.length) body.push(`allowed_commands = ${tomlArray(c.allowed_commands)}`);
    if (c.blocked_commands.length) body.push(`blocked_commands = ${tomlArray(c.blocked_commands)}`);
    if (c.auto_route !== "off") writeStringScalar(body, "auto_route", c.auto_route);
    writeNumberScalar(body, "auto_route_ttl_minutes", parseInteger(c.auto_route_ttl_minutes));
    writeNumberScalar(body, "auto_route_confidence_threshold", parseInteger(c.auto_route_confidence_threshold));
    writeNumberScalar(body, "auto_route_sticky_bonus", parseInteger(c.auto_route_sticky_bonus));
    writeNumberScalar(body, "auto_route_divergence_count", parseInteger(c.auto_route_divergence_count));
    if (c.prefix_agent_name !== "off") writeStringScalar(body, "prefix_agent_name", c.prefix_agent_name);
    // `default_thread_ownership_enabled` returns true, so `false` is the value
    // worth writing.
    if (!c.thread_ownership_enabled) writeBoolScalar(body, "thread_ownership_enabled", false);
    writeNumberScalar(body, "conversation_ownership_ttl_seconds", parseInteger(c.conversation_ownership_ttl_seconds));
    if (c.conversation_ownership_include_dms) writeBoolScalar(body, "conversation_ownership_include_dms", true);
    // The guard covers the extras as well as the body: a table whose keys the
    // form has no widget for would otherwise be dropped whole, preserved keys
    // included. Same form as the `[model]` and `[resources]` guards above.
    const channelOverrideExtras = renderExtraScalars(safeChannelOverrideExtras);
    if (body.length || channelOverrideExtras.length) {
      lines.push("", "[channel_overrides]", ...body, ...channelOverrideExtras);
    }
  }

  // [skill_workshop]
  {
    const body: string[] = [];
    const w = form.skill_workshop;
    // Each field is written only when it differs from the Rust `Default`, so an
    // agent that has never been configured for the workshop produces no table
    // at all. `auto_capture` is the one that reads backwards: its default is
    // `true`, so `false` is the value worth writing.
    if (w.enabled) writeBoolScalar(body, "enabled", true);
    if (!w.auto_capture) writeBoolScalar(body, "auto_capture", false);
    if (w.approval_policy !== "pending") {
      writeStringScalar(body, "approval_policy", w.approval_policy);
    }
    if (w.review_mode !== "heuristic") {
      writeStringScalar(body, "review_mode", w.review_mode);
    }
    writeNumberScalar(body, "max_pending", parseInteger(w.max_pending));
    writeNumberScalar(body, "max_pending_age_days", parseInteger(w.max_pending_age_days));
    if (w.evolution_mode !== "free") {
      writeStringScalar(body, "evolution_mode", w.evolution_mode);
    }
    // Same guard shape as `[compaction]` and `[channel_overrides]`.
    const skillWorkshopExtras = renderExtraScalars(safeSkillWorkshopExtras);
    if (body.length || skillWorkshopExtras.length) {
      lines.push("", "[skill_workshop]", ...body, ...skillWorkshopExtras);
    }
  }

  // [[fallback_models]]
  for (const fb of form.fallback_models ?? []) {
    const body: string[] = [];
    writeStringScalar(body, "provider", fb.provider.trim());
    writeStringScalar(body, "model", fb.model.trim());
    writeStringScalar(body, "api_key_env", fb.api_key_env.trim());
    writeStringScalar(body, "base_url", fb.base_url.trim());
    // Re-emit provider-specific extras (e.g. Qwen's enable_memory). Same
    // newline-defence as the section-extras path: refuse multi-line
    // output that would re-anchor scoping inside this `[[fallback_models]]`
    // table item.
    body.push(...renderExtraScalars(fb.extras ?? {}));
    if (body.length) lines.push("", "[[fallback_models]]", ...body);
  }

  // [[context_injection]]
  for (const ci of form.context_injection) {
    const body: string[] = [];
    writeStringScalar(body, "name", ci.name.trim());
    writeStringScalar(body, "content", ci.content);
    if (ci.position !== "system") writeStringScalar(body, "position", ci.position);
    writeStringScalar(body, "condition", ci.condition.trim());
    // The un-widgeted keys ride along as inline values — the same merge the
    // preserved stashes get, and the only legal one here: `[[context_injection]]`
    // is an array of tables, so a nested header could not be addressed to this
    // row instead of whichever row the parser would anchor it to.
    for (const [key, value] of Object.entries(ci.preserved ?? {})) {
      if (value === null || value === undefined) continue;
      body.push(`${tomlBareKeyOrQuoted(key)} = ${jsonValueToInlineToml(value)}`);
    }
    if (body.length) lines.push("", "[[context_injection]]", ...body);
  }

  // Deferred section sub-tables (e.g. [model.exotic_subtable]) — must be
  // emitted at top-level scope, not inside the [section] block. Build a
  // nested object so smol-toml uses dotted-key headers like
  // `[model.exotic_subtable]` rather than quoting the dotted name.
  const nestedDeferred: TomlTable = {};
  for (const [dottedKey, value] of Object.entries(deferredSectionExtras)) {
    // Split on the FIRST dot only. `String.split(".", 2)` truncates rather
    // than preserving the remainder, so a preserved name that itself
    // contains a dot (e.g. "workspaces.notes.v2", a legitimate arbitrary
    // user string) lost everything after the second segment and
    // overwrote a sibling entry.
    const dot = dottedKey.indexOf(".");
    if (dot === -1) continue;
    const section = dottedKey.slice(0, dot);
    const subKey = dottedKey.slice(dot + 1);
    if (!isTomlTable(nestedDeferred[section])) {
      nestedDeferred[section] = {};
    }
    (nestedDeferred[section] as TomlTable)[subKey] = value;
  }
  if (Object.keys(nestedDeferred).length > 0) {
    try {
      const block = stringify(nestedDeferred).trimEnd();
      if (block) lines.push("", block);
    } catch {
      // Skip unrenderable values rather than corrupting the document.
    }
  }

  // Top-level extras' sub-tables come last (TOML scoping requires it).
  const trailer = stringifyExtras(topTableExtras);
  if (trailer) lines.push("", trailer.trimEnd());

  return lines.join("\n") + "\n";
};

// Walk a section's extras: scalar/array values stay (safe to inline);
// table-typed values (objects, arrays-of-tables) are moved to `deferred`
// keyed by the full dotted path so they get emitted as proper top-level
// sub-tables. We defer aggressively because smol-toml's stringify
// renders any object value as a multi-line `[name]` block, even when the
// inner content would have fit in inline-table syntax.
const pluckSafeExtras = (
  table: TomlTable,
  deferred: Record<string, TomlTable | TomlTable[]>,
  sectionName: string,
): TomlTable => {
  const safe: TomlTable = {};
  for (const [key, value] of Object.entries(table)) {
    if (isTomlTable(value) || isArrayOfTables(value)) {
      deferred[`${sectionName}.${key}`] = value as TomlTable | TomlTable[];
    } else {
      safe[key] = value;
    }
  }
  return safe;
};

/**
 * `[model] router_override`, as an inline table.
 *
 * Inline rather than a `[model.router_override]` header: that header would land
 * inside the `[model]` block and re-scope every bare key written after it.
 *
 * Nothing is emitted when nothing is set. The Rust field is
 * `Option<AgentRouterOverride>` and every one of its members defaults to "no
 * opinion", so an empty table would be a configured section that configures
 * nothing — and, unlike the tri-state scalars, there is no absent-versus-false
 * distinction to preserve: `fixed = false` is what the absent key already means.
 */
const renderRouterOverride = (m: ManifestFormState["model"]): string[] => {
  const parts: string[] = [];
  if (m.router_fixed) parts.push("fixed = true");
  // Empty is the daemon's "any profile allowed", so it needs no key either.
  if (m.router_allowed_profiles.length) {
    parts.push(`allowed_profiles = ${tomlArray(m.router_allowed_profiles)}`);
  }
  if (m.router_cost_budget) {
    parts.push(`cost_budget = ${escapeTomlString(m.router_cost_budget)}`);
  }
  const fallback = m.router_default_profile.trim();
  if (fallback) parts.push(`default_profile = ${escapeTomlString(fallback)}`);
  // The un-widgeted keys merge back into the single inline table — the same
  // merge response_format's preserved stash gets, and for the same reason:
  // a `[model.router_override.<key>]` header after this bare assignment
  // would re-anchor TOML scoping, so inline is the only legal home.
  for (const [key, value] of Object.entries(m.router_override_preserved ?? {})) {
    if (value === null || value === undefined) continue;
    parts.push(`${tomlBareKeyOrQuoted(key)} = ${jsonValueToInlineToml(value)}`);
  }
  return parts.length ? [`router_override = { ${parts.join(", ")} }`] : [];
};

const renderSchedule = (
  s: ManifestFormState["schedule"],
  preserved: TomlTable,
): string => {
  // The variant's own keys, as inline `key = value` pairs. The preserved keys
  // are spliced into the same inline table rather than emitted under a
  // `[schedule.<variant>]` header: a header here would land after this bare
  // `schedule = …` key and re-scope every line that follows it.
  const inner: string[] = [];
  switch (s.mode) {
    case "reactive":
      // `Reactive` is a unit variant, so serde reads it back as the bare string
      // `schedule = "reactive"` — there is no inner table to splice into.
      // Nothing is lost by that: the parse only preserves out of a variant
      // table, and a variant table is what sets every mode but this one.
      return ""; // default
    case "periodic":
      inner.push(`cron = ${escapeTomlString(s.cron)}`);
      break;
    case "proactive":
      inner.push(`conditions = ${tomlArray(s.conditions)}`);
      break;
    case "continuous": {
      const interval =
        parseUnsignedTomlInteger(s.check_interval_secs) ?? SCHEDULE_DEFAULT_INTERVAL;
      inner.push(`check_interval_secs = ${interval}`);
      break;
    }
  }
  const preservedVariant = preserved[s.mode];
  if (isTomlTable(preservedVariant)) {
    // `jsonValueToInlineToml`, not `renderExtraScalars`: the preserved slot
    // stores every value shape the variant table carried, and the two halves
    // of this field must agree about what survives. `renderExtraScalars`
    // refuses multi-line output — which is every table-typed value — so a
    // preserved key holding a table or an array of tables was stashed by the
    // parse half and dropped by this one. A `[schedule.<key>]` header is not
    // an alternative here: it would land after this bare `schedule = …` key
    // and TOML forbids extending an already-assigned inline table.
    for (const [key, value] of Object.entries(preservedVariant)) {
      if (value === null || value === undefined) continue;
      inner.push(`${tomlBareKeyOrQuoted(key)} = ${jsonValueToInlineToml(value)}`);
    }
  }
  return `schedule = { ${s.mode} = { ${inner.join(", ")} } }`;
};

const renderResponseFormat = (rf: ManifestFormState["response_format"]): string => {
  if (rf.mode === "text") return "";
  // Keys the form does not render merge back into the single inline table
  // here, after the form's own parts. `jsonValueToInlineToml` is the renderer
  // for them because `renderExtraScalars` refuses table-typed values — and a
  // `[custom]` header inside this value would re-anchor TOML scoping anyway.
  const preservedParts = Object.entries(rf.preserved ?? {}).map(
    ([key, value]) => `${tomlBareKeyOrQuoted(key)} = ${jsonValueToInlineToml(value)}`,
  );
  if (rf.mode === "json") {
    const parts = ['type = "json"', ...preservedParts];
    return `response_format = { ${parts.join(", ")} }`;
  }
  // json_schema — schemas can be deeply nested, which makes inline-table
  // syntax brittle. Build the value once via JSON, then convert to TOML
  // using a small recursive emitter that always produces inline syntax.
  // Invalid form state is blocked by validateManifestForm before submit.
  // Keep preview serialization total while sharing the exact same supported schema domain with the validator.
  const schemaValue = parseSupportedJsonSchema(rf.schema) ?? {};
  const parts: string[] = [`type = "json_schema"`, `name = ${escapeTomlString(rf.name || "response")}`];
  parts.push(`schema = ${jsonValueToInlineToml(schemaValue)}`);
  if (rf.strict) parts.push("strict = true");
  parts.push(...preservedParts);
  return `response_format = { ${parts.join(", ")} }`;
};

// Recursively render a JSON value as TOML inline syntax (no [headers],
// no newlines). Suitable for embedding inside an inline-table key.
const jsonValueToInlineToml = (value: unknown): string => {
  if (value === null || value === undefined) return '""'; // TOML has no null
  if (typeof value === "string") return escapeTomlString(value);
  if (typeof value === "boolean") return String(value);
  // smol-toml parses integers past JavaScript's safe range as BigInt, and
  // the preserved stashes carry it whole. Without this arm the value fell
  // through to the object branch and rendered as an empty string — the
  // preserved path CORRUPTING instead of losing. The digits are valid TOML:
  // the format places no bound on integer magnitude.
  if (typeof value === "bigint") return value.toString();
  if (typeof value === "number") {
    return Number.isFinite(value) ? String(value) : "0";
  }
  if (Array.isArray(value)) {
    return `[${value.map(jsonValueToInlineToml).join(", ")}]`;
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).map(
      ([k, v]) => `${tomlBareKeyOrQuoted(k)} = ${jsonValueToInlineToml(v)}`,
    );
    return `{ ${entries.join(", ")} }`;
  }
  return '""';
};

// TOML bare keys allow only [A-Za-z0-9_-]. Anything else needs quoting.
const tomlBareKeyOrQuoted = (key: string): string =>
  /^[A-Za-z0-9_-]+$/.test(key) ? key : escapeTomlString(key);

const stringifyExtras = (extras: TomlTable): string => {
  if (Object.keys(extras).length === 0) return "";
  return stringify(extras);
};

const renderExtraScalars = (extras: TomlTable): string[] => {
  const lines: string[] = [];
  for (const [key, value] of Object.entries(extras)) {
    if (value === null || value === undefined) continue;
    try {
      const rendered = stringify({ [key]: value }).trimEnd();
      // Defensive: refuse multi-line output. Inserted inside a [section]
      // block, an embedded `[name]` header would re-anchor TOML scoping
      // for everything below. Callers should have routed these through
      // pluckSafeExtras already, but belt-and-braces.
      if (rendered.includes("\n")) continue;
      lines.push(rendered);
    } catch {
      // Drop unrenderable values rather than crashing the form preview.
    }
  }
  return lines;
};

const splitTopLevelExtras = (
  extras: TomlTable,
): { inline: TomlTable; tables: TomlTable } => {
  const inline: TomlTable = {};
  const tables: TomlTable = {};
  for (const [key, value] of Object.entries(extras)) {
    if (isTomlTable(value) || isArrayOfTables(value)) {
      tables[key] = value;
    } else {
      inline[key] = value;
    }
  }
  return { inline, tables };
};

const isArrayOfTables = (v: unknown): boolean =>
  Array.isArray(v) && v.length > 0 && v.every((item) => isTomlTable(item));

const omitKey = (table: TomlTable, key: string): TomlTable => {
  const { [key]: _omit, ...rest } = table;
  return rest;
};

const stringifyOrEmpty = (value: unknown): string => {
  if (value === undefined || value === null) return "{}";
  try {
    const out = JSON.stringify(value, null, 2);
    return typeof out === "string" ? out : "{}";
  } catch {
    return "{}";
  }
};

const containsJsonNull = (value: unknown): boolean => {
  if (value === null) return true;
  if (Array.isArray(value)) return value.some(containsJsonNull);
  if (typeof value === "object") return Object.values(value).some(containsJsonNull);
  return false;
};

const hasUnsupportedJsonNumber = (raw: string): boolean => {
  for (let index = 0; index < raw.length; index += 1) {
    if (raw[index] === '"') {
      index += 1;
      while (index < raw.length && raw[index] !== '"') {
        if (raw[index] === "\\") index += 1;
        index += 1;
      }
      continue;
    }
    if (raw[index] !== "-" && !/[0-9]/.test(raw[index])) continue;

    const token = raw
      .slice(index)
      .match(/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/)?.[0];
    if (!token) continue;
    const value = Number(token);
    const mantissa = token.split(/[eE]/, 1)[0];
    const underflowed = value === 0 && /[1-9]/.test(mantissa);
    if (
      !Number.isFinite(value) ||
      underflowed ||
      (Number.isInteger(value) && !Number.isSafeInteger(value))
    ) {
      return true;
    }
    index += token.length - 1;
  }
  return false;
};

// JSON Schema roots are objects or booleans.
// TOML has no null value, so a schema containing a JSON null cannot be represented without changing its meaning and must be rejected before serialization.
const parseSupportedJsonSchema = (raw: string): boolean | Record<string, unknown> | undefined => {
  try {
    const parsed: unknown = JSON.parse(raw);
    if (hasUnsupportedJsonNumber(raw)) return undefined;
    if (typeof parsed === "boolean") return parsed;
    if (isTomlTable(parsed) && !containsJsonNull(parsed)) return parsed;
  } catch {
    // The caller reports the field-level validation error.
  }
  return undefined;
};

// Mirrors `Path::is_absolute` on the platforms the daemon runs on: POSIX
// root, Windows drive letter, or UNC prefix.
const isAbsoluteWorkspacePath = (path: string): boolean =>
  path.startsWith("/") || path.startsWith("\\") || /^[A-Za-z]:[\\/]/.test(path);

// Mirrors the kernel's `WorkspaceMode` alias set (`#[serde(alias = "r",
// alias = "read", alias = "read-only")]` on `ReadOnly`,
// crates/librefang-types/src/agent.rs) and the TUI's
// `canonical_workspace_mode` (crates/librefang-cli/src/tui/event.rs, #7835).
// `"readonly"` is the enum's own canonical serialized form — what
// `toml::to_string_pretty` writes on every save and what the template
// endpoints publish — so it MUST be recognized here, or a read-only shared
// folder silently becomes read-write the moment this form re-saves it.
const READONLY_MODE_ALIASES = new Set(["r", "read", "read-only", "readonly"]);

// TOML bare-key characters only. `expand_workspace_alias`
// (crates/librefang-runtime/src/tool_runner/fs.rs) resolves `@name/rest` by
// matching only the segment before the first `/` against the declared
// name, so a name containing `/` (or other punctuation) serializes to a
// manifest the kernel accepts but the agent can never address.
const WORKSPACE_ALIAS_SAFE_NAME = /^[A-Za-z0-9_-]+$/;

// Names already declared as preserved `[workspaces]` entries (mount-based
// declarations the form can't render) — feeds `validateManifestForm`'s
// collision check. Pulled out as a named helper, rather than inlined at
// the call site, so the exact logic the app runs is covered by a test
// instead of only the validator's own unit tests (#8013: this parameter
// was previously wired nowhere in the app).
export const preservedWorkspaceNamesFromExtras = (extras: ManifestExtras): string[] => {
  const workspaces = extras.topLevel.workspaces;
  return isTomlTable(workspaces) ? Object.keys(workspaces) : [];
};

// Form-validation errors. Returns an empty array when submittable.
//
// `model.provider` / `model.model` are deliberately NOT required here even
// though the form marks them `required` visually: a blank value is the
// documented way an agent inherits the daemon's configured default (the
// `provider_hint` / hint text the form shows next to them says exactly
// this), and `ModelConfig`'s own `""` is written through verbatim by both
// `AgentTypeSpec::apply_to` and `into_new_manifest`. Treating blank as an
// error here made Save silently no-op on every agent (type) that was ever
// created without a pinned provider — there was no toast, just two red
// borders that may be scrolled out of view.
export const validateManifestForm = (
  form: ManifestFormState,
  // Names already present as preserved declarations (e.g. mount-based
  // entries), so a form row cannot silently collide with them.
  preservedWorkspaceNames: Iterable<string> = [],
): string[] => {
  const errors: string[] = [];
  if (!form.name.trim()) errors.push("name");
  if (form.schedule.mode === "periodic" && !form.schedule.cron.trim()) {
    errors.push("schedule.cron");
  }
  if (
    form.schedule.mode === "continuous" &&
    !isPositiveUnsignedTomlInteger(form.schedule.check_interval_secs)
  ) {
    errors.push("schedule.check_interval_secs");
  }
  if (form.response_format.mode === "json_schema") {
    const schema = form.response_format.schema.trim();
    if (!schema || parseSupportedJsonSchema(schema) === undefined) {
      errors.push("response_format.schema");
    }
  }
  // Sampling preferences — same ranges `PATCH /api/agents/{id}/model` enforces
  // (crates/librefang-api/src/routes/agents/config.rs), so an out-of-range
  // value is reported here too instead of reaching the TOML at all (#8112).
  // The two per-agent counts. Blank inherits, anything else must be a whole
  // number the daemon can read into `Option<usize>`; without this the form let
  // `-5` through to the TOML and the operator learned about it as a 400.
  // Each is checked against its own Rust ceiling — the whole-number floor
  // they share is `usize`-shaped, and `max_concurrent_invocations` is `u32`.
  if (!isBlankOrUnsignedTomlInteger(form.max_history_messages)) {
    errors.push("max_history_messages");
  }
  if (!isBlankOrU32TomlInteger(form.max_concurrent_invocations)) {
    errors.push("max_concurrent_invocations");
  }
  // The model's three integer parameters. max_tokens is `Option<u32>` and its
  // ceiling lives in MODEL_PARAM_RANGES (#8332) — the table's number, not a
  // second one here. The two token counts beside it are `Option<u64>`: no
  // typo reaches their ceiling through TOML, so what the validator owes them
  // is the shape — a negative or a non-integer used to pass and parseInteger
  // dropped the key from the file without a word.
  if (
    !isBlankOrUnsignedTomlIntegerAtMost(form.model.max_tokens, MODEL_MAX_TOKENS_CEILING)
  ) {
    errors.push("model.max_tokens");
  }
  if (!isBlankOrUnsignedTomlInteger(form.model.context_window)) {
    errors.push("model.context_window");
  }
  if (!isBlankOrUnsignedTomlInteger(form.model.max_output_tokens)) {
    errors.push("model.max_output_tokens");
  }
  // The rest of the `Option<u32>` inventory — same ceiling, same shape.
  if (!isBlankOrU32TomlInteger(form.autonomous.heartbeat_timeout_secs)) {
    errors.push("autonomous.heartbeat_timeout_secs");
  }
  if (!isBlankOrU32TomlInteger(form.auto_dream_min_sessions)) {
    errors.push("auto_dream_min_sessions");
  }
  if (!isBlankOrU32TomlInteger(form.compaction.max_retries)) {
    errors.push("compaction.max_retries");
  }
  if (!isBlankOrU32TomlInteger(form.compaction.max_loop_steps_before_aggregate)) {
    errors.push("compaction.max_loop_steps_before_aggregate");
  }
  if (!isBlankOrU32TomlInteger(form.compaction.strip_reasoning_after_turns)) {
    errors.push("compaction.strip_reasoning_after_turns");
  }
  if (!isBlankOrU32TomlInteger(form.skill_workshop.max_pending_age_days)) {
    errors.push("skill_workshop.max_pending_age_days");
  }
  // The sampling ranges read from MODEL_PARAM_RANGES, not a second number
  // beside it: the table is the same source the widget's own min/max and the
  // PATCH route's ceiling come from, so a range edit lands everywhere at
  // once instead of the validator quietly keeping yesterday's bounds.
  for (const param of ["temperature", "top_p", "frequency_penalty", "presence_penalty"] as const) {
    const { min, max } = MODEL_PARAM_RANGES[param];
    // A parameter without a table max is unbound above — the table is the
    // source, and the validator follows it rather than inventing a bound.
    if (max === undefined) continue;
    if (!isInRange(form.model[param], min, max)) errors.push(`model.${param}`);
  }
  // Folder rows: duplicate names produce a duplicate TOML key (hard parse
  // failure on the daemon), and `path` mirrors the kernel's rule — relative
  // to workspaces_dir, no `..`. A mount row carries an absolute host path
  // and is not authored here, so only rows are checked.
  //
  // A wholly blank row (freshly added, untouched) is not an error and is
  // dropped silently by the serializer. A half-filled row — only one of
  // name/path set — is a different case: the serializer drops it exactly
  // the same way, so without this check the agent is created believing it
  // has a shared folder it does not have. Flag whichever side is blank.
  const seenWorkspaceNames = new Set<string>(preservedWorkspaceNames);
  for (const ws of form.workspaces) {
    const name = ws.name.trim();
    const wsPath = ws.path.trim();
    if (!name && !wsPath) continue;

    if (!name) {
      errors.push(`workspaces.${ws._uid}.name`);
    } else {
      if (seenWorkspaceNames.has(name) || !WORKSPACE_ALIAS_SAFE_NAME.test(name)) {
        errors.push(`workspaces.${ws._uid}.name`);
      }
      seenWorkspaceNames.add(name);
    }

    if (!wsPath) {
      errors.push(`workspaces.${ws._uid}.path`);
    } else if (isAbsoluteWorkspacePath(wsPath) || wsPath.split(/[\\/]/).includes("..")) {
      errors.push(`workspaces.${ws._uid}.path`);
    }
  }

  return errors;
};

export interface ParseResult {
  ok: true;
  form: ManifestFormState;
  extras: ManifestExtras;
}
export interface ParseError {
  ok: false;
  message: string;
  line?: number;
  column?: number;
}

const asString = (v: unknown): string => (typeof v === "string" ? v : "");
const asNumberString = (v: unknown): string => {
  if (typeof v === "number" && Number.isFinite(v)) return String(v);
  if (typeof v === "bigint") return v.toString();
  return "";
};
const asBoolean = (v: unknown, fallback: boolean): boolean =>
  typeof v === "boolean" ? v : fallback;
const asStringArray = (v: unknown): string[] => {
  if (!Array.isArray(v)) return [];
  return v.filter((x): x is string => typeof x === "string");
};
const containsBigInt = (value: unknown): boolean => {
  if (typeof value === "bigint") return true;
  if (Array.isArray(value)) return value.some(containsBigInt);
  if (isTomlTable(value)) return Object.values(value).some(containsBigInt);
  return false;
};
const asEnum = <T extends readonly string[]>(
  v: unknown,
  allowed: T,
  fallback: T[number],
): T[number] => {
  if (typeof v === "string" && (allowed as readonly string[]).includes(v)) {
    return v as T[number];
  }
  return fallback;
};

/**
 * Like `asEnum`, for a key whose Rust type is `Option<Enum>`.
 *
 * An absent key is a state of its own — inherit — and not one of the enum's
 * variants, so `""` is a legitimate result that `asEnum`'s signature (its
 * fallback must be one of `allowed`) cannot express. Widening `asEnum` instead
 * would make every one of its callers handle a `""` their field cannot hold.
 */
/**
 * A Rust `Option<bool>` as the form holds it: `""` inherits, the two strings
 * are an explicit override. An explicit `false` is a statement and has to
 * survive, which is why this is not a plain boolean with a default.
 */
const asTriStateBool = (v: unknown): "" | "true" | "false" =>
  typeof v === "boolean" ? (v ? "true" : "false") : "";

const asOptionalEnum = <T extends readonly string[]>(
  v: unknown,
  allowed: T,
): T[number] | "" =>
  typeof v === "string" && (allowed as readonly string[]).includes(v)
    ? (v as T[number])
    : "";

export const parseManifestToml = (toml: string): ParseResult | ParseError => {
  let parsed: TomlTable;
  try {
    parsed = parse(toml, { integersAsBigInt: "asNeeded" });
  } catch (e) {
    if (e instanceof TomlError) {
      return { ok: false, message: e.message, line: e.line, column: e.column };
    }
    return { ok: false, message: e instanceof Error ? e.message : String(e) };
  }

  if (
    isTomlTable(parsed.response_format) &&
    asString(parsed.response_format.type) === "json_schema" &&
    containsBigInt(parsed.response_format.schema)
  ) {
    return {
      ok: false,
      message: "json_schema_unsafe_integer",
    };
  }

  const form = emptyManifestForm();
  const extras = emptyManifestExtras();
  let parsedUid = 0;
  const generateParsedUid = (): string => `parsed-${++parsedUid}`;

  form.name = asString(parsed.name);
  form.version = asString(parsed.version) || form.version;
  form.description = asString(parsed.description);
  form.author = asString(parsed.author);
  form.module = asString(parsed.module) || form.module;
  form.enabled = asBoolean(parsed.enabled, true);
  form.priority = asEnum(parsed.priority, PRIORITIES, "Normal");
  form.session_mode = asEnum(parsed.session_mode, SESSION_MODES, "persistent");
  form.web_search_augmentation = asEnum(
    parsed.web_search_augmentation,
    WEB_SEARCH_MODES,
    "auto",
  );
  form.show_progress = asBoolean(parsed.show_progress, true);
  form.cache_context = asBoolean(parsed.cache_context, false);
  form.mcp_disabled = asBoolean(parsed.mcp_disabled, false);
  form.max_history_messages = asNumberString(parsed.max_history_messages);
  form.max_concurrent_invocations = asNumberString(parsed.max_concurrent_invocations);
  form.tool_exec_backend = asOptionalEnum(parsed.tool_exec_backend, TOOL_EXEC_BACKENDS);
  form.profile = asOptionalEnum(parsed.profile, TOOL_PROFILES);
  form.reconcile_orphans = asEnum(parsed.reconcile_orphans, ORPHAN_POLICIES, "keep");
  form.assignee_wake = asTriStateBool(parsed.assignee_wake);
  form.pinned_model = asString(parsed.pinned_model);
  form.workspace = asString(parsed.workspace);
  form.skills_disabled = asBoolean(parsed.skills_disabled, false);
  form.tools_disabled = asBoolean(parsed.tools_disabled, false);
  form.inherit_parent_context = asBoolean(parsed.inherit_parent_context, true);
  form.generate_identity_files = asBoolean(parsed.generate_identity_files, true);
  form.tags = asStringArray(parsed.tags);
  form.skills = asStringArray(parsed.skills);
  form.mcp_servers = asStringArray(parsed.mcp_servers);
  form.tool_allowlist = asStringArray(parsed.tool_allowlist);
  form.tool_blocklist = asStringArray(parsed.tool_blocklist);
  form.allowed_plugins = asStringArray(parsed.allowed_plugins);
  form.schedule = parseScheduleField(parsed.schedule);
  // The chosen variant's unmatched keys are preserved rather than consumed —
  // the same treatment every other table the form owns gets, and for the same
  // underlying reason: forward compatibility. The daemon rejects an unknown
  // key anywhere inside schedule today — measured against the parse the PATCH
  // runs, `toml::from_str::<AgentManifest>`: a key inside the variant comes
  // back as `unexpected keys in table: zz, available keys: cron` — so a
  // preserved key is never a field it was reading — what preservation buys
  // is a field a future
  // manifest carries surviving an old editor's save. The `[schedule]` root is
  // the one level with nothing to preserve: `ScheduleMode` is an
  // externally-tagged enum, so even a sibling key beside the variant is a
  // document the daemon rejects outright.
  if (isTomlTable(parsed.schedule) && form.schedule.mode !== "reactive") {
    const variantTable = parsed.schedule[form.schedule.mode];
    const knownKeys = SCHEDULE_VARIANT_KEYS[form.schedule.mode];
    if (knownKeys && isTomlTable(variantTable)) {
      const preserved = stripKnown(variantTable, knownKeys);
      if (Object.keys(preserved).length) {
        extras.schedule = { [form.schedule.mode]: preserved };
      }
    }
  }
  form.exec_policy_shorthand = parseExecPolicyShorthand(parsed.exec_policy);
  form.response_format = parseResponseFormatField(parsed.response_format);

  // Extras for top-level: capture exec_policy only when it's a table
  // (the form owns the shorthand string form).
  const topExtras: TomlTable = {};
  for (const [k, v] of Object.entries(parsed)) {
    if (FORM_TOP_LEVEL_KEYS.has(k)) continue;
    topExtras[k] = v;
  }
  // exec_policy as a full table (not a shorthand string) is preserved in extras.
  if (isTomlTable(parsed.exec_policy)) topExtras.exec_policy = parsed.exec_policy;
  // response_format the form cannot re-emit goes back into extras to avoid
  // silent loss. Mapped tables (`type = "json"` / `"json_schema"`) instead
  // stash their un-owned keys on the form state — parseResponseFormatField —
  // because the form re-emits the whole table as one inline assignment, and
  // an extras copy alongside it would be the double-emission the
  // mutual-exclusion filter below exists to prevent. Text mode renders
  // nothing, so there any table present survives only through extras; the
  // old guard's `type !== "text"` carve-out dropped `{ type = "text", … }`
  // tables whole, unknown keys included.
  if (isTomlTable(parsed.response_format) && form.response_format.mode === "text") {
    topExtras.response_format = parsed.response_format;
  }
  extras.topLevel = topExtras;

  // [model]
  const modelTable = isTomlTable(parsed.model) ? parsed.model : {};
  form.model.provider = asString(modelTable.provider);
  form.model.model = asString(modelTable.model);
  form.model.system_prompt = asString(modelTable.system_prompt);
  form.model.temperature = asNumberString(modelTable.temperature);
  form.model.max_tokens = asNumberString(modelTable.max_tokens);
  form.model.top_p = asNumberString(modelTable.top_p);
  form.model.frequency_penalty = asNumberString(modelTable.frequency_penalty);
  form.model.presence_penalty = asNumberString(modelTable.presence_penalty);
  form.model.context_window = asNumberString(modelTable.context_window);
  form.model.max_output_tokens = asNumberString(modelTable.max_output_tokens);
  form.model.api_key_env = asString(modelTable.api_key_env);
  form.model.base_url = asString(modelTable.base_url);
  // The profile router's per-agent settings live in `[model]` — which is why
  // this form had to grow them: they are manifest fields, and the routing
  // panel was the only surface that could write them.
  form.model.mode = asEnum(modelTable.mode, MODEL_MODES, "fixed");
  const routerOverride = isTomlTable(modelTable.router_override)
    ? modelTable.router_override
    : {};
  form.model.router_fixed = asBoolean(routerOverride.fixed, false);
  form.model.router_allowed_profiles = asStringArray(routerOverride.allowed_profiles);
  form.model.router_cost_budget = asOptionalEnum(routerOverride.cost_budget, COST_TIERS);
  form.model.router_default_profile = asString(routerOverride.default_profile);
  const routerPreserved = stripKnown(routerOverride, ROUTER_OVERRIDE_KEYS);
  if (Object.keys(routerPreserved).length) {
    form.model.router_override_preserved = routerPreserved;
  }
  extras.model = stripKnown(modelTable, FORM_MODEL_KEYS);

  // [[fallback_models]] — capture provider-specific flatten extras too,
  // so e.g. Qwen's enable_memory survives a TOML→Form→TOML round-trip.
  // `undefined` is the absent key (inherit the global fallback_providers → null);
  // a declared empty array is the disable-all statement and must stay `[]` (#7749).
  form.fallback_models = parsed.fallback_models === undefined
    ? null
    : (parsed.fallback_models as unknown[])
        .filter(isTomlTable)
        .map((fb) => ({
      _uid: generateParsedUid(),
      provider: asString(fb.provider),
      model: asString(fb.model),
      api_key_env: asString(fb.api_key_env),
      base_url: asString(fb.base_url),
      extras: stripKnown(fb, FALLBACK_MODEL_KEYS),
    }));


  // [resources]
  const resourceTable = isTomlTable(parsed.resources) ? parsed.resources : {};
  form.resources.max_llm_tokens_per_hour = asNumberString(resourceTable.max_llm_tokens_per_hour);
  form.resources.max_tool_calls_per_minute = asNumberString(resourceTable.max_tool_calls_per_minute);
  form.resources.max_cost_per_hour_usd = asNumberString(resourceTable.max_cost_per_hour_usd);
  form.resources.max_cost_per_day_usd = asNumberString(resourceTable.max_cost_per_day_usd);
  form.resources.max_cost_per_month_usd = asNumberString(resourceTable.max_cost_per_month_usd);
  form.resources.max_memory_bytes = asNumberString(resourceTable.max_memory_bytes);
  form.resources.max_cpu_time_ms = asNumberString(resourceTable.max_cpu_time_ms);
  form.resources.max_network_bytes_per_hour = asNumberString(resourceTable.max_network_bytes_per_hour);
  form.resources.burst_ratio = asNumberString(resourceTable.burst_ratio);
  extras.resources = stripKnown(resourceTable, FORM_RESOURCE_KEYS);

  // [capabilities]
  const capTable = isTomlTable(parsed.capabilities) ? parsed.capabilities : {};
  form.capabilities.network = asStringArray(capTable.network);
  form.capabilities.shell = asStringArray(capTable.shell);
  form.capabilities.tools = asStringArray(capTable.tools);
  // `undefined` is the absent key (unrestricted → null); a declared `[]` stays
  // `[]` — the deny survives the round trip instead of silently reopening (#7749).
  form.capabilities.memory_read = capTable.memory_read === undefined ? null : asStringArray(capTable.memory_read);
  form.capabilities.memory_write = capTable.memory_write === undefined ? null : asStringArray(capTable.memory_write);
  form.capabilities.agent_message = asStringArray(capTable.agent_message);
  form.capabilities.ofp_connect = asStringArray(capTable.ofp_connect);
  form.capabilities.agent_spawn = asBoolean(capTable.agent_spawn, false);
  form.capabilities.ofp_discover = asBoolean(capTable.ofp_discover, false);
  for (const key of CAPABILITY_ROUTING_KEYS) {
    form.capabilities[key] = readCapabilityRouting(capTable, key);
  }
  extras.capabilities = stripKnown(capTable, FORM_CAPABILITY_KEYS);

  // [thinking]
  if (isTomlTable(parsed.thinking)) {
    form.thinking.enabled = true;
    form.thinking.budget_tokens = asNumberString(parsed.thinking.budget_tokens);
    form.thinking.stream_thinking = asBoolean(parsed.thinking.stream_thinking, false);
    extras.thinking = stripKnown(parsed.thinking, FORM_THINKING_KEYS);
  }

  // [autonomous]
  if (isTomlTable(parsed.autonomous)) {
    const a = parsed.autonomous;
    form.autonomous.enabled = true;
    form.autonomous.max_iterations = asNumberString(a.max_iterations);
    form.autonomous.max_restarts = asNumberString(a.max_restarts);
    form.autonomous.heartbeat_interval_secs = asNumberString(a.heartbeat_interval_secs);
    form.autonomous.heartbeat_timeout_secs = asNumberString(a.heartbeat_timeout_secs);
    form.autonomous.heartbeat_keep_recent = asNumberString(a.heartbeat_keep_recent);
    form.autonomous.heartbeat_channel = asString(a.heartbeat_channel);
    form.autonomous.quiet_hours = asString(a.quiet_hours);
    extras.autonomous = stripKnown(a, FORM_AUTONOMOUS_KEYS);
  }

  form.auto_dream_min_hours = asNumberString(parsed.auto_dream_min_hours);
  form.auto_dream_min_sessions = asNumberString(parsed.auto_dream_min_sessions);
  if (isTomlTable(parsed.rl_export)) {
    form.rl_export = asTriStateBool(parsed.rl_export.enabled);
    extras.rl_export = stripKnown(parsed.rl_export, new Set(["enabled"]));
  }
  if (isTomlTable(parsed.async_tasks)) {
    const at = parsed.async_tasks;
    form.async_tasks.default_timeout_secs = asNumberString(at.default_timeout_secs);
    // An absent key reads as the daemon's default, not as `false`: the daemon
    // notifies, and showing that as "off" invited the operator to turn on
    // something that was already on.
    form.async_tasks.notify_on_timeout = asBoolean(at.notify_on_timeout, true);
    extras.async_tasks = stripKnown(
      at,
      new Set(["default_timeout_secs", "notify_on_timeout"]),
    );
  }

  // [channel_overrides]
  if (isTomlTable(parsed.channel_overrides)) {
    const c = parsed.channel_overrides;
    form.channel_overrides.model = asString(c.model);
    form.channel_overrides.system_prompt = asString(c.system_prompt);
    form.channel_overrides.dm_policy = asOptionalEnum(c.dm_policy, CHANNEL_ENUMS.dm_policy);
    form.channel_overrides.group_policy = asOptionalEnum(c.group_policy, CHANNEL_ENUMS.group_policy);
    form.channel_overrides.group_trigger_patterns = asStringArray(c.group_trigger_patterns);
    form.channel_overrides.reply_precheck = asBoolean(c.reply_precheck, false);
    form.channel_overrides.reply_precheck_model = asString(c.reply_precheck_model);
    form.channel_overrides.rate_limit_per_minute = asNumberString(c.rate_limit_per_minute);
    form.channel_overrides.rate_limit_per_user = asNumberString(c.rate_limit_per_user);
    form.channel_overrides.threading = asBoolean(c.threading, false);
    form.channel_overrides.output_format = asOptionalEnum(c.output_format, CHANNEL_ENUMS.output_format);
    form.channel_overrides.usage_footer = asOptionalEnum(c.usage_footer, CHANNEL_ENUMS.usage_footer);
    form.channel_overrides.typing_mode = asOptionalEnum(c.typing_mode, CHANNEL_ENUMS.typing_mode);
    form.channel_overrides.message_debounce_ms = asNumberString(c.message_debounce_ms);
    form.channel_overrides.message_debounce_max_ms = asNumberString(c.message_debounce_max_ms);
    form.channel_overrides.message_debounce_max_buffer = asNumberString(c.message_debounce_max_buffer);
    form.channel_overrides.clear_done_reaction = asBoolean(c.clear_done_reaction, false);
    form.channel_overrides.disable_commands = asBoolean(c.disable_commands, false);
    form.channel_overrides.allowed_commands = asStringArray(c.allowed_commands);
    form.channel_overrides.blocked_commands = asStringArray(c.blocked_commands);
    form.channel_overrides.auto_route = asEnum(c.auto_route, CHANNEL_ENUMS.auto_route, "off");
    form.channel_overrides.auto_route_ttl_minutes = asNumberString(c.auto_route_ttl_minutes);
    form.channel_overrides.auto_route_confidence_threshold = asNumberString(c.auto_route_confidence_threshold);
    form.channel_overrides.auto_route_sticky_bonus = asNumberString(c.auto_route_sticky_bonus);
    form.channel_overrides.auto_route_divergence_count = asNumberString(c.auto_route_divergence_count);
    form.channel_overrides.prefix_agent_name = asEnum(c.prefix_agent_name, CHANNEL_ENUMS.prefix_agent_name, "off");
    form.channel_overrides.thread_ownership_enabled = asBoolean(c.thread_ownership_enabled, true);
    form.channel_overrides.conversation_ownership_ttl_seconds = asNumberString(c.conversation_ownership_ttl_seconds);
    form.channel_overrides.conversation_ownership_include_dms = asBoolean(c.conversation_ownership_include_dms, false);
    extras.channel_overrides = stripKnown(c, FORM_CHANNEL_OVERRIDE_KEYS);
  }

  // [skill_workshop]
  if (isTomlTable(parsed.skill_workshop)) {
    const w = parsed.skill_workshop;
    form.skill_workshop.enabled = asBoolean(w.enabled, false);
    // `true` is the Rust default, so an absent key means capture is ON.
    form.skill_workshop.auto_capture = asBoolean(w.auto_capture, true);
    form.skill_workshop.approval_policy = asEnum(
      w.approval_policy,
      SKILL_APPROVAL_POLICIES,
      "pending",
    );
    form.skill_workshop.review_mode = asEnum(w.review_mode, SKILL_REVIEW_MODES, "heuristic");
    form.skill_workshop.max_pending = asNumberString(w.max_pending);
    form.skill_workshop.max_pending_age_days = asNumberString(w.max_pending_age_days);
    form.skill_workshop.evolution_mode = asEnum(
      w.evolution_mode,
      SKILL_EVOLUTION_MODES,
      "free",
    );
    extras.skill_workshop = stripKnown(w, FORM_SKILL_WORKSHOP_KEYS);
  }

  // [compaction]
  if (isTomlTable(parsed.compaction)) {
    const c = parsed.compaction;
    form.compaction.threshold_messages = asNumberString(c.threshold_messages);
    form.compaction.keep_recent = asNumberString(c.keep_recent);
    form.compaction.max_summary_tokens = asNumberString(c.max_summary_tokens);
    form.compaction.token_threshold_ratio = asNumberString(c.token_threshold_ratio);
    form.compaction.max_chunk_chars = asNumberString(c.max_chunk_chars);
    form.compaction.max_retries = asNumberString(c.max_retries);
    form.compaction.aggregate_developer_loops = asTriStateBool(
      c.aggregate_developer_loops,
    );
    form.compaction.max_loop_steps_before_aggregate = asNumberString(
      c.max_loop_steps_before_aggregate,
    );
    form.compaction.strip_reasoning_after_turns = asNumberString(
      c.strip_reasoning_after_turns,
    );
    extras.compaction = stripKnown(c, FORM_COMPACTION_KEYS);
  }

  // [proactive_memory]
  if (isTomlTable(parsed.proactive_memory)) {
    const pm = parsed.proactive_memory;
    form.proactive_memory.enabled = asTriStateBool(pm.enabled);
    form.proactive_memory.auto_memorize = asTriStateBool(pm.auto_memorize);
    form.proactive_memory.auto_retrieve = asTriStateBool(pm.auto_retrieve);
    form.proactive_memory.extraction_model = asString(pm.extraction_model);
    form.proactive_memory.session_scoped_recall = asTriStateBool(pm.session_scoped_recall);
    form.proactive_memory.min_similarity = asNumberString(pm.min_similarity);
    form.proactive_memory.allow_self_consolidation = asTriStateBool(
      pm.allow_self_consolidation,
    );
    extras.proactive_memory = stripKnown(pm, FORM_PROACTIVE_MEMORY_KEYS);
  }

  // [routing]
  if (isTomlTable(parsed.routing)) {
    const r = parsed.routing;
    form.routing.enabled = true;
    form.routing.simple_model = asString(r.simple_model);
    form.routing.medium_model = asString(r.medium_model);
    form.routing.complex_model = asString(r.complex_model);
    form.routing.simple_threshold = asNumberString(r.simple_threshold);
    form.routing.complex_threshold = asNumberString(r.complex_threshold);
    extras.routing = stripKnown(r, FORM_ROUTING_KEYS);
  }

  // [[context_injection]]
  if (Array.isArray(parsed.context_injection)) {
    form.context_injection = parsed.context_injection
      .filter(isTomlTable)
      .map((ci) => {
        const row: ManifestFormState["context_injection"][number] = {
          _uid: generateParsedUid(),
          name: asString(ci.name),
          content: asString(ci.content),
          position: asEnum(ci.position, INJECTION_POSITIONS, "system"),
          condition: asString(ci.condition),
        };
        const preserved = stripKnown(ci, CONTEXT_INJECTION_KEYS);
        if (Object.keys(preserved).length) row.preserved = preserved;
        return row;
      });
  }

  // Only `path`-based declarations become rows. A `mount` entry points at an
  // absolute host directory; rewriting it as an empty `path` and dropping it
  // in the incomplete-row filter would silently delete the declaration on
  // save, so the entry is preserved verbatim in extras instead (mirrors the
  // TUI editor, #7835).
  if (isTomlTable(parsed.workspaces)) {
    const preservedWorkspaces: TomlTable = {};
    for (const [name, v] of Object.entries(parsed.workspaces)) {
      if (isTomlTable(v) && typeof (v as TomlTable).path === "string") {
        const row: ManifestFormState["workspaces"][number] = {
          _uid: generateParsedUid(),
          name,
          path: (v as TomlTable).path as string,
          mode: READONLY_MODE_ALIASES.has(asString((v as TomlTable).mode))
            ? ("r" as const)
            : ("rw" as const),
        };
        const preserved = stripKnown(v, WORKSPACE_ROW_KEYS);
        if (Object.keys(preserved).length) row.preserved = preserved;
        form.workspaces.push(row);
      } else {
        preservedWorkspaces[name] = v;
      }
    }
    if (Object.keys(preservedWorkspaces).length) {
      extras.topLevel.workspaces = preservedWorkspaces;
    }
  }

  return { ok: true, form, extras };
};

const isTomlTable = (v: unknown): v is TomlTable =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const stripKnown = (table: TomlTable, knownKeys: Set<string>): TomlTable => {
  const out: TomlTable = {};
  for (const [key, value] of Object.entries(table)) {
    if (!knownKeys.has(key)) out[key] = value;
  }
  return out;
};

const parseScheduleField = (raw: unknown): ManifestFormState["schedule"] => {
  if (typeof raw === "string") return { mode: "reactive" };
  if (!isTomlTable(raw)) return { mode: "reactive" };
  if (isTomlTable(raw.periodic)) {
    return { mode: "periodic", cron: asString(raw.periodic.cron) };
  }
  if (isTomlTable(raw.proactive)) {
    return { mode: "proactive", conditions: asStringArray(raw.proactive.conditions) };
  }
  if (isTomlTable(raw.continuous)) {
    return {
      mode: "continuous",
      check_interval_secs:
        asNumberString(raw.continuous.check_interval_secs) || SCHEDULE_DEFAULT_INTERVAL,
    };
  }
  return { mode: "reactive" };
};

// exec_policy_lenient on the kernel side (serde_compat.rs) accepts
// aliases for each canonical mode. The form's dropdown only knows the
// canonical names, so normalize aliases at the parse boundary —
// otherwise the alias spelling rounds-trips to an empty shorthand and
// the user's intent (deny / allowlist / full) is silently lost.
const EXEC_POLICY_ALIASES: Record<string, ManifestFormState["exec_policy_shorthand"]> = {
  none: "deny",
  disabled: "deny",
  restricted: "allowlist",
  all: "full",
  unrestricted: "full",
};

const parseExecPolicyShorthand = (
  raw: unknown,
): ManifestFormState["exec_policy_shorthand"] => {
  if (typeof raw !== "string") return "";
  // Lowercased first, because the kernel lowercases it: `exec_policy_lenient`
  // normalises through `to_lowercase()` before mapping
  // (`crates/librefang-types/src/serde_compat.rs:262`, wired in at
  // `agent.rs:1347`), so `"Deny"` and `"FULL"` are valid manifests the runtime
  // honours. Matching exactly here read them as a spelling the form did not
  // know, returned "", and dropped the key on the next save — an agent whose
  // policy was `"Deny"` came back with none, and one carrying `shell_exec` is
  // promoted to `Full` when none is present
  // (`kernel/spawn.rs:236-250`, `kernel/boot.rs:2690-2705`).
  const spelling = raw.toLowerCase();
  if ((EXEC_SHORTHANDS as readonly string[]).includes(spelling)) {
    return spelling as ManifestFormState["exec_policy_shorthand"];
  }
  return EXEC_POLICY_ALIASES[spelling] ?? "";
};

// The keys the form re-emits for each mapped mode. Everything else inside a
// mapped table is stashed on the form state and merged back when the field
// renders, so an unknown key alongside `type = "json"` no longer vanishes on
// save — the same preservation every section table gets, adapted to a field
// whose whole value is one inline table.
const RESPONSE_FORMAT_JSON_KEYS = new Set(["type"]);
const RESPONSE_FORMAT_JSON_SCHEMA_KEYS = new Set(["type", "name", "schema", "strict"]);

/**
 * Attach the un-owned keys of the parsed table to the mapped form state.
 *
 * Empty means nothing to stash, and the returned object stays exactly the
 * mapped one — so a table the form fully understands carries no hidden
 * payload around.
 */
const withResponseFormatPreserved = (
  mapped: { mode: "json" } | { mode: "json_schema"; name: string; schema: string; strict: boolean },
  raw: TomlTable,
  owned: Set<string>,
): ManifestFormState["response_format"] => {
  const preserved = stripKnown(raw, owned);
  if (!Object.keys(preserved).length) return mapped;
  return { ...mapped, preserved };
};

const parseResponseFormatField = (raw: unknown): ManifestFormState["response_format"] => {
  if (!isTomlTable(raw)) return { mode: "text" };
  const type = asString(raw.type);
  if (type === "json") {
    return withResponseFormatPreserved({ mode: "json" }, raw, RESPONSE_FORMAT_JSON_KEYS);
  }
  if (type === "json_schema") {
    return withResponseFormatPreserved(
      {
        mode: "json_schema",
        name: asString(raw.name),
        // JSON.stringify(undefined) returns undefined (not a string!), which
        // would break the `schema: string` type and trigger React's
        // uncontrolled→controlled warning when fed to <textarea value={…}>.
        // Default to `{}` whenever the source schema is missing or
        // unrenderable.
        schema: stringifyOrEmpty(raw.schema),
        strict: asBoolean(raw.strict, false),
      },
      raw,
      RESPONSE_FORMAT_JSON_SCHEMA_KEYS,
    );
  }
  return { mode: "text" };
};
