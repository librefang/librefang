import {
  Children,
  Fragment,
  createContext,
  isValidElement,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ChevronDown, Plus, RotateCcw, Trash2, X } from "lucide-react";
import {
  AUTO_ROUTE_STRATEGIES,
  CAPABILITY_ROUTING_KEYS,
  CONTEXT_ENGINE_NAMES,
  DM_POLICIES,
  GROUP_POLICIES,
  HOOK_FAILURE_POLICIES,
  HOOK_RUNTIMES,
  JSON_ROW_TYPES,
  OUTPUT_FORMATS,
  PREFIX_STYLES,
  TYPING_MODES,
  USAGE_FOOTERS,
  formatPreservedValue,
  generateUid,
} from "../lib/agentManifest";
import type { JsonRowType, ManifestExtras, ManifestFormState } from "../lib/agentManifest";
import type { ModelRoutingInertReason } from "../api";
import { isToolAdmittedByCapabilities } from "../lib/toolGrants";
import {
  applyRoutingEngine,
  ROUTING_TIER_DEFAULTS,
  routingEngineOf,
  routingTierModel,
  type RoutingEngine,
} from "../lib/routingEngine";

/// The tri-state caption for a memory capability field (#7749 review):
/// `null` is the omitted key (unrestricted), `[]` is the declared-empty deny.
/// Only shown when the list is empty — a non-empty list speaks for itself.
/// The toggle exists because an empty tag input cannot carry the distinction.
function MemoryScopeNote({
  value,
  onSet,
}: {
  value: string[] | null;
  onSet: (next: string[] | null) => void;
}) {
  const { t } = useTranslation();
  const isEmpty = (value ?? []).length === 0;
  if (!isEmpty) return null;
  if (value === null) {
    return (
      <p className="mt-1 text-xs text-text-dim">
        {t("agents.form.memory_scope_unrestricted")}{" "}
        <button
          type="button"
          className="underline hover:text-brand"
          onClick={() => onSet([])}
        >
          {t("agents.form.memory_scope_restrict")}
        </button>
      </p>
    );
  }
  return (
    <p className="mt-1 text-xs text-warning">
      {t("agents.form.memory_scope_denied")}{" "}
      <button
        type="button"
        className="underline hover:text-brand"
        onClick={() => onSet(null)}
      >
        {t("agents.form.memory_scope_unrestrict")}
      </button>
    </p>
  );
}
import { AgentIdentityFileEditor } from "./AgentIdentityFileEditor";
import { MultiSelectCmdk } from "./ui/MultiSelectCmdk";
import { ModelParamField } from "./ui/ModelParamField";
import { CollapsibleSection, FieldCountBadge } from "./ui/CollapsibleSection";
import type { CollapsibleSectionProps } from "./ui/CollapsibleSection";
import { Field } from "./ui/Field";
import { ModelPicker } from "./ui/ModelPicker";
import { StepLadderInput } from "./ui/StepLadderInput";
import {
  ASYNC_TASK_TIMEOUT_LADDER,
  BURST_RATIO_LADDER,
  CHANNEL_DEBOUNCE_BUFFER_LADDER,
  CHANNEL_DEBOUNCE_MAX_LADDER,
  CHANNEL_DEBOUNCE_MS_LADDER,
  CHANNEL_RATE_LIMIT_LADDER,
  CHANNEL_ROUTE_BONUS_LADDER,
  CHANNEL_ROUTE_CONFIDENCE_LADDER,
  CHANNEL_ROUTE_DIVERGENCE_LADDER,
  CHANNEL_ROUTE_TTL_LADDER,
  CHANNEL_THREAD_OWNERSHIP_TTL_LADDER,
  EXEC_NO_OUTPUT_TIMEOUT_LADDER,
  EXEC_OUTPUT_BYTES_LADDER,
  EXEC_TIMEOUT_LADDER,
  HOOK_CACHE_TTL_LADDER,
  HOOK_CIRCUIT_FAILURES_LADDER,
  HOOK_CIRCUIT_RESET_LADDER,
  HOOK_MEMORY_MB_LADDER,
  HOOK_PRIORITY_LADDER,
  HOOK_QUEUE_DEPTH_LADDER,
  HOOK_RETRIES_LADDER,
  AUTO_DREAM_MIN_HOURS_LADDER,
  AUTO_DREAM_MIN_SESSIONS_LADDER,
  COMPACTION_CHUNK_CHARS_LADDER,
  COMPACTION_KEEP_RECENT_LADDER,
  COMPACTION_LOOP_STEPS_LADDER,
  COMPACTION_MAX_RETRIES_LADDER,
  COMPACTION_STRIP_REASONING_LADDER,
  COMPACTION_SUMMARY_TOKENS_LADDER,
  COMPACTION_THRESHOLD_LADDER,
  COMPACTION_TOKEN_RATIO_LADDER,
  COST_PER_DAY_LADDER,
  COST_PER_HOUR_LADDER,
  COST_PER_MONTH_LADDER,
  CPU_TIME_MS_LADDER,
  HEARTBEAT_INTERVAL_LADDER,
  HEARTBEAT_KEEP_RECENT_LADDER,
  HEARTBEAT_TIMEOUT_LADDER,
  LLM_TOKENS_PER_HOUR_LADDER,
  MAX_CONCURRENT_INVOCATIONS_LADDER,
  MAX_HISTORY_MESSAGES_LADDER,
  MAX_ITERATIONS_LADDER,
  MAX_RESTARTS_LADDER,
  MIN_HISTORY_MESSAGES,
  MEMORY_BYTES_LADDER,
  MIN_SIMILARITY_LADDER,
  NETWORK_BYTES_PER_HOUR_LADDER,
  ROUTING_THRESHOLD_LADDER,
  SKILL_WORKSHOP_MAX_AGE_LADDER,
  SKILL_WORKSHOP_MAX_PENDING_LADDER,
  THINKING_BUDGET_LADDER,
  TOOL_CALLS_PER_MINUTE_LADDER,
  formatBytes,
  formatCount,
  formatHours,
  formatMillis,
  formatPercent,
  formatSeconds,
  formatUsd,
} from "../lib/quantityLadders";
import {
  overLimitWarning,
  resolveMaxTokensLimit,
  selectModelLimits,
} from "../lib/modelLimits";

/**
 * The routing tiers and `pinned_model` hold a bare model name — the daemon
 * resolves it against the global catalog, so `provider/model` would not
 * resolve — while the picker speaks in pairs. Adapting at the call site keeps
 * the picker from having to know that some of its callers discard the
 * provider.
 */
const asModelName = (name: string) => (name ? { provider: "", model: name } : null);

/**
 * Catalog entry for the skill/tool finder (#5049). Both fields are
 * optional so the caller can pass partial data: an unknown skill or
 * tool that the user has typed in still renders as a chip even if the
 * registry doesn't know its description.
 */
export interface ManifestCatalogEntry {
  name: string;
  description?: string;
  /**
   * Built-in tools the entry needs granted, when the catalog carries them.
   *
   * Only the skill catalog supplies this today (`GET /api/skills` →
   * `required_tools`), and it is what lets the skills field name the tools an
   * assigned skill needs and flag the ones the agent's `capabilities.tools`
   * does not admit. Absent means the caller's catalog predates the field or
   * does not model it — not that the entry needs nothing.
   */
  required_tools?: string[];
}

interface AgentManifestFormProps {
  value: ManifestFormState;
  onChange: (next: ManifestFormState) => void;
  providers: { name: string }[];
  /**
   * Model options for the picker. The capacity fields are optional because a
   * caller may only have ids to hand; when they are present they trim the
   * ladders and drive the over-limit advisory.
   *
   * `limits_known === false` marks the capacities as discovery placeholders
   * rather than measurements (#7780). The form ignores them in that case: an
   * unknown limit is not a ceiling, and warning against an invented one trains
   * operators to ignore warnings.
   */
  models: {
    provider: string;
    id: string;
    context_window?: number;
    max_output_tokens?: number;
    limits_known?: boolean;
  }[];
  /**
   * State of the query that produced `models`. Without it a catalog that is
   * still loading, or failed to load, reads the same as an empty one and the
   * pickers claim "No models found" instead of showing the spinner or a retry.
   */
  modelsFetching?: boolean;
  modelsError?: boolean;
  onModelsRetry?: () => void;
  invalidFields: Set<string>;
  // Read-only view of preserved-but-not-form-renderable extras. We show
  // a hint next to dropdowns whose form widget can't represent the
  // contents (e.g. a full `[exec_policy]` table) so the user isn't
  // misled by a default-looking dropdown that hides serialized state.
  extras: ManifestExtras;
  /**
   * Installed-skill catalog from `GET /api/skills`. When present, the
   * "Skills" field renders a fuzzy-find combobox seeded with these
   * names (#5049); when absent (or empty), the field falls back to the
   * plain tag-input so users can still type unknown identifiers.
   */
  skillCatalog?: ManifestCatalogEntry[];
  /**
   * Tool catalog from `GET /api/tools`. Drives the same finder
   * affordance for the "Tool ID Allowlist" capability field.
   */
  toolCatalog?: ManifestCatalogEntry[];
  /**
   * Configured MCP servers catalog from `GET /api/mcp/servers`. When
   * present, the "MCP Servers" field renders a multi-select dropdown
   * seeded with these names (#5246); when absent the field falls back
   * to the plain tag input so callers without a catalog still work and
   * users can reference servers the dashboard doesn't know about yet.
   */
  mcpCatalog?: ManifestCatalogEntry[];
  /**
   * The model router's profile catalog from the server (`GET
   * /api/model-router/profiles`). Present, the profile allowlist renders the
   * same finder the skills and tools lists use; absent, the plain tag box —
   * a caller without the query loses nothing it ever had.
   */
  routerProfileCatalog?: ManifestCatalogEntry[];
  /** Whether the router is enabled kernel-wide; `undefined` is unknown. */
  routerProfilesEnabled?: boolean;
  /**
   * How the "Name" field behaves for this caller (#8028).
   *
   * - `"editable"` (default): the field a caller spawning a brand-new agent
   *   fills in themselves.
   * - `"readonly"`: identity is decided elsewhere (a URL path segment, a
   *   sibling field) and this form only displays it. Rendering it editable
   *   here — as the agent-type editor did — invites an operator to "rename"
   *   an existing type: the request goes through, a success toast appears,
   *   and nothing changes, because the server pins the name to the URL
   *   rather than trusting the body.
   * - `"hidden"`: the caller collects the name through its own field (e.g.
   *   the agent-type create dialog, which has always had exactly one Name
   *   input) and would otherwise end up with two fields that disagree about
   *   which one wins.
   */
  nameField?: "editable" | "readonly" | "hidden";
  /**
   * Why the kernel will not run the tier router this section configures (#8446).
   * `"stable_mode"` shows a warning in the Routing section; absent or `null` means routing is live.
   */
  routingInertReason?: ModelRoutingInertReason | null;
  /**
   * Which sections this caller renders, in the order they appear here.
   *
   * Omitted (the default) renders every section, which is what the
   * create-agent modal wants: one scrolling page with the whole manifest.
   *
   * The agent view passes one config group at a time so the same editor backs
   * every group instead of living in a second drawer behind an "Edit full
   * configuration" button. Splitting the manifest across groups rather than
   * stacking a second surface on top of the first is the point: advanced
   * fields reveal in place, and there is no second place to look.
   *
   * Ids name a *section*, not a field, because a section is the unit a group
   * can host. Where two groups need part of a former section the section was
   * split rather than duplicated — `model` became `prompt` (General) plus
   * `model` (Model), `discovery` became `skills` and `mcp_servers`, and `tags`
   * moved into `identity`.
   *
   * An empty array is a caller error and renders nothing; pass
   * `undefined` to mean "all".
   */
  sections?: ManifestSectionId[];
  /**
   * Whether the folded halves of every section open on render.
   *
   * One switch for the whole form, not one per section: the agent view's
   * config tab carries a single "advanced mode" toggle, and a per-section
   * disclosure that remembered its own state would be a second answer to the
   * question the toggle asks. `invalid` still forces a folded group open in
   * basic mode, because a validation error the operator cannot see is
   * indistinguishable from no error.
   */
  advanced?: boolean;
  /**
   * The agent whose workspace files this form may edit, when there is one.
   *
   * Only the identity section uses it, to offer `IDENTITY.md`'s front-matter
   * as fields. It is optional because the create-agent modal renders this same
   * form before an agent exists, and a workspace file cannot be read or
   * written for an id the server has never seen; the section simply omits the
   * editor there, which is the truth rather than a disabled control.
   *
   * The name will not do: `/api/agents/{id}` resolves an id, not a name, and a
   * name-shaped path segment answers `invalid_agent_id`. This is the same id
   * the detail panel already holds.
   *
   * The editor writes the file, never the manifest, and has its own save. It
   * is not wired into this form's `onChange` on purpose: one Save button that
   * wrote `agent.toml` and a workspace file at once would make the two stores
   * disagree depending on which half failed.
   */
  agentId?: string;
}

/**
 * Every addressable section of the manifest editor, in render order. See
 * `sections` on `AgentManifestFormProps` for why sections are the unit of
 * composition.
 *
 * A runtime array rather than a bare union so callers that need to reason
 * about the whole set — the tab map, and the tests that guard it — can,
 * instead of restating the list and drifting from it.
 */
export const MANIFEST_SECTION_IDS = [
  "identity",
  "metadata",
  "model",
  "prompt",
  "limits",
  "capabilities",
  "tools",
  "skills",
  "mcp_servers",
  "scheduling",
  "fallback_models",
  "thinking",
  "autonomous",
  "proactive_memory",
  "auto_dream",
  "channel_overrides",
  "skill_workshop",
  "compaction",
  "context_engine",
  "async_tasks",
  "routing",
  "context_injection",
  "response_format",
  "lifecycle",
  "exec_policy",
  "shared_folders",
] as const;

export type ManifestSectionId = (typeof MANIFEST_SECTION_IDS)[number];

/**
 * Which section renders the field a validation message names.
 *
 * `validateManifestForm` reports a dotted field path (`schedule.cron`), and
 * with the sections split across tabs a message is only actionable if the
 * operator is already looking at the tab that hosts the field. The caller
 * needs to be able to send them there, which means knowing which section owns
 * each path.
 *
 * The prefixes are matched in order, so the first entry that matches wins and
 * a bare `name` cannot be swallowed by a `model.` rule.
 *
 * A test fails when `validateManifestForm` grows a path no entry covers, so a
 * new rule cannot quietly produce an error nobody can find.
 */
const FIELD_PREFIX_TO_SECTION: ReadonlyArray<readonly [RegExp, ManifestSectionId]> = [
  [/^name$/, "identity"],
  [/^metadata\./, "metadata"],
  [/^tools\./, "tools"],
  [/^exec_policy\./, "exec_policy"],
  [/^context_engine\./, "context_engine"],
  [/^model\./, "model"],
  [/^schedule\./, "scheduling"],
  [/^response_format\./, "response_format"],
  [/^workspaces\./, "shared_folders"],
  [/^autonomous\./, "autonomous"],
  [/^compaction\./, "compaction"],
  [/^skill_workshop\./, "skill_workshop"],
  [/^channel_overrides\./, "channel_overrides"],
  // The two per-agent counts render inside the "Lifecycle" section, not the
  // resource "Limits" one, so they are matched exactly rather than by prefix: a
  // loose prefix would claim any future field that starts the same way, and the
  // routability guard only checks that *some* section claims a path, not that it
  // is the one that renders it.
  [/^max_history_messages$/, "lifecycle"],
  [/^max_concurrent_invocations$/, "lifecycle"],
  // Top-level in the manifest, but it renders inside the auto_dream section,
  // next to its sibling threshold — matched exactly for the same reason the
  // two counts above are.
  [/^auto_dream_min_sessions$/, "auto_dream"],
];

/** The section that renders `path`, or `undefined` when no entry covers it. */
export const sectionForInvalidField = (
  path: string,
): ManifestSectionId | undefined =>
  FIELD_PREFIX_TO_SECTION.find(([pattern]) => pattern.test(path))?.[1];

/** Every field path prefix the validator's errors are expected to start with. */
export const knownInvalidFieldPrefixes = (): string[] =>
  FIELD_PREFIX_TO_SECTION.map(([pattern]) => pattern.source);

export function AgentManifestForm({
  value,
  onChange,
  providers,
  models,
  modelsFetching = false,
  modelsError = false,
  onModelsRetry,
  invalidFields,
  extras,
  skillCatalog,
  toolCatalog,
  mcpCatalog,
  routerProfileCatalog,
  routerProfilesEnabled,
  nameField = "editable",
  routingInertReason,
  agentId,
  sections,
  advanced = false,
}: AgentManifestFormProps) {
  const { t } = useTranslation();

  // `undefined` means "every section" (the create modal). A caller that
  // passes a list gets exactly that list.
  const shows = (id: ManifestSectionId): boolean =>
    sections === undefined || sections.includes(id);

  // The caller's list is an instruction about order as well as membership: a
  // config group lists its sections in the sequence the operator should read
  // them, and the canonical order below is only the default. Unknown ids are
  // dropped — the page-side guard is a test, not a runtime check, and a caller
  // outside the repo should get a missing section rather than a crash — while
  // a repeated id keeps its first position, since it has one DOM node and one
  // React key.
  const orderedSections = useMemo<ManifestSectionId[]>(() => {
    if (sections === undefined) return [...MANIFEST_SECTION_IDS];
    const known = new Set<string>(MANIFEST_SECTION_IDS);
    const seen = new Set<string>();
    const ordered: ManifestSectionId[] = [];
    for (const id of sections) {
      if (!known.has(id) || seen.has(id)) continue;
      seen.add(id);
      ordered.push(id);
    }
    return ordered;
  }, [sections]);

  // The form-level total: every mounted section reports the count it already
  // measures for its own badge (see `useSectionCount`) and withdraws it when it
  // goes away, so the sum covers exactly what is on screen — the active config
  // group in the agent view, the whole manifest in the create modal. The values
  // live in a ref and only the sum is state: a section re-reporting the same
  // number must not schedule a render for it.
  const sectionCounts = useRef(new Map<string, number>());
  const [totalFieldCount, setTotalFieldCount] = useState(0);
  const sectionCountRegistry = useMemo<SectionCountRegistry>(() => {
    const sync = (): void => {
      let total = 0;
      for (const count of sectionCounts.current.values()) total += count;
      setTotalFieldCount((prev) => (prev === total ? prev : total));
    };
    return {
      report: (sectionId, count) => {
        sectionCounts.current.set(sectionId, count);
        sync();
      },
      withdraw: (sectionId) => {
        sectionCounts.current.delete(sectionId);
        sync();
      },
    };
  }, []);

  // The provider the agent already runs on stays selectable even when the
  // caller filtered it out of `providers` (rejected key, local service down).
  const providerOptions = useMemo(() => {
    const current = value.model.provider;
    if (!current || providers.some((p) => p.name === current)) return providers;
    return [...providers, { name: current }];
  }, [providers, value.model.provider]);

  // Curried setters for the nested-state update boilerplate.
  const update = (patch: Partial<ManifestFormState>): void => onChange({ ...value, ...patch });
  const updateModel = (patch: Partial<ManifestFormState["model"]>): void =>
    onChange({ ...value, model: { ...value.model, ...patch } });
  const updateResources = (patch: Partial<ManifestFormState["resources"]>): void =>
    onChange({ ...value, resources: { ...value.resources, ...patch } });
  const updateCapabilities = (patch: Partial<ManifestFormState["capabilities"]>): void =>
    onChange({ ...value, capabilities: { ...value.capabilities, ...patch } });
  const updateThinking = (patch: Partial<ManifestFormState["thinking"]>): void =>
    onChange({ ...value, thinking: { ...value.thinking, ...patch } });
  const updateAutonomous = (patch: Partial<ManifestFormState["autonomous"]>): void =>
    onChange({ ...value, autonomous: { ...value.autonomous, ...patch } });
  const updateChannelOverrides = (
    patch: Partial<ManifestFormState["channel_overrides"]>,
  ): void =>
    onChange({ ...value, channel_overrides: { ...value.channel_overrides, ...patch } });

  const updateExecPolicy = (
    patch: Partial<ManifestFormState["exec_policy"]>,
  ): void => onChange({ ...value, exec_policy: { ...value.exec_policy, ...patch } });

  const updateContextEngine = (
    patch: Partial<ManifestFormState["context_engine"]>,
  ): void =>
    onChange({ ...value, context_engine: { ...value.context_engine, ...patch } });

  // The context engine's two levels, aliased once: the section below reads
  // `ce.hooks.ingest` about thirty times, and `value.context_engine.hooks`
  // every one of those would be noise rather than clarity.
  const ce = value.context_engine;
  const hook = ce.hooks;

  /** The hooks table is two levels down, so it gets a setter of its own. */
  const updateHook = (
    patch: Partial<ManifestFormState["context_engine"]["hooks"]>,
  ): void =>
    onChange({
      ...value,
      context_engine: {
        ...value.context_engine,
        hooks: { ...value.context_engine.hooks, ...patch },
      },
    });

  const updateSkillWorkshop = (
    patch: Partial<ManifestFormState["skill_workshop"]>,
  ): void =>
    onChange({ ...value, skill_workshop: { ...value.skill_workshop, ...patch } });

  const updateCompaction = (
    patch: Partial<ManifestFormState["compaction"]>,
  ): void =>
    onChange({ ...value, compaction: { ...value.compaction, ...patch } });

  const updateProactiveMemory = (
    patch: Partial<ManifestFormState["proactive_memory"]>,
  ): void =>
    onChange({ ...value, proactive_memory: { ...value.proactive_memory, ...patch } });

  const updateRouting = (patch: Partial<ManifestFormState["routing"]>): void =>
    onChange({ ...value, routing: { ...value.routing, ...patch } });

  // The one routing choice, derived from the manifest rather than stored: a
  // manifest that arrives from the API or a template has no engine field, so
  // it is classified by the state the kernel would resolve it to. The setter
  // writes the three fields together (lib/routingEngine.ts holds the table and
  // the reason for each cell).
  const routingEngine = routingEngineOf(value);
  const setRoutingEngine = (engine: RoutingEngine): void =>
    onChange(applyRoutingEngine(value, engine));

  // The main model field holds an id without a provider, so it only offers a
  // list once a provider is chosen; before that the free-text fallback below
  // takes over. `pinned_model` lands on the same provider in Stable mode — the
  // kernel overwrites the model id alone — so its picker shares this list.
  // `models` may be the unfiltered catalog (the routing tiers and fallbacks
  // need it whole), so this narrows it back for both fields.
  const filteredModels = useMemo(
    () => (value.model.provider ? models.filter((m) => m.provider === value.model.provider) : []),
    [models, value.model.provider],
  );

  // The picker wants `{ id }` rather than `{ name }`. Memoised because it is
  // passed to every fallback row, and a fresh array each render would defeat
  // the picker's own memoisation of its provider list.
  // The picker's "keep the current provider reachable" guard can only fire if
  // that provider is in the list it is handed. A fallback whose provider was
  // dropped from discovery — key removed since — must still be re-selectable,
  // so union the fallbacks' providers in alongside the configured ones.
  const providerPickerList = useMemo(() => {
    const names = new Set(providerOptions.map((p) => p.name));
    for (const fb of value.fallback_models ?? []) {
      if (fb.provider) names.add(fb.provider);
    }
    return [...names].map((id) => ({ id }));
  }, [providerOptions, value.fallback_models]);

  // Shared by every catalog-backed picker below: a loading or failed catalog
  // must not render as an empty one.
  const pickerCatalog = {
    isFetching: modelsFetching,
    error: modelsError ? t("chat.unable_to_load_models") : null,
    onRetry: onModelsRetry,
  };

  // Build {options, meta} pairs for the skill/tool finders (#5049).
  // The catalog is union-ed with the user's current selection so
  // entries the registry doesn't know about (e.g. a skill the user
  // typed in by hand, or one that is staged but not yet installed)
  // remain visible as chips and selectable in the dropdown.
  const skillFinder = useMemo(
    () => mergeCatalog(skillCatalog, value.skills),
    [skillCatalog, value.skills],
  );

  const skillNeeds = useMemo(
    () =>
      skillNeedsAgainstGrants(
        value.skills,
        skillCatalog,
        value.capabilities.tools,
        value.tools_disabled,
      ),
    [skillCatalog, value.skills, value.capabilities.tools, value.tools_disabled],
  );

  // The finder's per-option descriptions with each skill's needs appended: the
  // picker is where the choice is made, so the needs belong beside it rather
  // than only in the notice that appears after the fact.
  const skillOptionMeta = useMemo(() => {
    if (!skillFinder) return undefined;
    const meta: Record<string, { description?: string }> = { ...skillFinder.meta };
    for (const entry of skillCatalog ?? []) {
      const needs = entry.required_tools ?? [];
      if (!entry.name || needs.length === 0) continue;
      const needsLine = t("agents.form.skills_needs", { tools: needs.join(", ") });
      const existing = meta[entry.name]?.description;
      meta[entry.name] = {
        description: existing ? `${existing} · ${needsLine}` : needsLine,
      };
    }
    return meta;
  }, [skillFinder, skillCatalog, t]);
  const toolFinder = useMemo(
    () => mergeCatalog(toolCatalog, value.capabilities.tools),
    [toolCatalog, value.capabilities.tools],
  );
  const mcpFinder = useMemo(
    () => mergeCatalog(mcpCatalog, value.mcp_servers),
    [mcpCatalog, value.mcp_servers],
  );
  const routerProfileFinder = useMemo(
    () => mergeCatalog(routerProfileCatalog, value.model.router_allowed_profiles),
    [routerProfileCatalog, value.model.router_allowed_profiles],
  );

  // Limits for the selected model, and only when the catalog vouches for them.
  // Shared with the agent detail drawer, which needs the same three answers and had none of them.
  const selectedModelLimits = useMemo(
    () => selectModelLimits(models, value.model.model, value.model.provider),
    [models, value.model.model, value.model.provider],
  );
  const maxTokensWarning = overLimitWarning(
    value.model.max_tokens,
    resolveMaxTokensLimit(value.model.max_output_tokens, selectedModelLimits.maxOutputTokens),
    t,
  );
  const contextWindowWarning = overLimitWarning(
    value.model.context_window,
    selectedModelLimits.contextWindow,
    t,
  );

  const jsonSchemaFormat =
    value.response_format.mode === "json_schema" ? value.response_format : null;

  return (
    <AdvancedModeContext.Provider value={advanced}>
    <SectionCountsContext.Provider value={sectionCountRegistry}>
    {/* One element, not the header and the list side by side: the create
        modal places the form as a grid cell, and two returned nodes would
        push its preview panel into the next row. */}
    <div>
    {/* One total per view, right-aligned above the sections it sums: the sum
        of every mounted section's badge, in the same pill. The agent view
        mounts one config group at a time, so the total is that group; the
        create modal mounts every section, so it is the whole manifest. Hidden
        at zero, like the badges themselves. */}
    {totalFieldCount > 0 ? (
      <div className="mb-4 flex justify-end" data-testid="manifest-fields-total">
        <FieldCountBadge count={totalFieldCount} labelKey="agents.form.fields_total" />
      </div>
    ) : null}
    {/* The blocks stay written in canonical order — the diff that added the
        caller's order should be the ordering mechanism, not a moved wall of
        JSX — and `ManifestSections` emits them in `orderedSections` order. */}
    <ManifestSections order={orderedSections} className="space-y-4">
      <Section when={shows("identity")} id="identity" title={t("agents.form.basics")}>
        {nameField !== "hidden" && (
          <Field
            label={t("agents.form.name")}
            required
            invalid={invalidFields.has("name")}
            error={invalidFields.has("name") ? t("agents.form.name_required") : undefined}
            errorId="agent-manifest-name-error"
            hint={nameField === "readonly" ? t("agents.form.name_locked_hint") : undefined}
          >
            <input
              type="text"
              value={value.name}
              onChange={(e) => update({ name: e.target.value })}
              placeholder={t("agents.form.name_placeholder")}
              className={`${inputClass} disabled:opacity-50 disabled:cursor-not-allowed`}
              autoFocus={nameField === "editable"}
              disabled={nameField === "readonly"}
              // `Field` wraps in a <div> rather than a <label> (#5246), so the
              // visible label is not associated with the control. Without this
              // the input has no accessible name.
              aria-label={t("agents.form.name")}
              aria-invalid={invalidFields.has("name") || undefined}
              aria-describedby={
                invalidFields.has("name") ? "agent-manifest-name-error" : undefined
              }
            />
          </Field>
        )}
        <Field label={t("agents.form.description")}>
          <input
            type="text"
            value={value.description}
            onChange={(e) => update({ description: e.target.value })}
            placeholder={t("agents.form.description_placeholder")}
            className={inputClass}
          />
        </Field>
        {/* BASIC: name and description. Version, module, priority and tags are
            bookkeeping the manifest carries but an operator rarely sets. */}
        <AdvancedFields>
          <div className="grid grid-cols-2 gap-3">
            <Field label={t("agents.form.version")}>
              <input
                type="text"
                value={value.version}
                onChange={(e) => update({ version: e.target.value })}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.author")}>
              <input
                type="text"
                value={value.author}
                onChange={(e) => update({ author: e.target.value })}
                className={inputClass}
              />
            </Field>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <Field label={t("agents.form.module")}>
              <input
                type="text"
                value={value.module}
                onChange={(e) => update({ module: e.target.value })}
                placeholder={t("agents.form.module_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.priority")}>
              <select
                value={value.priority}
                onChange={(e) => update({ priority: e.target.value as ManifestFormState["priority"] })}
                className={inputClass}
              >
                <option value="Low">{t("agents.form.priority_low")}</option>
                <option value="Normal">{t("agents.form.priority_normal")}</option>
                <option value="High">{t("agents.form.priority_high")}</option>
                <option value="Critical">{t("agents.form.priority_critical")}</option>
              </select>
            </Field>
          </div>
            <Field label={t("agents.form.tags")}>
            <TagInput
              value={value.tags}
              onChange={(next) => update({ tags: next })}
              placeholder={t("agents.form.tags_placeholder")}
            />
          </Field>
        </AdvancedFields>
        {/* The agent's own identity document, when there is an agent. It sits
            outside `AdvancedFields` because it is not an optional detail of the
            manifest — it is the file the prompt reads as "## Identity", and
            the three fields it holds had no surface at all before. */}
        {agentId && <AgentIdentityFileEditor agentId={agentId} />}
      </Section>

      {/* A free-form table, so the editor is the table: one row per key with
          the value's JSON type named by the operator. Inferring the type from
          the text would rewrite `"5"` into `5` on the next save, and both are
          legal values of `HashMap<String, serde_json::Value>`. */}
      <FormSection
        id="metadata"
        shows={shows}
        title={t("agents.form.metadata")}
        defaultOpen={false}
      >
        <p className="text-[10px] text-text-dim/70 mb-2">
          {t("agents.form.metadata_hint")}
        </p>
        {value.metadata.map((row, idx) => (
          <JsonRowEditor
            key={row._uid}
            row={row}
            index={idx}
            labels={{
              key: t("agents.form.metadata_key"),
              type: t("agents.form.metadata_type"),
              value: t("agents.form.metadata_value"),
              remove: t("agents.form.metadata_remove"),
            }}
            onChange={(next) =>
              update({ metadata: patchListItem(value.metadata, idx, next) })
            }
            onRemove={() =>
              update({ metadata: value.metadata.filter((_, i) => i !== idx) })
            }
          />
        ))}
        <PreservedValues
          entries={value.metadata_preserved ?? {}}
          hint={t("agents.form.metadata_preserved_hint")}
        />
        <button
          type="button"
          onClick={() =>
            update({
              metadata: [
                ...value.metadata,
                { _uid: generateUid(), key: "", valueType: "string", value: "" },
              ],
            })
          }
          className="flex items-center gap-1 text-xs text-brand hover:underline"
        >
          <Plus className="w-3.5 h-3.5" />
          {t("agents.form.metadata_add")}
        </button>
      </FormSection>

      <Section when={shows("model")} id="model" title={t("agents.form.model")}>
        {/* No visible label: the card above is titled "Model" and the field
            would repeat the word one line lower. The picker keeps its own
            accessible name through its `label` prop. */}
        <Field
          hint={t("agents.form.inherit_default")}
          invalid={
            invalidFields.has("model.provider") || invalidFields.has("model.model")
          }
        >
          {/* One control for choosing a model, the same one the fallback chain,
              the routing tiers and `pinned_model` already use. A provider
              <select> beside a model <select> answered the same question a
              second way, and it was the pair that could not search: the
              catalog runs to hundreds of ids, and the fallback rows had a
              finder while the primary model — the field an operator sets
              first — did not.

              `allowCustom` is not a nicety. The catalog comes from live
              discovery, and the control this replaces fell back to free text
              whenever discovery returned nothing for a provider. Without the
              escape hatch an operator could not set a model at all in exactly
              the situation that needs one. */}
          <ModelPicker
            label={t("agents.form.model")}
            variant="pair"
            allowCustom
            value={
              value.model.provider || value.model.model
                ? { provider: value.model.provider, model: value.model.model }
                : null
            }
            onChange={(next) =>
              updateModel({ provider: next.provider, model: next.model })
            }
            models={models}
            providers={providerPickerList}
          />
          {/* The way back from a pinned model — the capability the drawer's
              model editor took with it, and the only control that can write
              it: `ModelPicker` commits a `{provider, model}` pair and has no
              empty commit, so before this button an agent pinned to one model
              could only be unpinned by hand-editing `agent.toml`.

              `"default"` is the sentinel the daemon resolves for both the
              provider and the model (`kernel/llm_drivers.rs:178` — "Resolve
              'default' or empty provider to the effective default provider"),
              and `ModelConfig::default()` spells the pair exactly this way,
              so this writes the canonical form rather than a private one. */}
          <div className="mt-1.5 flex justify-end">
            <button
              type="button"
              onClick={() => updateModel({ provider: "default", model: "default" })}
              className="inline-flex items-center gap-1 text-[10px] font-semibold text-text-dim hover:text-brand transition-colors"
            >
              <RotateCcw className="h-3 w-3" />
              {t("agents.use_global_default", { defaultValue: "Use global default" })}
            </button>
          </div>
        </Field>
        {/*
          BASIC: which model runs. Everything else in this section — sampling
          preferences, endpoint limits, credentials, the router's per-agent
          settings — is depth the Advanced disclosure unfolds in place.
        */}
        <AdvancedFields
          invalid={[
            "model.temperature",
            "model.top_p",
            "model.frequency_penalty",
            "model.presence_penalty",
            "model.max_tokens",
            "model.context_window",
            "model.max_output_tokens",
          ].some((f) => invalidFields.has(f))}
        >
        {/*
          Sampling preferences. Each is tri-state and empty means inherit —
          this agent has no opinion, so the per-model override supplies the
          value. An agent that does state a preference wins over that override,
          which is what lets two instances of one agent type run the same model
          at different temperatures.
        */}
        <p className="text-[11px] text-text-dim">{t("agents.form.preferences_hint")}</p>
        {/*
          The same rung ladder the token fields use. These were four bare
          number boxes with a `step` attribute, so setting a temperature meant
          knowing that 0.7 is the usual default and 2 is the ceiling — the
          control stated neither, while the model settings drawer rendered the
          identical parameter as a slider. One parameter, one control.
        */}
        <div className="grid grid-cols-2 gap-3">
          <ModelParamField
            param="temperature"
            value={value.model.temperature}
            onChange={(next) => updateModel({ temperature: next })}
            invalid={invalidFields.has("model.temperature")}
          />
          <ModelParamField
            param="top_p"
            value={value.model.top_p}
            onChange={(next) => updateModel({ top_p: next })}
            invalid={invalidFields.has("model.top_p")}
          />
          <ModelParamField
            param="frequency_penalty"
            value={value.model.frequency_penalty}
            onChange={(next) => updateModel({ frequency_penalty: next })}
            invalid={invalidFields.has("model.frequency_penalty")}
          />
          <ModelParamField
            param="presence_penalty"
            value={value.model.presence_penalty}
            onChange={(next) => updateModel({ presence_penalty: next })}
            invalid={invalidFields.has("model.presence_penalty")}
          />
        </div>
        <p className="text-[11px] text-text-dim">{t("model_param.local_samplers_hint")}</p>
        <div className="grid grid-cols-2 gap-3">
          <ModelParamField
            param="top_k"
            value={value.model.top_k}
            onChange={(next) => updateModel({ top_k: next })}
            invalid={invalidFields.has("model.top_k")}
          />
          <ModelParamField
            param="min_p"
            value={value.model.min_p}
            onChange={(next) => updateModel({ min_p: next })}
            invalid={invalidFields.has("model.min_p")}
          />
          <ModelParamField
            param="repeat_penalty"
            value={value.model.repeat_penalty}
            onChange={(next) => updateModel({ repeat_penalty: next })}
            invalid={invalidFields.has("model.repeat_penalty")}
          />
        </div>
        <ModelParamField
          param="max_tokens"
          value={value.model.max_tokens}
          onChange={(next) => updateModel({ max_tokens: next })}
          cap={selectedModelLimits.maxOutputTokens}
          warning={maxTokensWarning}
          invalid={invalidFields.has("model.max_tokens")}
        />
        {/*
          Endpoint limits, not preferences. These describe what the model can
          accept; over-limit values are reported rather than clamped, so the
          operator sees the conflict instead of a number they never chose.
        */}
        <p className="text-[11px] text-text-dim">{t("agents.form.limits_hint")}</p>
        <ModelParamField
          param="context_window"
          value={value.model.context_window}
          onChange={(next) => updateModel({ context_window: next })}
          warning={contextWindowWarning}
          invalid={invalidFields.has("model.context_window")}
          error={t("agents.form.whole_number_required")}
        />
        <ModelParamField
          param="max_output_tokens"
          value={value.model.max_output_tokens}
          onChange={(next) => updateModel({ max_output_tokens: next })}
          invalid={invalidFields.has("model.max_output_tokens")}
          error={t("agents.form.whole_number_required")}
        />
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.api_key_env")} hint={t("agents.form.api_key_env_hint")}>
            <input
              type="text"
              value={value.model.api_key_env}
              onChange={(e) => updateModel({ api_key_env: e.target.value })}
              placeholder={t("agents.form.api_key_env_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.base_url")}>
            <input
              type="text"
              value={value.model.base_url}
              onChange={(e) => updateModel({ base_url: e.target.value })}
              placeholder={t("agents.form.base_url_placeholder")}
              className={inputClass}
            />
          </Field>
        </div>
        {/*
          The router override's per-agent settings. They are manifest fields,
          and this form is their only editor: the routing panel that shared
          them died here — two writers to the same five values, and a form
          save serialized its seeded state verbatim over anything the panel
          had written, both surfaces toasting success. The profile allowlist
          keeps the panel's server-backed catalog.

          Rendered under every engine, because they are live under every
          engine. `allowed_profiles` and `cost_budget` are not routing
          preferences: the spawn gate checks them against every profile an
          `agent_spawn` names, whatever `mode` says —
          `check_profile_against_parent`
          (crates/librefang-runtime/src/tool_runner/agent.rs), over the
          override the kernel hands it through `model_router_override_for`
          (crates/librefang-kernel-handle/src/catalog_query.rs). Hiding them
          behind the profile engine would put a live cap out of reach —
          editable only by switching engines, editing, and switching back.
        */}
        <div className="space-y-2">
          <p className="text-[11px] text-text-dim">{t("agents.form.router_hint")}</p>
          {/* Deliberately a statement and not a control. The engine table
              never writes the pin — writing it would revoke a capability — so
              the only way an operator meets one is by arriving with it on
              disk, and the alternative to stating it is meeting it as a
              refused `agent_spawn`. Offering to set it would reopen the state
              range the table closed; clearing it is already reachable, by
              choosing the profile engine. */}
          {value.model.router_fixed && (
            <p className="text-[11px] text-warning">{t("agents.form.router_pinned_note")}</p>
          )}
          <div className="grid grid-cols-2 gap-3">
            <Field
              label={t("agents.form.router_cost_budget")}
              hint={t("agents.form.router_cost_budget_hint")}
            >
              {/* Empty is the daemon's "no cap": the router may pick any tier. */}
              <select
                value={value.model.router_cost_budget}
                onChange={(e) =>
                  updateModel({
                    router_cost_budget: e.target.value as
                      ManifestFormState["model"]["router_cost_budget"],
                  })
                }
                className={inputClass}
              >
                <option value="">{t("agents.form.router_cost_budget_none")}</option>
                <option value="cheap">cheap</option>
                <option value="medium">medium</option>
                <option value="expensive">expensive</option>
              </select>
            </Field>
            <Field
              label={t("agents.form.router_default_profile")}
              hint={t("agents.form.router_default_profile_hint")}
            >
              <input
                type="text"
                value={value.model.router_default_profile}
                onChange={(e) => updateModel({ router_default_profile: e.target.value })}
                placeholder={t("agents.form.router_default_profile_placeholder")}
                className={inputClass}
              />
            </Field>
          </div>
          <Field
            label={t("agents.form.router_allowed_profiles")}
            hint={t("agents.form.router_allowed_profiles_hint")}
          >
            {routerProfileFinder ? (
              <MultiSelectCmdk
                options={routerProfileFinder.options}
                optionMeta={routerProfileFinder.meta}
                value={value.model.router_allowed_profiles}
                onChange={(next) => {
                  const nextValue =
                    typeof next === "function"
                      ? next(value.model.router_allowed_profiles)
                      : next;
                  updateModel({ router_allowed_profiles: nextValue });
                }}
                placeholder={t("agents.form.router_profiles_search_placeholder", {
                  defaultValue: "Search model profiles…",
                })}
                allowFreeText
              />
            ) : (
              <TagInput
                value={value.model.router_allowed_profiles}
                onChange={(next) => updateModel({ router_allowed_profiles: next })}
                placeholder={t("agents.form.router_allowed_profiles_placeholder")}
              />
            )}
          </Field>
          {routerProfilesEnabled === false && (
            <p className="text-[11px] text-text-dim">
              {t("agents.form.router_kernel_off")}
            </p>
          )}
        </div>
        </AdvancedFields>
      </Section>

      <Section when={shows("prompt")} id="prompt" title={t("agents.form.system_prompt")}>
        {/* Labelled on the control, not above it: the card says "System
            Prompt" and a second copy one line down reads as a stutter. */}
        <Field>
          <textarea
            aria-label={t("agents.form.system_prompt")}
            value={value.model.system_prompt}
            onChange={(e) => updateModel({ system_prompt: e.target.value })}
            placeholder={t("agents.form.system_prompt_placeholder")}
            rows={3}
            className={textareaClass}
          />
        </Field>
      </Section>

      <Section when={shows("limits")} id="limits" title={t("agents.form.resources")}>
        {/* BASIC: the two quotas an operator sets first — the hourly token
            budget and the daily cost ceiling. The rest of the resource table
            folds behind Advanced. */}
        <div className="grid grid-cols-2 gap-3">
          <StepLadderInput
            label={t("agents.form.tokens_per_hour")}
            value={value.resources.max_llm_tokens_per_hour}
            onChange={(next) => updateResources({ max_llm_tokens_per_hour: next })}
            ladder={LLM_TOKENS_PER_HOUR_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.inherit_default")}
            min={0}
          />
          <StepLadderInput
            label={t("agents.form.cost_per_day")}
            value={value.resources.max_cost_per_day_usd}
            onChange={(next) => updateResources({ max_cost_per_day_usd: next })}
            ladder={COST_PER_DAY_LADDER}
            formatRung={formatUsd}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.unlimited_placeholder")}
            min={0}
            // Dollars, so the custom box must accept cents: an unset step
            // defaults to 1 and the browser marks a value like 0.50 invalid.
            step={0.01}
          />
        </div>
        <AdvancedFields>
          <div className="grid grid-cols-2 gap-3">
          <StepLadderInput
            label={t("agents.form.tool_calls_per_minute")}
            value={value.resources.max_tool_calls_per_minute}
            onChange={(next) => updateResources({ max_tool_calls_per_minute: next })}
            ladder={TOOL_CALLS_PER_MINUTE_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.tool_calls_per_minute_placeholder")}
            min={0}
          />
          <StepLadderInput
            label={t("agents.form.cost_per_hour")}
            value={value.resources.max_cost_per_hour_usd}
            onChange={(next) => updateResources({ max_cost_per_hour_usd: next })}
            ladder={COST_PER_HOUR_LADDER}
            formatRung={formatUsd}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.unlimited_placeholder")}
            min={0}
            // Dollars, so the custom box must accept cents: an unset step
            // defaults to 1 and the browser marks a value like 0.50 invalid.
            step={0.01}
          />
          <StepLadderInput
            label={t("agents.form.cost_per_month")}
            value={value.resources.max_cost_per_month_usd}
            onChange={(next) => updateResources({ max_cost_per_month_usd: next })}
            ladder={COST_PER_MONTH_LADDER}
            formatRung={formatUsd}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.unlimited_placeholder")}
            min={0}
            // Dollars, so the custom box must accept cents: an unset step
            // defaults to 1 and the browser marks a value like 0.50 invalid.
            step={0.01}
          />
          <StepLadderInput
            label={t("agents.form.network_bytes_per_hour")}
            value={value.resources.max_network_bytes_per_hour}
            onChange={(next) => updateResources({ max_network_bytes_per_hour: next })}
            ladder={NETWORK_BYTES_PER_HOUR_LADDER}
            formatRung={formatBytes}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.network_bytes_placeholder")}
            min={0}
          />
          <StepLadderInput
            label={t("agents.form.memory_bytes")}
            value={value.resources.max_memory_bytes}
            onChange={(next) => updateResources({ max_memory_bytes: next })}
            ladder={MEMORY_BYTES_LADDER}
            formatRung={formatBytes}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.memory_bytes_placeholder")}
            min={0}
          />
          <StepLadderInput
            label={t("agents.form.cpu_time_ms")}
            value={value.resources.max_cpu_time_ms}
            onChange={(next) => updateResources({ max_cpu_time_ms: next })}
            ladder={CPU_TIME_MS_LADDER}
            formatRung={formatMillis}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            customPlaceholder={t("agents.form.cpu_time_placeholder")}
            min={0}
          />
          </div>
        <StepLadderInput
          label={t("agents.form.burst_ratio")}
          value={value.resources.burst_ratio}
          onChange={(next) => updateResources({ burst_ratio: next })}
          ladder={BURST_RATIO_LADDER}
          formatRung={formatPercent}
          inheritLabel={t("model_param.inherit")}
          customLabel={t("model_param.custom")}
          customPlaceholder={t("agents.form.burst_ratio_placeholder")}
          min={0}
          max={1}
          step={0.01}
        />
          <p className="text-[10px] text-text-dim/70 mt-1">
            {t("agents.form.burst_ratio_hint")}
          </p>
        </AdvancedFields>
      </Section>

      <Section when={shows("capabilities")} id="capabilities" title={t("agents.form.capabilities")}>
        {/* The gate is an intersection, and this section edits one layer of
            it. Stated first, before any field, because an operator who reads
            only the controls below will believe a grant here is the decision —
            and the layer that most often subtracts from it (the per-user
            policy) is configured somewhere this editor cannot show. */}
        <p className="text-[11px] text-text-dim">
          {t("agents.form.capabilities_layers_note")}
        </p>
        <Field label={t("agents.form.network_hosts")} hint={t("agents.form.network_hosts_hint")}>
          <TagInput
            value={value.capabilities.network}
            onChange={(next) => updateCapabilities({ network: next })}
            placeholder={t("agents.form.network_hosts_placeholder")}
          />
        </Field>
        <Field label={t("agents.form.shell_commands")} hint={t("agents.form.shell_commands_hint")}>
          <TagInput
            value={value.capabilities.shell}
            onChange={(next) => updateCapabilities({ shell: next })}
            placeholder={t("agents.form.shell_commands_placeholder")}
          />
        </Field>
        {/* BASIC: what the agent may reach on the network and run in a shell.
            The per-tool grant, the memory glob lists and the media-routing
            table are depth. */}
        <AdvancedFields>
        <Field label={t("agents.form.cap_tools")} hint={t("agents.form.cap_tools_hint")}>
          {toolFinder ? (
            <MultiSelectCmdk
              options={toolFinder.options}
              optionMeta={toolFinder.meta}
              value={value.capabilities.tools}
              onChange={(next) => {
                const nextValue =
                  typeof next === "function" ? next(value.capabilities.tools) : next;
                updateCapabilities({ tools: nextValue });
              }}
              placeholder={t("agents.form.cap_tools_search_placeholder", {
                defaultValue: "Search tools…",
              })}
              allowFreeText
            />
          ) : (
            <TagInput
              value={value.capabilities.tools}
              onChange={(next) => updateCapabilities({ tools: next })}
              placeholder={t("agents.form.cap_tools_placeholder")}
            />
          )}
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.memory_read")}>
            {/*
              `?? []` renders the undeclared state as an empty box. Editing it
              turns it into a declared list, which is what the operator means by
              typing here; leaving it alone keeps `null` and the key stays off
              disk.
            */}
            <TagInput
              value={value.capabilities.memory_read ?? []}
              onChange={(next) => updateCapabilities({ memory_read: next })}
              placeholder={t("agents.form.memory_glob_placeholder")}
            />
            <MemoryScopeNote
              value={value.capabilities.memory_read}
              onSet={(next) => updateCapabilities({ memory_read: next })}
            />
          </Field>
          <Field label={t("agents.form.memory_write")}>
            <TagInput
              value={value.capabilities.memory_write ?? []}
              onChange={(next) => updateCapabilities({ memory_write: next })}
              placeholder={t("agents.form.memory_glob_placeholder")}
            />
            <MemoryScopeNote
              value={value.capabilities.memory_write}
              onSet={(next) => updateCapabilities({ memory_write: next })}
            />
          </Field>
          <Field label={t("agents.form.agent_message")}>
            <TagInput
              value={value.capabilities.agent_message}
              onChange={(next) => updateCapabilities({ agent_message: next })}
              placeholder={t("agents.form.agent_message_placeholder")}
            />
          </Field>
          <Field label={t("agents.form.ofp_connect")}>
            <TagInput
              value={value.capabilities.ofp_connect}
              onChange={(next) => updateCapabilities({ ofp_connect: next })}
              placeholder={t("agents.form.ofp_connect_placeholder")}
            />
          </Field>
        </div>
        <div className="flex flex-wrap gap-4 pt-1">
          <Toggle
            label={t("agents.form.agent_spawn")}
            checked={value.capabilities.agent_spawn}
            onChange={(checked) => updateCapabilities({ agent_spawn: checked })}
          />
          <Toggle
            label={t("agents.form.ofp_discover")}
            checked={value.capabilities.ofp_discover}
            onChange={(checked) => updateCapabilities({ ofp_discover: checked })}
          />
        </div>

        {/*
          Media capability routing. Each field is one modality this agent's own
          model may not handle; leaving it blank inherits the kernel-global
          `[capabilities]` block, which is why the placeholder is the word
          "inherit" rather than an example value — a blank field here is a real
          setting, not an unfilled one.
        */}
        <div className="space-y-2.5 border-t border-border-subtle/60 pt-2.5">
          <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
            {t("agents.form.capability_routing")}
          </p>
          <p className="text-[11px] leading-snug text-text-dim">
            {t("agents.form.capability_routing_hint")}
          </p>
          <div className="grid grid-cols-2 gap-3">
            {CAPABILITY_ROUTING_KEYS.map((key) => (
              <Field
                key={key}
                label={t(`agents.form.capability_${key}`)}
                hint={t(`agents.form.capability_${key}_hint`)}
              >
                <input
                  type="text"
                  value={value.capabilities[key]}
                  onChange={(e) => updateCapabilities({ [key]: e.target.value })}
                  placeholder={t("agents.form.capability_inherit_placeholder")}
                  className={inputClass}
                />
              </Field>
            ))}
          </div>
        </div>
        <div className="grid grid-cols-2 gap-3 mt-2">
          <Field label={t("agents.form.tool_exec_backend")} hint={t("agents.form.inherit_default")}>
            <select
              value={value.tool_exec_backend}
              onChange={(e) =>
                update({
                  tool_exec_backend: e.target.value as ManifestFormState["tool_exec_backend"],
                })
              }
              className={inputClass}
            >
              <option value="">{t("agents.form.inherit_default")}</option>
              <option value="local">local</option>
              <option value="docker">docker</option>
              <option value="ssh">ssh</option>
              <option value="daytona">daytona</option>
            </select>
          </Field>
        </div>
        </AdvancedFields>
      </Section>

      {/* Not `capabilities.tools`, which lists the names an agent may call.
          This is `[tools.<name>]` — the per-tool parameter overrides for a
          tool that is already available. */}
      <FormSection
        id="tools"
        shows={shows}
        title={t("agents.form.tool_overrides")}
        defaultOpen={false}
      >
        <p className="text-[10px] text-text-dim/70 mb-2">
          {t("agents.form.tool_overrides_hint")}
        </p>
        {value.tools.map((entry, idx) => (
          <div
            key={entry._uid}
            className="mb-2 space-y-2 rounded-lg border border-border-subtle/60 bg-main/40 p-2"
          >
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={entry.name}
                onChange={(e) =>
                  update({
                    tools: patchListItem(value.tools, idx, {
                      ...entry,
                      name: e.target.value,
                    }),
                  })
                }
                placeholder={t("agents.form.tool_name")}
                aria-label={t("agents.form.tool_name")}
                className={inputClass}
              />
              <button
                type="button"
                onClick={() =>
                  update({ tools: value.tools.filter((_, i) => i !== idx) })
                }
                className="shrink-0 text-text-dim hover:text-error"
                aria-label={t("agents.form.tool_remove")}
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            {entry.params.map((row, paramIdx) => (
              <JsonRowEditor
                key={row._uid}
                row={row}
                index={paramIdx}
                labels={{
                  key: t("agents.form.tool_param_key"),
                  type: t("agents.form.tool_param_type"),
                  value: t("agents.form.tool_param_value"),
                  remove: t("agents.form.tool_param_remove"),
                }}
                onChange={(next) =>
                  update({
                    tools: patchListItem(value.tools, idx, {
                      ...entry,
                      params: patchListItem(entry.params, paramIdx, next),
                    }),
                  })
                }
                onRemove={() =>
                  update({
                    tools: patchListItem(value.tools, idx, {
                      ...entry,
                      params: entry.params.filter((_, i) => i !== paramIdx),
                    }),
                  })
                }
              />
            ))}
            <PreservedValues
              entries={entry.params_preserved ?? {}}
              hint={t("agents.form.tool_params_preserved_hint")}
            />
            <PreservedValues
              entries={entry.preserved ?? {}}
              hint={t("agents.form.tool_preserved_hint")}
            />
            <button
              type="button"
              onClick={() =>
                update({
                  tools: patchListItem(value.tools, idx, {
                    ...entry,
                    params: [
                      ...entry.params,
                      { _uid: generateUid(), key: "", valueType: "string", value: "" },
                    ],
                  }),
                })
              }
              className="flex items-center gap-1 text-xs text-brand hover:underline"
            >
              <Plus className="w-3.5 h-3.5" />
              {t("agents.form.tool_param_add")}
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={() =>
            update({
              tools: [
                ...value.tools,
                { _uid: generateUid(), name: "", params: [] },
              ],
            })
          }
          className="flex items-center gap-1 text-xs text-brand hover:underline"
        >
          <Plus className="w-3.5 h-3.5" />
          {t("agents.form.tool_add")}
        </button>
      </FormSection>

      <Section when={shows("skills")} id="skills" title={t("agents.form.skills")}>
        <Field hint={t("agents.form.skills_hint")} ariaLabel={t("agents.form.skills")}>
          {skillFinder ? (
            <MultiSelectCmdk
              options={skillFinder.options}
              optionMeta={skillOptionMeta}
              value={value.skills}
              onChange={(next) => {
                const nextValue =
                  typeof next === "function" ? next(value.skills) : next;
                update({ skills: nextValue });
              }}
              placeholder={t("agents.form.skills_search_placeholder", {
                defaultValue: "Search installed skills…",
              })}
              allowFreeText
            />
          ) : (
            <TagInput
              value={value.skills}
              onChange={(next) => update({ skills: next })}
              placeholder={t("agents.form.skills_placeholder")}
            />
          )}
        </Field>
        {/* What each assigned skill asks for, and the ones this agent cannot
            give it. Rendered for every skill that declares needs, not only the
            broken ones: the line is why the warning below it can be believed,
            and a notice that appeared from nowhere would read as noise. */}
        {skillNeeds.length > 0 && (
          <div className="mt-1 space-y-0.5">
            {skillNeeds.map(({ name, needs, missing }) => (
              <p key={name} className="text-[11px] text-text-dim">
                <span className="font-mono">{name}</span>
                {" — "}
                <span>{t("agents.form.skills_needs", { tools: needs.join(", ") })}</span>
                {missing.length > 0 && (
                  <span className="text-warning">
                    {" "}
                    {t("agents.form.skills_missing_tools", {
                      tools: missing.join(", "),
                    })}
                  </span>
                )}
              </p>
            ))}
          </div>
        )}
      </Section>

      <Section when={shows("mcp_servers")} id="mcp_servers" title={t("agents.form.mcp_servers")}>
        <Field hint={t("agents.form.mcp_servers_hint")} ariaLabel={t("agents.form.mcp_servers")}>
          {mcpFinder ? (
            <MultiSelectCmdk
              options={mcpFinder.options}
              optionMeta={mcpFinder.meta}
              value={value.mcp_servers}
              onChange={(next) => {
                const nextValue =
                  typeof next === "function" ? next(value.mcp_servers) : next;
                update({ mcp_servers: nextValue });
              }}
              placeholder={t("agents.form.mcp_servers_search_placeholder", {
                defaultValue: "Search MCP servers…",
              })}
              allowFreeText
            />
          ) : (
            <TagInput
              value={value.mcp_servers}
              onChange={(next) => update({ mcp_servers: next })}
              placeholder={t("agents.form.mcp_servers_placeholder")}
            />
          )}
        </Field>
        <div className="mt-2">
          <Toggle
            label={t("agents.form.mcp_disabled")}
            checked={value.mcp_disabled}
            onChange={(checked) => update({ mcp_disabled: checked })}
          />
        </div>
      </Section>

      <FormSection id="scheduling" shows={shows}
        title={t("agents.form.scheduling")}
        defaultOpen={false}
        invalid={
          invalidFields.has("schedule.cron") ||
          invalidFields.has("schedule.check_interval_secs")
        }
      >
        <Field label={t("agents.form.schedule_mode")} hint={t("agents.form.schedule_mode_hint")}>
          <select
            value={value.schedule.mode}
            onChange={(e) => {
              const mode = e.target.value as ManifestFormState["schedule"]["mode"];
              if (mode === "reactive") update({ schedule: { mode } });
              else if (mode === "periodic") update({ schedule: { mode, cron: "" } });
              else if (mode === "proactive") update({ schedule: { mode, conditions: [] } });
              else update({ schedule: { mode, check_interval_secs: "300" } });
            }}
            className={inputClass}
          >
            <option value="reactive">{t("agents.form.schedule_reactive")}</option>
            <option value="periodic">{t("agents.form.schedule_periodic")}</option>
            <option value="proactive">{t("agents.form.schedule_proactive")}</option>
            <option value="continuous">{t("agents.form.schedule_continuous")}</option>
          </select>
        </Field>
        {value.schedule.mode === "periodic" && (
          <Field
            label={t("agents.form.cron")}
            hint={t("agents.form.cron_hint")}
            required
            invalid={invalidFields.has("schedule.cron")}
            error={t("agents.form.cron_required_error")}
            errorId="agent-manifest-schedule-cron-error"
          >
            <input
              id="agent-manifest-schedule-cron"
              type="text"
              value={value.schedule.cron}
              onChange={(e) => update({ schedule: { mode: "periodic", cron: e.target.value } })}
              placeholder={t("agents.form.cron_placeholder")}
              className={inputClass}
              aria-label={t("agents.form.cron")}
              aria-invalid={invalidFields.has("schedule.cron") || undefined}
              aria-required="true"
              aria-describedby={
                invalidFields.has("schedule.cron")
                  ? "agent-manifest-schedule-cron-error"
                  : undefined
              }
            />
          </Field>
        )}
        {value.schedule.mode === "proactive" && (
          <Field label={t("agents.form.conditions")}>
            <TagInput
              value={value.schedule.conditions}
              onChange={(next) => update({ schedule: { mode: "proactive", conditions: next } })}
              placeholder={t("agents.form.conditions_placeholder")}
            />
          </Field>
        )}
        {value.schedule.mode === "continuous" && (
          <Field
            label={t("agents.form.check_interval_secs")}
            required
            invalid={invalidFields.has("schedule.check_interval_secs")}
            error={t("agents.detail.schedule_invalid_interval")}
            errorId="agent-manifest-schedule-interval-error"
          >
            <input
              id="agent-manifest-schedule-interval"
              type="number"
              min="1"
              value={value.schedule.check_interval_secs}
              onChange={(e) =>
                update({
                  schedule: { mode: "continuous", check_interval_secs: e.target.value },
                })
              }
              placeholder={t("agents.form.check_interval_placeholder")}
              className={inputClass}
              aria-label={t("agents.form.check_interval_secs")}
              aria-invalid={
                invalidFields.has("schedule.check_interval_secs") || undefined
              }
              aria-required="true"
              aria-describedby={
                invalidFields.has("schedule.check_interval_secs")
                  ? "agent-manifest-schedule-interval-error"
                  : undefined
              }
            />
          </Field>
        )}
      </FormSection>

      <FormSection id="fallback_models" shows={shows} title={t("agents.form.fallback_models")} defaultOpen={false}>
        <p className="text-[10px] text-text-dim/70 mb-2">{t("agents.form.fallback_models_hint")}</p>
        {(value.fallback_models ?? []).map((fb, idx) => (
          <div
            key={fb._uid}
            className="rounded-lg border border-border-subtle/60 bg-main/40 p-2 mb-2 space-y-2"
          >
            <div className="flex items-center justify-between">
              <span className="text-[10px] font-bold text-text-dim uppercase">#{idx + 1}</span>
              <button
                type="button"
                onClick={() => update({ fallback_models: (value.fallback_models ?? []).filter((_, i) => i !== idx) })}
                className="text-text-dim hover:text-error"
                aria-label={t("agents.form.remove_fallback")}
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            <div className="grid grid-cols-2 gap-2">
              {/* Provider and model are one decision here — which model this
                  fallback stands for — so they are one control rather than two
                  boxes to type an id into. `allowCustom` stays on because a
                  fallback exists precisely for the case where the preferred
                  provider is not reachable. */}
              <div className="col-span-2">
                <ModelPicker
                  label={`${t("agents.form.model_id")} ${idx + 1}`}
                  variant="pair"
                  allowCustom
                  value={fb.model ? { provider: fb.provider, model: fb.model } : null}
                  onChange={(next) =>
                    update({
                      fallback_models: patchListItem(value.fallback_models ?? [], idx, {
                        ...fb,
                        provider: next.provider,
                        model: next.model,
                      }),
                    })
                  }
                  models={models}
                  providers={providerPickerList}
                  {...pickerCatalog}
                />
              </div>
              <input
                type="text"
                value={fb.api_key_env}
                onChange={(e) => update({ fallback_models: patchListItem(value.fallback_models ?? [], idx, { ...fb, api_key_env: e.target.value }) })}
                placeholder={t("agents.form.api_key_env")}
                className={inputClass}
              />
              <input
                type="text"
                value={fb.base_url}
                onChange={(e) => update({ fallback_models: patchListItem(value.fallback_models ?? [], idx, { ...fb, base_url: e.target.value }) })}
                placeholder={t("agents.form.base_url")}
                className={inputClass}
              />
            </div>
          </div>
        ))}
        <button
          type="button"
          onClick={() =>
            update({
              fallback_models: [
                ...value.fallback_models ?? [],
                { _uid: generateUid(), provider: "", model: "", api_key_env: "", base_url: "", extras: {} },
              ],
            })
          }
          className="flex items-center gap-1 text-xs text-brand hover:underline"
        >
          <Plus className="w-3.5 h-3.5" />
          {t("agents.form.add_fallback")}
        </button>
        {(value.fallback_models ?? []).length === 0 &&
          (value.fallback_models === null ? (
            <p className="mt-1 text-xs text-text-dim">
              {t("agents.form.fallback_scope_inherited")}{" "}
              <button
                type="button"
                className="underline hover:text-brand"
                onClick={() => update({ fallback_models: [] })}
              >
                {t("agents.form.fallback_scope_disable")}
              </button>
            </p>
          ) : (
            <p className="mt-1 text-xs text-warning">
              {t("agents.form.fallback_scope_disabled")}{" "}
              <button
                type="button"
                className="underline hover:text-brand"
                onClick={() => update({ fallback_models: null })}
              >
                {t("agents.form.fallback_scope_inherit")}
              </button>
            </p>
          ))}
      </FormSection>

      <FormSection id="thinking" shows={shows} title={t("agents.form.thinking")} defaultOpen={false}>
        <Toggle
          label={t("agents.form.thinking_enabled")}
          checked={value.thinking.enabled}
          onChange={(checked) => updateThinking({ enabled: checked })}
        />
        {value.thinking.enabled && (
          <div className="grid grid-cols-2 gap-3 mt-2">
            <StepLadderInput
              label={t("agents.form.budget_tokens")}
              value={value.thinking.budget_tokens}
              onChange={(next) => updateThinking({ budget_tokens: next })}
              ladder={THINKING_BUDGET_LADDER}
              formatRung={formatCount}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              customPlaceholder={t("agents.form.budget_tokens_placeholder")}
              min={0}
            />

            <Field label={t("agents.form.stream_thinking")}>
              <Toggle
                label=""
                ariaLabel={t("agents.form.stream_thinking")}
                checked={value.thinking.stream_thinking}
                onChange={(checked) => updateThinking({ stream_thinking: checked })}
              />
            </Field>
          </div>
        )}
      </FormSection>

      <FormSection id="autonomous" shows={shows}
        title={t("agents.form.autonomous")}
        defaultOpen={false}
        invalid={invalidFields.has("autonomous.heartbeat_timeout_secs")}
      >
        <Toggle
          label={t("agents.form.autonomous_enabled")}
          checked={value.autonomous.enabled}
          onChange={(checked) => updateAutonomous({ enabled: checked })}
        />
        {value.autonomous.enabled && (
          <>
            {/* BASIC: how long an autonomous run may go on. The restart cap,
                the heartbeats and quiet hours are depth. */}
            <div className="grid grid-cols-2 gap-3 mt-2">
              <StepLadderInput
                label={t("agents.form.max_iterations")}
                value={value.autonomous.max_iterations}
                onChange={(next) => updateAutonomous({ max_iterations: next })}
                ladder={MAX_ITERATIONS_LADDER}
                formatRung={formatCount}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                customPlaceholder={t("agents.form.max_iterations_placeholder")}
                min={1}
              />
            </div>
            <AdvancedFields
          invalid={invalidFields.has("autonomous.heartbeat_timeout_secs")}
        >
              <div className="grid grid-cols-2 gap-3 mt-2">
            <StepLadderInput
              label={t("agents.form.max_restarts")}
              value={value.autonomous.max_restarts}
              onChange={(next) => updateAutonomous({ max_restarts: next })}
              ladder={MAX_RESTARTS_LADDER}
              formatRung={formatCount}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              customPlaceholder={t("agents.form.max_restarts_placeholder")}
              min={0}
            />

            <StepLadderInput
              label={t("agents.form.heartbeat_interval_secs")}
              value={value.autonomous.heartbeat_interval_secs}
              onChange={(next) => updateAutonomous({ heartbeat_interval_secs: next })}
              ladder={HEARTBEAT_INTERVAL_LADDER}
              formatRung={formatSeconds}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              customPlaceholder={t("agents.form.heartbeat_interval_placeholder")}
              min={1}
            />

            <StepLadderInput
              label={t("agents.form.heartbeat_timeout_secs")}
              value={value.autonomous.heartbeat_timeout_secs}
              onChange={(next) => updateAutonomous({ heartbeat_timeout_secs: next })}
              ladder={HEARTBEAT_TIMEOUT_LADDER}
              formatRung={formatSeconds}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              customPlaceholder={t("agents.form.auto_placeholder")}
              min={1}
            
            invalid={invalidFields.has("autonomous.heartbeat_timeout_secs")}
            error={
              invalidFields.has("autonomous.heartbeat_timeout_secs")
                ? t("agents.form.u32_overflow")
                : undefined
            }/>

            <StepLadderInput
              label={t("agents.form.heartbeat_keep_recent")}
              value={value.autonomous.heartbeat_keep_recent}
              onChange={(next) => updateAutonomous({ heartbeat_keep_recent: next })}
              ladder={HEARTBEAT_KEEP_RECENT_LADDER}
              formatRung={formatCount}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              customPlaceholder={t("agents.form.auto_placeholder")}
              min={0}
            />

            <Field
              label={t("agents.form.heartbeat_channel")}
              hint={t("agents.form.heartbeat_channel_hint")}
            >
              <input
                type="text"
                value={value.autonomous.heartbeat_channel}
                onChange={(e) => updateAutonomous({ heartbeat_channel: e.target.value })}
                placeholder={t("agents.form.heartbeat_channel_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.quiet_hours")} hint={t("agents.form.quiet_hours_hint")}>
              <input
                type="text"
                value={value.autonomous.quiet_hours}
                onChange={(e) => updateAutonomous({ quiet_hours: e.target.value })}
                placeholder={t("agents.form.quiet_hours_placeholder")}
                className={inputClass}
              />
            </Field>
              </div>
            </AdvancedFields>
          </>
        )}
      </FormSection>

      <FormSection
        id="proactive_memory" shows={shows}
        title={t("config.sec_proactive_memory")}
        defaultOpen={false}
      >
        {/* Every field is an override of a kernel default, so every one of them
            leads with "inherit" — an agent that configures nothing here is the
            normal case, and the table is not written at all when that is what
            it says. */}
        <div className="grid grid-cols-2 gap-3">
          <TriStateField
            label={t("memory.proactive_enabled")}
            value={value.proactive_memory.enabled}
            onChange={(next) => updateProactiveMemory({ enabled: next })}
          />
          <TriStateField
            label={t("config.fld_auto_memorize")}
            value={value.proactive_memory.auto_memorize}
            onChange={(next) => updateProactiveMemory({ auto_memorize: next })}
          />
          <TriStateField
            label={t("config.fld_auto_retrieve")}
            value={value.proactive_memory.auto_retrieve}
            onChange={(next) => updateProactiveMemory({ auto_retrieve: next })}
          />
        </div>
        {/* BASIC: what automatic memory does. The scoping stamp, the
            consolidation gate, the extraction model and the similarity
            floor are depth. */}
        <AdvancedFields>
          <div className="grid grid-cols-2 gap-3">
          <TriStateField
            label={t("memory.session_scoped_recall")}
            value={value.proactive_memory.session_scoped_recall}
            onChange={(next) => updateProactiveMemory({ session_scoped_recall: next })}
          />
          <TriStateField
            label={t("agents.form.proactive_memory_allow_self_consolidation")}
            value={value.proactive_memory.allow_self_consolidation}
            onChange={(next) =>
              updateProactiveMemory({ allow_self_consolidation: next })
            }
          />
          <Field label={t("config.fld_extraction_model")} hint={t("config.desc_extraction_model")}>
            <input
              type="text"
              value={value.proactive_memory.extraction_model}
              onChange={(e) =>
                updateProactiveMemory({ extraction_model: e.target.value })
              }
              placeholder={t("agents.form.inherit_default")}
              className={inputClass}
            />
          </Field>
          </div>

        <StepLadderInput
          label={t("agents.form.proactive_memory_min_similarity")}
          value={value.proactive_memory.min_similarity}
          onChange={(next) => updateProactiveMemory({ min_similarity: next })}
          ladder={MIN_SIMILARITY_LADDER}
          formatRung={formatPercent}
          inheritLabel={t("model_param.inherit")}
          customLabel={t("model_param.custom")}
          min={0}
          max={1}
          step={0.01}
        />
        </AdvancedFields>
      </FormSection>

      <FormSection
        id="auto_dream" shows={shows}
        title={t("memory.tab_dreams")}
        defaultOpen={false}
        invalid={invalidFields.has("auto_dream_min_sessions")}
      >
        {/* Both are `Option`, so both lead with inherit: an agent that has
            never been given a threshold should not acquire one by being
            opened and saved. */}
        <div className="grid grid-cols-2 gap-3">
          <StepLadderInput
            label={t("agents.form.auto_dream_min_hours")}
            value={value.auto_dream_min_hours}
            onChange={(next) => update({ auto_dream_min_hours: next })}
            ladder={AUTO_DREAM_MIN_HOURS_LADDER}
            formatRung={formatHours}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={0}
          />
          <StepLadderInput
            label={t("agents.form.auto_dream_min_sessions")}
            value={value.auto_dream_min_sessions}
            onChange={(next) => update({ auto_dream_min_sessions: next })}
            ladder={AUTO_DREAM_MIN_SESSIONS_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={0}
          
            invalid={invalidFields.has("auto_dream_min_sessions")}
            error={
              invalidFields.has("auto_dream_min_sessions")
                ? t("agents.form.u32_overflow")
                : undefined
            }/>
        </div>
      </FormSection>

      <FormSection id="channel_overrides" shows={shows}
        title={t("agents.form.channel_overrides")}
        defaultOpen={false}
        // Every error this section can carry is the count shape, so the
        // prefix is the whole condition — a list of paths would drift as
        // fields are added.
        invalid={[...invalidFields].some((f) => f.startsWith("channel_overrides."))}
      >
        <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim mt-3">{t("agents.form.channel_overrides_group_reply")}</p>
        <div className="grid grid-cols-2 gap-3">
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_model")}>
                  <input type="text" value={value.channel_overrides.model}
                    onChange={(e) => updateChannelOverrides({ model: e.target.value })}
                    placeholder={t("agents.form.inherit_default")} className={inputClass} />
                </Field></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_system_prompt")}>
                  <input type="text" value={value.channel_overrides.system_prompt}
                    onChange={(e) => updateChannelOverrides({ system_prompt: e.target.value })}
                    placeholder={t("agents.form.inherit_default")} className={inputClass} />
                </Field></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_dm_policy")}>
                  <select value={value.channel_overrides.dm_policy}
                    onChange={(e) => updateChannelOverrides({ dm_policy: e.target.value as ManifestFormState["channel_overrides"]["dm_policy"] })}
                    className={inputClass}>
                    <option value="">{t("agents.form.inherit_default")}</option>
                    {(DM_POLICIES as readonly string[]).map((v) => (<option key={v} value={v}>{v}</option>))}
                  </select>
                </Field></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_group_policy")}>
                  <select value={value.channel_overrides.group_policy}
                    onChange={(e) => updateChannelOverrides({ group_policy: e.target.value as ManifestFormState["channel_overrides"]["group_policy"] })}
                    className={inputClass}>
                    <option value="">{t("agents.form.inherit_default")}</option>
                    {(GROUP_POLICIES as readonly string[]).map((v) => (<option key={v} value={v}>{v}</option>))}
                  </select>
                </Field></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_group_trigger_patterns")}>
                  <TagInput value={value.channel_overrides.group_trigger_patterns}
                    onChange={(next) => updateChannelOverrides({ group_trigger_patterns: next })}
                    placeholder={t("agents.form.inherit_default")} />
                </Field></div>
              <div><Toggle label={t("agents.form.channel_overrides_reply_precheck")} checked={value.channel_overrides.reply_precheck}
                  onChange={(checked) => updateChannelOverrides({ reply_precheck: checked })} /></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_reply_precheck_model")}>
                  <input type="text" value={value.channel_overrides.reply_precheck_model}
                    onChange={(e) => updateChannelOverrides({ reply_precheck_model: e.target.value })}
                    placeholder={t("agents.form.inherit_default")} className={inputClass} />
                </Field></div>
        </div>
        {/* BASIC: how the agent replies on a channel — the model, the
            prompt and the reply policies. Rate limits, debouncing,
            auto-routing, command control and thread ownership fold
            behind Advanced. */}
        <AdvancedFields>
        <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim mt-3">{t("agents.form.channel_overrides_group_limits")}</p>
        <div className="grid grid-cols-2 gap-3">
                <StepLadderInput label={t("agents.form.channel_overrides_rate_limit_per_minute")} value={value.channel_overrides.rate_limit_per_minute}
                  onChange={(next) => updateChannelOverrides({ rate_limit_per_minute: next })} ladder={CHANNEL_RATE_LIMIT_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.rate_limit_per_minute")}
                  error={invalidFields.has("channel_overrides.rate_limit_per_minute") ? t("agents.form.u32_overflow") : undefined} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_rate_limit_per_user")} value={value.channel_overrides.rate_limit_per_user}
                  onChange={(next) => updateChannelOverrides({ rate_limit_per_user: next })} ladder={CHANNEL_RATE_LIMIT_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.rate_limit_per_user")}
                  error={invalidFields.has("channel_overrides.rate_limit_per_user") ? t("agents.form.u32_overflow") : undefined} />
              <div className="col-span-2"></div>
              <div><Toggle label={t("agents.form.channel_overrides_threading")} checked={value.channel_overrides.threading}
                  onChange={(checked) => updateChannelOverrides({ threading: checked })} /></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_output_format")}>
                  <select value={value.channel_overrides.output_format}
                    onChange={(e) => updateChannelOverrides({ output_format: e.target.value as ManifestFormState["channel_overrides"]["output_format"] })}
                    className={inputClass}>
                    <option value="">{t("agents.form.inherit_default")}</option>
                    {(OUTPUT_FORMATS as readonly string[]).map((v) => (<option key={v} value={v}>{v}</option>))}
                  </select>
                </Field></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_usage_footer")}>
                  <select value={value.channel_overrides.usage_footer}
                    onChange={(e) => updateChannelOverrides({ usage_footer: e.target.value as ManifestFormState["channel_overrides"]["usage_footer"] })}
                    className={inputClass}>
                    <option value="">{t("agents.form.inherit_default")}</option>
                    {(USAGE_FOOTERS as readonly string[]).map((v) => (<option key={v} value={v}>{v}</option>))}
                  </select>
                </Field></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_typing_mode")}>
                  <select value={value.channel_overrides.typing_mode}
                    onChange={(e) => updateChannelOverrides({ typing_mode: e.target.value as ManifestFormState["channel_overrides"]["typing_mode"] })}
                    className={inputClass}>
                    <option value="">{t("agents.form.inherit_default")}</option>
                    {(TYPING_MODES as readonly string[]).map((v) => (<option key={v} value={v}>{v}</option>))}
                  </select>
                </Field></div>
        </div>
        <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim mt-3">{t("agents.form.channel_overrides_group_debounce")}</p>
        <div className="grid grid-cols-2 gap-3">
                <StepLadderInput label={t("agents.form.channel_overrides_message_debounce_ms")} value={value.channel_overrides.message_debounce_ms}
                  onChange={(next) => updateChannelOverrides({ message_debounce_ms: next })} ladder={CHANNEL_DEBOUNCE_MS_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.message_debounce_ms")}
                  error={invalidFields.has("channel_overrides.message_debounce_ms") ? t("agents.form.whole_number_required") : undefined} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_message_debounce_max_ms")} value={value.channel_overrides.message_debounce_max_ms}
                  onChange={(next) => updateChannelOverrides({ message_debounce_max_ms: next })} ladder={CHANNEL_DEBOUNCE_MAX_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.message_debounce_max_ms")}
                  error={invalidFields.has("channel_overrides.message_debounce_max_ms") ? t("agents.form.whole_number_required") : undefined} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_message_debounce_max_buffer")} value={value.channel_overrides.message_debounce_max_buffer}
                  onChange={(next) => updateChannelOverrides({ message_debounce_max_buffer: next })} ladder={CHANNEL_DEBOUNCE_BUFFER_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.message_debounce_max_buffer")}
                  error={invalidFields.has("channel_overrides.message_debounce_max_buffer") ? t("agents.form.whole_number_required") : undefined} />
              <div className="col-span-2"></div>
              <div><Toggle label={t("agents.form.channel_overrides_clear_done_reaction")} checked={value.channel_overrides.clear_done_reaction}
                  onChange={(checked) => updateChannelOverrides({ clear_done_reaction: checked })} /></div>
              <div><Toggle label={t("agents.form.channel_overrides_disable_commands")} checked={value.channel_overrides.disable_commands}
                  onChange={(checked) => updateChannelOverrides({ disable_commands: checked })} /></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_allowed_commands")}>
                  <TagInput value={value.channel_overrides.allowed_commands}
                    onChange={(next) => updateChannelOverrides({ allowed_commands: next })}
                    placeholder={t("agents.form.inherit_default")} />
                </Field></div>
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_blocked_commands")}>
                  <TagInput value={value.channel_overrides.blocked_commands}
                    onChange={(next) => updateChannelOverrides({ blocked_commands: next })}
                    placeholder={t("agents.form.inherit_default")} />
                </Field></div>
        </div>
        <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim mt-3">{t("agents.form.channel_overrides_group_routing")}</p>
        <div className="grid grid-cols-2 gap-3">
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_auto_route")}>
                  <select value={value.channel_overrides.auto_route}
                    onChange={(e) => updateChannelOverrides({ auto_route: e.target.value as ManifestFormState["channel_overrides"]["auto_route"] })}
                    className={inputClass}>
                    {(AUTO_ROUTE_STRATEGIES as readonly string[]).map((v) => (<option key={v} value={v}>{v}</option>))}
                  </select>
                </Field></div>
                <StepLadderInput label={t("agents.form.channel_overrides_auto_route_ttl_minutes")} value={value.channel_overrides.auto_route_ttl_minutes}
                  onChange={(next) => updateChannelOverrides({ auto_route_ttl_minutes: next })} ladder={CHANNEL_ROUTE_TTL_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.auto_route_ttl_minutes")}
                  error={invalidFields.has("channel_overrides.auto_route_ttl_minutes") ? t("agents.form.u32_overflow") : undefined} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_auto_route_confidence_threshold")} value={value.channel_overrides.auto_route_confidence_threshold}
                  onChange={(next) => updateChannelOverrides({ auto_route_confidence_threshold: next })} ladder={CHANNEL_ROUTE_CONFIDENCE_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.auto_route_confidence_threshold")}
                  error={invalidFields.has("channel_overrides.auto_route_confidence_threshold") ? t("agents.form.u32_overflow") : undefined} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_auto_route_sticky_bonus")} value={value.channel_overrides.auto_route_sticky_bonus}
                  onChange={(next) => updateChannelOverrides({ auto_route_sticky_bonus: next })} ladder={CHANNEL_ROUTE_BONUS_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.auto_route_sticky_bonus")}
                  error={invalidFields.has("channel_overrides.auto_route_sticky_bonus") ? t("agents.form.u32_overflow") : undefined} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_auto_route_divergence_count")} value={value.channel_overrides.auto_route_divergence_count}
                  onChange={(next) => updateChannelOverrides({ auto_route_divergence_count: next })} ladder={CHANNEL_ROUTE_DIVERGENCE_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.auto_route_divergence_count")}
                  error={invalidFields.has("channel_overrides.auto_route_divergence_count") ? t("agents.form.u32_overflow") : undefined} />
              <div className="col-span-2"></div>
        </div>
        <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim mt-3">{t("agents.form.channel_overrides_group_ownership")}</p>
        <div className="grid grid-cols-2 gap-3">
              <div className="col-span-2"><Field label={t("agents.form.channel_overrides_prefix_agent_name")}>
                  <select value={value.channel_overrides.prefix_agent_name}
                    onChange={(e) => updateChannelOverrides({ prefix_agent_name: e.target.value as ManifestFormState["channel_overrides"]["prefix_agent_name"] })}
                    className={inputClass}>
                    {(PREFIX_STYLES as readonly string[]).map((v) => (<option key={v} value={v}>{v}</option>))}
                  </select>
                </Field></div>
              <div><Toggle label={t("agents.form.channel_overrides_thread_ownership_enabled")} checked={value.channel_overrides.thread_ownership_enabled}
                  onChange={(checked) => updateChannelOverrides({ thread_ownership_enabled: checked })} /></div>
                <StepLadderInput label={t("agents.form.channel_overrides_conversation_ownership_ttl_seconds")} value={value.channel_overrides.conversation_ownership_ttl_seconds}
                  onChange={(next) => updateChannelOverrides({ conversation_ownership_ttl_seconds: next })} ladder={CHANNEL_THREAD_OWNERSHIP_TTL_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0}
                  invalid={invalidFields.has("channel_overrides.conversation_ownership_ttl_seconds")}
                  error={invalidFields.has("channel_overrides.conversation_ownership_ttl_seconds") ? t("agents.form.whole_number_required") : undefined} />
              <div className="col-span-2"></div>
              <div><Toggle label={t("agents.form.channel_overrides_conversation_ownership_include_dms")} checked={value.channel_overrides.conversation_ownership_include_dms}
                  onChange={(checked) => updateChannelOverrides({ conversation_ownership_include_dms: checked })} /></div>
        </div>
        </AdvancedFields>
      </FormSection>

      <FormSection
        id="skill_workshop" shows={shows}
        title={t("agents.form.skill_workshop")}
        defaultOpen={false}
        // Every error this section can carry is a count, so the prefix is the
        // whole condition — a list of paths would drift as fields are added.
        invalid={[...invalidFields].some((f) => f.startsWith("skill_workshop."))}
      >
        {/* The two switches are plain booleans, not tri-states: the Rust struct
            supplies them from its `Default`, so "absent" and "the default" are
            the same state and rendering a third one would invent a distinction
            the manifest does not have. */}
        <div className="flex flex-wrap gap-4">
          <Toggle
            label={t("agents.form.skill_workshop_enabled")}
            checked={value.skill_workshop.enabled}
            onChange={(checked) => updateSkillWorkshop({ enabled: checked })}
          />
          <Toggle
            label={t("agents.form.skill_workshop_auto_capture")}
            checked={value.skill_workshop.auto_capture}
            onChange={(checked) => updateSkillWorkshop({ auto_capture: checked })}
          />
        </div>
        {/* BASIC: the two switches. Approval, review and evolution modes
            and the pending caps fold behind Advanced. */}
        <AdvancedFields
          invalid={invalidFields.has("skill_workshop.max_pending_age_days")}
        >
        <div className="grid grid-cols-2 gap-3 mt-2">
          <Field label={t("agents.form.skill_workshop_approval_policy")}>
            <select
              value={value.skill_workshop.approval_policy}
              onChange={(e) =>
                updateSkillWorkshop({
                  approval_policy: e.target
                    .value as ManifestFormState["skill_workshop"]["approval_policy"],
                })
              }
              className={inputClass}
            >
              <option value="pending">pending</option>
              <option value="auto">auto</option>
            </select>
          </Field>
          <Field label={t("agents.form.skill_workshop_review_mode")}>
            <select
              value={value.skill_workshop.review_mode}
              onChange={(e) =>
                updateSkillWorkshop({
                  review_mode: e.target
                    .value as ManifestFormState["skill_workshop"]["review_mode"],
                })
              }
              className={inputClass}
            >
              <option value="heuristic">heuristic</option>
              <option value="threshold_llm">threshold_llm</option>
              <option value="none">none</option>
            </select>
          </Field>
          <StepLadderInput
            label={t("agents.form.skill_workshop_max_pending")}
            value={value.skill_workshop.max_pending}
            onChange={(next) => updateSkillWorkshop({ max_pending: next })}
            ladder={SKILL_WORKSHOP_MAX_PENDING_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={1}
            invalid={invalidFields.has("skill_workshop.max_pending")}
            error={
              invalidFields.has("skill_workshop.max_pending")
                ? t("agents.form.u32_overflow")
                : undefined
            }
          />
          <StepLadderInput
            label={t("agents.form.skill_workshop_max_pending_age_days")}
            value={value.skill_workshop.max_pending_age_days}
            onChange={(next) => updateSkillWorkshop({ max_pending_age_days: next })}
            ladder={SKILL_WORKSHOP_MAX_AGE_LADDER}
            formatRung={formatHours}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={1}
            invalid={invalidFields.has("skill_workshop.max_pending_age_days")}
            error={
              invalidFields.has("skill_workshop.max_pending_age_days")
                ? t("agents.form.u32_overflow")
                : undefined
            }/>
          <Field label={t("agents.form.skill_workshop_evolution_mode")}>
            <select
              value={value.skill_workshop.evolution_mode}
              onChange={(e) =>
                updateSkillWorkshop({
                  evolution_mode: e.target
                    .value as ManifestFormState["skill_workshop"]["evolution_mode"],
                })
              }
              className={inputClass}
            >
              <option value="free">free</option>
              <option value="controlled">controlled</option>
            </select>
          </Field>
        </div>
        </AdvancedFields>
      </FormSection>

      <FormSection id="compaction" shows={shows}
        title={t("config.sec_compaction")}
        defaultOpen={false}
        invalid={[
          "compaction.max_retries",
          "compaction.max_loop_steps_before_aggregate",
          "compaction.strip_reasoning_after_turns",
        ].some((f) => invalidFields.has(f))}
      >
        {/* Nine overrides of the kernel's compaction defaults, all `Option`, so
            every one of them leads with inherit and an untouched table is not
            written at all. */}
        <AdvancedFields
          invalid={[
            "compaction.max_retries",
            "compaction.max_loop_steps_before_aggregate",
            "compaction.strip_reasoning_after_turns",
          ].some((f) => invalidFields.has(f))}
        >
        <div className="grid grid-cols-2 gap-3">
          <StepLadderInput
            label={t("config.fld_threshold_messages")}
            value={value.compaction.threshold_messages}
            onChange={(next) => updateCompaction({ threshold_messages: next })}
            ladder={COMPACTION_THRESHOLD_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={1}
          />
          <StepLadderInput
            label={t("config.fld_keep_recent")}
            value={value.compaction.keep_recent}
            onChange={(next) => updateCompaction({ keep_recent: next })}
            ladder={COMPACTION_KEEP_RECENT_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={0}
          />
          <StepLadderInput
            label={t("config.fld_max_summary_tokens")}
            value={value.compaction.max_summary_tokens}
            onChange={(next) => updateCompaction({ max_summary_tokens: next })}
            ladder={COMPACTION_SUMMARY_TOKENS_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={1}
          />
          <StepLadderInput
            label={t("config.fld_token_threshold_ratio")}
            value={value.compaction.token_threshold_ratio}
            onChange={(next) => updateCompaction({ token_threshold_ratio: next })}
            ladder={COMPACTION_TOKEN_RATIO_LADDER}
            formatRung={formatPercent}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={0}
            max={1}
            step={0.01}
          />
          <StepLadderInput
            label={t("config.fld_max_chunk_chars")}
            value={value.compaction.max_chunk_chars}
            onChange={(next) => updateCompaction({ max_chunk_chars: next })}
            ladder={COMPACTION_CHUNK_CHARS_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={1}
          />
          <StepLadderInput
            label={t("config.fld_max_retries")}
            value={value.compaction.max_retries}
            onChange={(next) => updateCompaction({ max_retries: next })}
            ladder={COMPACTION_MAX_RETRIES_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={0}
          
            invalid={invalidFields.has("compaction.max_retries")}
            error={
              invalidFields.has("compaction.max_retries")
                ? t("agents.form.u32_overflow")
                : undefined
            }/>
          <StepLadderInput
            label={t("agents.form.compaction_max_loop_steps_before_aggregate")}
            value={value.compaction.max_loop_steps_before_aggregate}
            onChange={(next) => updateCompaction({ max_loop_steps_before_aggregate: next })}
            ladder={COMPACTION_LOOP_STEPS_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={1}
          
            invalid={invalidFields.has("compaction.max_loop_steps_before_aggregate")}
            error={
              invalidFields.has("compaction.max_loop_steps_before_aggregate")
                ? t("agents.form.u32_overflow")
                : undefined
            }/>
          <StepLadderInput
            label={t("agents.form.compaction_strip_reasoning_after_turns")}
            value={value.compaction.strip_reasoning_after_turns}
            onChange={(next) => updateCompaction({ strip_reasoning_after_turns: next })}
            ladder={COMPACTION_STRIP_REASONING_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={0}
          
            invalid={invalidFields.has("compaction.strip_reasoning_after_turns")}
            error={
              invalidFields.has("compaction.strip_reasoning_after_turns")
                ? t("agents.form.u32_overflow")
                : undefined
            }/>
        </div>
        {/* A tri-state select, not a toggle: the value is an `Option<bool>`, and
            a toggle collapses "inherit" and "false" onto the same unchecked
            state — so touching it would write an explicit `false` where the
            operator had not decided anything. */}
        <div className="mt-2">
          <TriStateField
            label={t("agents.form.compaction_aggregate_developer_loops")}
            value={value.compaction.aggregate_developer_loops}
            onChange={(next) => updateCompaction({ aggregate_developer_loops: next })}
          />
        </div>
        </AdvancedFields>
      </FormSection>

      {/* The memory subsystem's own configuration: which engine assembles the
          context, and the hooks that get to shape it. Typed rather than
          generic because the type is knowable — eight keys at the top, a hooks
          table, an optional sidecar and a list of plugin registries. */}
      <FormSection
        id="context_engine"
        shows={shows}
        title={t("config.sec_context_engine")}
        defaultOpen={false}
        invalid={invalidFields.has("context_engine.sidecar.command")}
      >
        <p className="text-[10px] text-text-dim/70 mb-2">
          {t("agents.form.context_engine_hint")}
        </p>
        <div className="grid grid-cols-2 gap-3">
          <Field
            label={t("agents.form.context_engine_engine")}
            hint={t("agents.form.context_engine_engine_hint")}
          >
            <OpenStringSelect
              value={ce.engine}
              options={CONTEXT_ENGINE_NAMES}
              inheritLabel={t("agents.form.inherit_default")}
              ariaLabel={t("agents.form.context_engine_engine")}
              onChange={(next) => updateContextEngine({ engine: next })}
            />
          </Field>
          <Field
            label={t("agents.form.context_engine_plugin")}
            hint={t("agents.form.context_engine_plugin_hint")}
          >
            <input
              type="text"
              value={ce.plugin}
              onChange={(e) => updateContextEngine({ plugin: e.target.value })}
              placeholder={t("agents.form.context_engine_plugin_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field
            label={t("agents.form.context_engine_plugin_stack")}
            hint={t("agents.form.context_engine_plugin_stack_hint")}
          >
            <TagInput
              value={ce.plugin_stack}
              onChange={(next) => updateContextEngine({ plugin_stack: next })}
              placeholder={t("agents.form.context_engine_plugin_stack_placeholder")}
            />
          </Field>
          <Field
            label={t("agents.form.context_engine_weights")}
            hint={t("agents.form.context_engine_weights_hint")}
          >
            <input
              type="text"
              value={ce.plugin_stack_weights}
              onChange={(e) =>
                updateContextEngine({ plugin_stack_weights: e.target.value })
              }
              placeholder={t("agents.form.context_engine_weights_placeholder")}
              className={inputClass}
            />
          </Field>
        </div>
        <div className="pt-2">
          <Toggle
            label={t("agents.form.context_engine_dedup")}
            checked={ce.deduplicate_file_reads}
            onChange={(checked) =>
              updateContextEngine({ deduplicate_file_reads: checked })
            }
          />
          <p className="text-[10px] text-text-dim/70">
            {t("agents.form.context_engine_dedup_hint")}
          </p>
        </div>

        {/* Plugin registries. The Rust default is the official registry, so an
            empty list and an absent key are different statements — the second
            keeps the default, the first switches browsing off. */}
        <div className="space-y-2 rounded-lg border border-border-subtle/40 p-2.5">
          <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
            {t("agents.form.context_engine_registries")}
          </p>
          <p className="text-[10px] text-text-dim/70">
            {t("agents.form.context_engine_registries_hint")}
          </p>
          {(ce.plugin_registries ?? []).map((row, idx) => (
            <div key={row._uid} className="flex items-center gap-2">
              <input
                type="text"
                value={row.name}
                onChange={(e) =>
                  updateContextEngine({
                    plugin_registries: patchListItem(ce.plugin_registries ?? [], idx, {
                      ...row,
                      name: e.target.value,
                    }),
                  })
                }
                placeholder={t("agents.form.context_engine_registry_name")}
                aria-label={`${t("agents.form.context_engine_registry_name")} ${idx + 1}`}
                className={inputClass}
              />
              <input
                type="text"
                value={row.github_repo}
                onChange={(e) =>
                  updateContextEngine({
                    plugin_registries: patchListItem(ce.plugin_registries ?? [], idx, {
                      ...row,
                      github_repo: e.target.value,
                    }),
                  })
                }
                placeholder={t("agents.form.context_engine_registry_repo")}
                aria-label={`${t("agents.form.context_engine_registry_repo")} ${idx + 1}`}
                className={inputClass}
              />
              <button
                type="button"
                onClick={() =>
                  updateContextEngine({
                    plugin_registries: (ce.plugin_registries ?? []).filter(
                      (_, i) => i !== idx,
                    ),
                  })
                }
                className="shrink-0 text-text-dim hover:text-error"
                aria-label={`${t("agents.form.context_engine_registry_remove")} ${idx + 1}`}
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
          {(ce.plugin_registries ?? []).length === 0 && (
            <TriStateNote
              absent={ce.plugin_registries === null}
              absentText={t("agents.form.context_engine_registry_default")}
              emptyText={t("agents.form.context_engine_registry_empty")}
              declareEmptyLabel={t("agents.form.context_engine_registry_declare_empty")}
              restoreDefaultLabel={t("agents.form.context_engine_registry_use_default")}
              onDeclareEmpty={() => updateContextEngine({ plugin_registries: [] })}
              onRestoreDefault={() => updateContextEngine({ plugin_registries: null })}
            />
          )}
          <button
            type="button"
            onClick={() =>
              updateContextEngine({
                plugin_registries: [
                  ...(ce.plugin_registries ?? []),
                  { _uid: generateUid(), name: "", github_repo: "" },
                ],
              })
            }
            className="flex items-center gap-1 text-xs text-brand hover:underline"
          >
            <Plus className="w-3.5 h-3.5" />
            {t("agents.form.context_engine_registry_add")}
          </button>
        </div>

        {/* Sidecar. `Option<ContextEngineSidecarConfig>` on the Rust side, so
            the switch is the Option and a command with the switch off writes
            nothing — an `[sidecar]` nobody asked for would make the daemon
            delegate every turn to a process that is not there. */}
        <div className="space-y-2 rounded-lg border border-border-subtle/40 p-2.5">
          <Toggle
            label={t("agents.form.context_engine_sidecar")}
            checked={ce.sidecar_enabled}
            onChange={(checked) => updateContextEngine({ sidecar_enabled: checked })}
          />
          <p className="text-[10px] text-text-dim/70">
            {t("agents.form.context_engine_sidecar_hint")}
          </p>
          {ce.sidecar_enabled && (
            <>
              <div className="grid grid-cols-2 gap-3">
                <Field label={t("agents.form.context_engine_sidecar_command")}>
                  <input
                    type="text"
                    value={ce.sidecar.command}
                    onChange={(e) =>
                      updateContextEngine({
                        sidecar: { ...ce.sidecar, command: e.target.value },
                      })
                    }
                    placeholder={t("agents.form.context_engine_sidecar_command_placeholder")}
                    className={inputClass}
                  />
                </Field>
                <Field
                  label={t("agents.form.context_engine_sidecar_args")}
                  hint={t("agents.form.context_engine_sidecar_args_hint")}
                >
                  <TagInput
                    value={ce.sidecar.args}
                    onChange={(next) =>
                      updateContextEngine({ sidecar: { ...ce.sidecar, args: next } })
                    }
                    placeholder={t("agents.form.context_engine_sidecar_args_placeholder")}
                  />
                </Field>
              </div>
              <StepLadderInput
                label={t("agents.form.context_engine_sidecar_timeout")}
                value={ce.sidecar.request_timeout_secs}
                onChange={(next) =>
                  updateContextEngine({
                    sidecar: { ...ce.sidecar, request_timeout_secs: next },
                  })
                }
                ladder={ASYNC_TASK_TIMEOUT_LADDER}
                formatRung={formatSeconds}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={0}
              />
              <PreservedValues
                entries={ce.sidecar.preserved ?? {}}
                hint={t("agents.form.context_engine_preserved_hint")}
              />
            </>
          )}
        </div>

        {/* Hooks. Nine script paths, and the knobs that govern how they run
            folded behind Advanced: which scripts run is the choice an operator
            comes here to make, and how long they may take is the follow-up. */}
        <div className="space-y-2 rounded-lg border border-border-subtle/40 p-2.5">
          <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
            {t("agents.form.context_engine_hooks")}
          </p>
          <p className="text-[10px] text-text-dim/70">
            {t("agents.form.context_engine_hooks_hint")}
          </p>
          <div className="grid grid-cols-2 gap-3">
            <HookScriptField
              label={t("agents.form.context_engine_hook_ingest")}
              value={hook.ingest}
              onChange={(next) => updateHook({ ingest: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_after_turn")}
              value={hook.after_turn}
              onChange={(next) => updateHook({ after_turn: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_bootstrap")}
              value={hook.bootstrap}
              onChange={(next) => updateHook({ bootstrap: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_assemble")}
              value={hook.assemble}
              onChange={(next) => updateHook({ assemble: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_compact")}
              value={hook.compact}
              onChange={(next) => updateHook({ compact: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_transform_tool_result")}
              value={hook.transform_tool_result}
              onChange={(next) => updateHook({ transform_tool_result: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_prepare_subagent")}
              value={hook.prepare_subagent}
              onChange={(next) => updateHook({ prepare_subagent: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_merge_subagent")}
              value={hook.merge_subagent}
              onChange={(next) => updateHook({ merge_subagent: next })}
            />
            <HookScriptField
              label={t("agents.form.context_engine_hook_on_event")}
              value={hook.on_event}
              onChange={(next) => updateHook({ on_event: next })}
            />
          </div>
          <AdvancedFields invalid={invalidFields.has("context_engine.sidecar.command")}>
            <div className="grid grid-cols-2 gap-3">
              <Field
                label={t("agents.form.context_engine_runtime")}
                hint={t("agents.form.context_engine_runtime_hint")}
              >
                <OpenStringSelect
                  value={hook.runtime}
                  options={HOOK_RUNTIMES}
                  inheritLabel={t("agents.form.inherit_default")}
                  ariaLabel={t("agents.form.context_engine_runtime")}
                  onChange={(next) => updateHook({ runtime: next })}
                />
              </Field>
              <StepLadderInput
                label={t("agents.form.context_engine_hook_timeout")}
                value={hook.hook_timeout_secs}
                onChange={(next) => updateHook({ hook_timeout_secs: next })}
                ladder={ASYNC_TASK_TIMEOUT_LADDER}
                formatRung={formatSeconds}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={1}
              />
              <StepLadderInput
                label={t("agents.form.context_engine_hook_retries")}
                value={hook.max_retries}
                onChange={(next) => updateHook({ max_retries: next })}
                ladder={HOOK_RETRIES_LADDER}
                formatRung={formatCount}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={0}
              />
              <StepLadderInput
                label={t("agents.form.context_engine_hook_retry_delay")}
                value={hook.retry_delay_ms}
                onChange={(next) => updateHook({ retry_delay_ms: next })}
                ladder={CHANNEL_DEBOUNCE_MS_LADDER}
                formatRung={(value) => `${value} ms`}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={0}
              />
              <StepLadderInput
                label={t("agents.form.context_engine_hook_memory")}
                value={hook.max_memory_mb}
                onChange={(next) => updateHook({ max_memory_mb: next })}
                ladder={HOOK_MEMORY_MB_LADDER}
                formatRung={(value) => `${value} MB`}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={1}
              />
              <StepLadderInput
                label={t("agents.form.context_engine_after_turn_queue")}
                value={hook.after_turn_queue_depth}
                onChange={(next) => updateHook({ after_turn_queue_depth: next })}
                ladder={HOOK_QUEUE_DEPTH_LADDER}
                formatRung={formatCount}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={1}
              />
              <StepLadderInput
                label={t("agents.form.context_engine_priority")}
                value={hook.priority}
                onChange={(next) => updateHook({ priority: next })}
                ladder={HOOK_PRIORITY_LADDER}
                formatRung={formatCount}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                // Signed: a lower priority runs later, and the Rust default is 0.
                min={-2147483648}
                max={2147483647}
              />
              <Field
                label={t("agents.form.context_engine_on_failure")}
                hint={t("agents.form.context_engine_on_failure_hint")}
              >
                <select
                  value={hook.on_hook_failure}
                  onChange={(e) =>
                    updateHook({
                      on_hook_failure: e.target
                        .value as ManifestFormState["context_engine"]["hooks"]["on_hook_failure"],
                    })
                  }
                  // The visible label is a `<span>`, so the control carries its
                  // own name. A select is never named by a placeholder, which
                  // is why every unlabelled select in this form is mute.
                  aria-label={t("agents.form.context_engine_on_failure")}
                  className={inputClass}
                >
                  {HOOK_FAILURE_POLICIES.map((policy) => (
                    <option key={policy} value={policy}>
                      {policy}
                    </option>
                  ))}
                </select>
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Field
                label={t("agents.form.context_engine_ingest_filter")}
                hint={t("agents.form.context_engine_ingest_filter_hint")}
              >
                <input
                  type="text"
                  value={hook.ingest_filter}
                  onChange={(e) => updateHook({ ingest_filter: e.target.value })}
                  aria-label={t("agents.form.context_engine_ingest_filter")}
                  className={inputClass}
                />
              </Field>
              <Field
                label={t("agents.form.context_engine_ingest_regex")}
                hint={t("agents.form.context_engine_ingest_regex_hint")}
              >
                <input
                  type="text"
                  value={hook.ingest_regex}
                  onChange={(e) => updateHook({ ingest_regex: e.target.value })}
                  aria-label={t("agents.form.context_engine_ingest_regex")}
                  className={inputClass}
                />
              </Field>
              <Field
                label={t("agents.form.context_engine_only_for_agents")}
                hint={t("agents.form.context_engine_only_for_agents_hint")}
              >
                <TagInput
                  value={hook.only_for_agent_ids}
                  onChange={(next) => updateHook({ only_for_agent_ids: next })}
                  placeholder={t("agents.form.context_engine_only_for_agents_placeholder")}
                />
              </Field>
              <Field
                label={t("agents.form.context_engine_allowed_secrets")}
                hint={t("agents.form.context_engine_allowed_secrets_hint")}
              >
                <TagInput
                  value={hook.allowed_secrets}
                  onChange={(next) => updateHook({ allowed_secrets: next })}
                  placeholder={t("agents.form.context_engine_allowed_secrets_placeholder")}
                />
              </Field>
              <Field label={t("agents.form.context_engine_otel_endpoint")}>
                <input
                  type="text"
                  value={hook.otel_endpoint}
                  onChange={(e) => updateHook({ otel_endpoint: e.target.value })}
                  placeholder={t("agents.form.context_engine_otel_endpoint_placeholder")}
                  className={inputClass}
                />
              </Field>
              <Field label={t("agents.form.context_engine_hook_protocol")}>
                <input
                  type="text"
                  value={hook.hook_protocol_version}
                  onChange={(e) => updateHook({ hook_protocol_version: e.target.value })}
                  placeholder={t("agents.form.context_engine_hook_protocol_placeholder")}
                  className={inputClass}
                />
              </Field>
            </div>
            <div className="grid grid-cols-3 gap-3">
              <StepLadderInput
                label={t("agents.form.context_engine_cache_ttl")}
                value={hook.hook_cache_ttl_secs}
                onChange={(next) => updateHook({ hook_cache_ttl_secs: next })}
                ladder={HOOK_CACHE_TTL_LADDER}
                formatRung={formatSeconds}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={0}
              />
              <StepLadderInput
                label={t("agents.form.context_engine_assemble_cache_ttl")}
                value={hook.assemble_cache_ttl_secs}
                onChange={(next) => updateHook({ assemble_cache_ttl_secs: next })}
                ladder={HOOK_CACHE_TTL_LADDER}
                formatRung={formatSeconds}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={0}
              />
              <StepLadderInput
                label={t("agents.form.context_engine_compact_cache_ttl")}
                value={hook.compact_cache_ttl_secs}
                onChange={(next) => updateHook({ compact_cache_ttl_secs: next })}
                ladder={HOOK_CACHE_TTL_LADDER}
                formatRung={formatSeconds}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                min={0}
              />
            </div>
            <div className="flex flex-wrap gap-4 pt-1">
              <Toggle
                label={t("agents.form.context_engine_shared_state")}
                checked={hook.enable_shared_state}
                onChange={(checked) => updateHook({ enable_shared_state: checked })}
              />
              <Toggle
                label={t("agents.form.context_engine_persistent_subprocess")}
                checked={hook.persistent_subprocess}
                onChange={(checked) => updateHook({ persistent_subprocess: checked })}
              />
              <Toggle
                label={t("agents.form.context_engine_prewarm")}
                checked={hook.prewarm_subprocesses}
                onChange={(checked) => updateHook({ prewarm_subprocesses: checked })}
              />
              <Toggle
                label={t("agents.form.context_engine_allow_filesystem")}
                checked={hook.allow_filesystem}
                onChange={(checked) => updateHook({ allow_filesystem: checked })}
              />
              <Toggle
                label={t("agents.form.context_engine_allow_network")}
                checked={hook.allow_network}
                onChange={(checked) => updateHook({ allow_network: checked })}
              />
            </div>
            <p className="text-[10px] text-text-dim/70">
              {t("agents.form.context_engine_sandbox_hint")}
            </p>

            {/* The circuit breaker is an `Option<CircuitBreakerConfig>`: the
                switch is the Option and its two knobs are optional even then,
                because the Rust side gives each its own default. */}
            <div className="space-y-2 rounded-lg border border-border-subtle/40 p-2.5">
              <Toggle
                label={t("agents.form.context_engine_circuit")}
                checked={hook.circuit_enabled}
                onChange={(checked) => updateHook({ circuit_enabled: checked })}
              />
              {hook.circuit_enabled && (
                <div className="grid grid-cols-2 gap-3">
                  <StepLadderInput
                    label={t("agents.form.context_engine_circuit_failures")}
                    value={hook.circuit_max_failures}
                    onChange={(next) => updateHook({ circuit_max_failures: next })}
                    ladder={HOOK_CIRCUIT_FAILURES_LADDER}
                    formatRung={formatCount}
                    inheritLabel={t("model_param.inherit")}
                    customLabel={t("model_param.custom")}
                    min={1}
                  />
                  <StepLadderInput
                    label={t("agents.form.context_engine_circuit_reset")}
                    value={hook.circuit_reset_secs}
                    onChange={(next) => updateHook({ circuit_reset_secs: next })}
                    ladder={HOOK_CIRCUIT_RESET_LADDER}
                    formatRung={formatSeconds}
                    inheritLabel={t("model_param.inherit")}
                    customLabel={t("model_param.custom")}
                    min={1}
                  />
                </div>
              )}
            </div>

            {/* `HashMap<String, String>`: a name and what it is for. One type,
                so the row editor has no type column to offer. */}
            <div className="space-y-1">
              <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
                {t("agents.form.context_engine_env_schema")}
              </p>
              <p className="text-[10px] text-text-dim/70">
                {t("agents.form.context_engine_env_schema_hint")}
              </p>
              {hook.env_schema.map((row, idx) => (
                <JsonRowEditor
                  key={row._uid}
                  row={row}
                  index={idx}
                  types={["string"]}
                  labels={{
                    key: t("agents.form.context_engine_env_schema_key"),
                    type: t("agents.form.context_engine_env_schema_type"),
                    value: t("agents.form.context_engine_env_schema_value"),
                    remove: t("agents.form.context_engine_env_schema_remove"),
                  }}
                  onChange={(next) =>
                    updateHook({ env_schema: patchListItem(hook.env_schema, idx, next) })
                  }
                  onRemove={() =>
                    updateHook({ env_schema: hook.env_schema.filter((_, i) => i !== idx) })
                  }
                />
              ))}
              <button
                type="button"
                onClick={() =>
                  updateHook({
                    env_schema: [
                      ...hook.env_schema,
                      { _uid: generateUid(), key: "", valueType: "string", value: "" },
                    ],
                  })
                }
                className="flex items-center gap-1 text-xs text-brand hover:underline"
              >
                <Plus className="w-3.5 h-3.5" />
                {t("agents.form.context_engine_env_schema_add")}
              </button>
            </div>

            {/* `hook_schemas` and any key a later release adds. Shown rather
                than hidden: a key the operator cannot see is indistinguishable
                from one the editor dropped. */}
            <PreservedValues
              entries={hook.preserved ?? {}}
              hint={t("agents.form.context_engine_preserved_hint")}
            />
          </AdvancedFields>
        </div>
        <PreservedValues
          entries={ce.preserved ?? {}}
          hint={t("agents.form.context_engine_preserved_hint")}
        />
      </FormSection>

      <FormSection
        id="async_tasks" shows={shows}
        title={t("agents.form.async_tasks")}
        defaultOpen={false}
      >
        <StepLadderInput
          label={t("config.fld_default_timeout_secs")}
          value={value.async_tasks.default_timeout_secs}
          onChange={(next) =>
            update({ async_tasks: { ...value.async_tasks, default_timeout_secs: next } })
          }
          ladder={ASYNC_TASK_TIMEOUT_LADDER}
          formatRung={formatSeconds}
          inheritLabel={t("model_param.inherit")}
          customLabel={t("model_param.custom")}
          min={1}
        />
        <p className="text-[10px] text-text-dim/70">{t("config.desc_default_timeout_secs")}</p>
        <Toggle
          label={t("agents.form.async_tasks_notify_on_timeout")}
          checked={value.async_tasks.notify_on_timeout}
          onChange={(checked) =>
            update({ async_tasks: { ...value.async_tasks, notify_on_timeout: checked } })
          }
        />
      </FormSection>

      <FormSection id="routing" shows={shows} title={t("agents.form.routing")} defaultOpen={false}>
        {routingInertReason === "stable_mode" && (
          <div className="mb-2">
            <ExtrasOverrideHint message={t("agents.form.routing_stable_inert")} />
          </div>
        )}
        {/*
          The routing engine, and the only place the choice is made. It used
          to be three controls across two sections — a "Router mode" select
          and an "Opt out of routing" toggle in the model section, an enable
          toggle here — and they could name one engine while running another:
          a manifest saying `mode = "flexible"` with the override on runs the
          tiers rather than the profile router, and one with no `[routing]`
          table runs neither. The selector writes the fields together
          (lib/routingEngine.ts holds the table), so what the operator picks
          is what the kernel resolves.
        */}
        <Field
          label={t("agents.form.routing_engine")}
          hint={t("agents.form.routing_engine_hint")}
          htmlFor="manifest-routing-engine"
        >
          <select
            id="manifest-routing-engine"
            value={routingEngine}
            onChange={(e) => setRoutingEngine(e.target.value as RoutingEngine)}
            className={inputClass}
          >
            <option value="fixed">{t("agents.form.routing_engine_fixed")}</option>
            <option value="effort">{t("agents.form.routing_engine_effort")}</option>
            <option value="profile">{t("agents.form.routing_engine_profile")}</option>
          </select>
        </Field>
        <p className="text-[10px] text-text-dim/70 mt-1">
          {routingEngine === "fixed" && t("agents.form.routing_engine_fixed_note")}
          {routingEngine === "effort" && t("agents.form.routing_engine_effort_note")}
          {routingEngine === "profile" && t("agents.form.routing_engine_profile_note")}
        </p>
        {/* Under the profile engine the tiers are not editable here, so the
            operator cannot otherwise tell what an unmatched turn runs on. The
            line names the models the kernel would use — the fallback table
            when there is one, and the daemon's default routing when there is
            not (which reaches an agent with no `[routing]` of its own). */}
        {routingEngine === "profile" && (
          <p className="text-[10px] text-text-dim/70 mt-1">
            {value.routing.enabled
              ? t("agents.form.routing_engine_profile_fallback", {
                  simple: routingTierModel(value, "simple_model"),
                  medium: routingTierModel(value, "medium_model"),
                  complex: routingTierModel(value, "complex_model"),
                })
              : t("agents.form.routing_engine_profile_fallback_none")}
          </p>
        )}
        {/* The tiers are the effort engine's own controls, and render only
            while it runs. Under the profile engine they are the fallback the
            kernel reaches for when no profile matches — preserved in the file
            (lib/routingEngine.ts leaves the table alone for that engine), but
            not the thing the operator chose, so they are not presented as the
            choice. Under the fixed engine nothing consults them at all. */}
        {routingEngine === "effort" && (
          <div className="space-y-2 mt-2">
            <div className="grid grid-cols-3 gap-3">
              <Field label={t("agents.form.simple_model")}>
                <ModelPicker
                  label={t("agents.form.simple_model")}
                  variant="model"
                  allowCustom
                  value={asModelName(value.routing.simple_model)}
                  onChange={(next) => updateRouting({ simple_model: next.model })}
                  onClear={() => updateRouting({ simple_model: "" })}
                  models={models}
                  {...pickerCatalog}
                />
              </Field>
              <Field label={t("agents.form.medium_model")}>
                <ModelPicker
                  label={t("agents.form.medium_model")}
                  variant="model"
                  allowCustom
                  value={asModelName(value.routing.medium_model)}
                  onChange={(next) => updateRouting({ medium_model: next.model })}
                  onClear={() => updateRouting({ medium_model: "" })}
                  models={models}
                  {...pickerCatalog}
                />
              </Field>
              <Field label={t("agents.form.complex_model")}>
                <ModelPicker
                  label={t("agents.form.complex_model")}
                  variant="model"
                  allowCustom
                  value={asModelName(value.routing.complex_model)}
                  onChange={(next) => updateRouting({ complex_model: next.model })}
                  onClear={() => updateRouting({ complex_model: "" })}
                  models={models}
                  {...pickerCatalog}
                />
              </Field>
            </div>
            {/* A blank picker reads as "no model", and it is not: the table
                arms the tier router by its presence, and serde fills every key
                it leaves out from `ModelRoutingConfig::default()`. The line
                names those values rather than leaving the operator to
                discover them from a routed turn (lib/routingEngine.ts holds
                them, guarded against the Rust `Default`). */}
            <p className="text-[10px] text-text-dim/70">
              {t("agents.form.routing_tier_blank_defaults", {
                simple: ROUTING_TIER_DEFAULTS.simple_model,
                medium: ROUTING_TIER_DEFAULTS.medium_model,
                complex: ROUTING_TIER_DEFAULTS.complex_model,
                simpleThreshold: ROUTING_TIER_DEFAULTS.simple_threshold,
                complexThreshold: ROUTING_TIER_DEFAULTS.complex_threshold,
              })}
            </p>
            <div className="grid grid-cols-2 gap-3">
              <StepLadderInput
                label={t("agents.form.simple_threshold")}
                value={value.routing.simple_threshold}
                onChange={(next) => updateRouting({ simple_threshold: next })}
                ladder={ROUTING_THRESHOLD_LADDER}
                formatRung={formatCount}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                customPlaceholder={t("agents.form.simple_threshold_placeholder")}
                min={0}
              />

              <StepLadderInput
                label={t("agents.form.complex_threshold")}
                value={value.routing.complex_threshold}
                onChange={(next) => updateRouting({ complex_threshold: next })}
                ladder={ROUTING_THRESHOLD_LADDER}
                formatRung={formatCount}
                inheritLabel={t("model_param.inherit")}
                customLabel={t("model_param.custom")}
                customPlaceholder={t("agents.form.complex_threshold_placeholder")}
                min={0}
              />

            </div>
          </div>
        )}
        {/* BASIC: the engine selector and, when it is the effort engine, its
            three tiers. Orphan reconciliation and the tool profile are depth. */}
        <AdvancedFields>
          <div className="grid grid-cols-2 gap-3 mt-2">
            <Field label={t("agents.form.reconcile_orphans")} hint={t("agents.form.inherit_default")}>
              <select
                value={value.reconcile_orphans}
                onChange={(e) =>
                  update({
                    reconcile_orphans: e.target.value as ManifestFormState["reconcile_orphans"],
                  })
                }
                className={inputClass}
              >
                <option value="">{t("agents.form.inherit_default")}</option>
                <option value="keep">keep</option>
                <option value="warn">warn</option>
                <option value="delete">delete</option>
              </select>
            </Field>
          </div>
          <div className="grid grid-cols-2 gap-3 mt-2">
            <Field label={t("agents.form.profile")} hint={t("agents.form.inherit_default")}>
              <select
                value={value.profile}
                onChange={(e) =>
                  update({
                    profile: e.target.value as ManifestFormState["profile"],
                  })
                }
                className={inputClass}
              >
                <option value="">{t("agents.form.inherit_default")}</option>
                <option value="minimal">minimal</option>
                <option value="coding">coding</option>
                <option value="research">research</option>
                <option value="messaging">messaging</option>
                <option value="automation">automation</option>
                <option value="full">full</option>
                <option value="custom">custom</option>
              </select>
            </Field>
          </div>
        </AdvancedFields>
      </FormSection>

      <FormSection id="context_injection" shows={shows} title={t("agents.form.context_injection")} defaultOpen={false}>
        <p className="text-[10px] text-text-dim/70 mb-2">
          {t("agents.form.context_injection_hint")}
        </p>
        {value.context_injection.map((ci, idx) => (
          <div
            key={ci._uid}
            className="rounded-lg border border-border-subtle/60 bg-main/40 p-2 mb-2 space-y-2"
          >
            <div className="flex items-center justify-between">
              <span className="text-[10px] font-bold text-text-dim uppercase">#{idx + 1}</span>
              <button
                type="button"
                onClick={() => update({ context_injection: value.context_injection.filter((_, i) => i !== idx) })}
                className="text-text-dim hover:text-error"
                aria-label={t("agents.form.remove_context_injection")}
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <input
                type="text"
                value={ci.name}
                onChange={(e) => update({ context_injection: patchListItem(value.context_injection, idx, { ...ci, name: e.target.value }) })}
                placeholder={t("agents.form.injection_name")}
                className={inputClass}
              />
              <select
                value={ci.position}
                onChange={(e) => update({
                  context_injection: patchListItem(value.context_injection, idx, {
                    ...ci,
                    position: e.target.value as ManifestFormState["context_injection"][number]["position"],
                  }),
                })}
                className={inputClass}
              >
                <option value="system">{t("agents.form.position_system")}</option>
                <option value="before_user">{t("agents.form.position_before_user")}</option>
                <option value="after_reset">{t("agents.form.position_after_reset")}</option>
              </select>
            </div>
            <textarea
              value={ci.content}
              onChange={(e) => update({ context_injection: patchListItem(value.context_injection, idx, { ...ci, content: e.target.value }) })}
              placeholder={t("agents.form.injection_content")}
              rows={2}
              className={textareaClass}
            />
            <input
              type="text"
              value={ci.condition}
              onChange={(e) => update({ context_injection: patchListItem(value.context_injection, idx, { ...ci, condition: e.target.value }) })}
              placeholder={t("agents.form.injection_condition")}
              className={inputClass}
            />
          </div>
        ))}
        <button
          type="button"
          onClick={() =>
            update({
              context_injection: [
                ...value.context_injection,
                { _uid: generateUid(), name: "", content: "", position: "system", condition: "" },
              ],
            })
          }
          className="flex items-center gap-1 text-xs text-brand hover:underline"
        >
          <Plus className="w-3.5 h-3.5" />
          {t("agents.form.add_injection")}
        </button>
      </FormSection>

      <FormSection id="response_format" shows={shows}
        title={t("agents.form.response_format")}
        defaultOpen={false}
        invalid={invalidFields.has("response_format.schema")}
      >
        {value.response_format.mode === "text" && extras.topLevel.response_format !== undefined && (
          <ExtrasOverrideHint message={t("agents.form.response_format_extras_hint")} />
        )}
        <Field label={t("agents.form.response_format_mode")}>
          <select
            value={value.response_format.mode}
            onChange={(e) => {
              const mode = e.target.value as ManifestFormState["response_format"]["mode"];
              if (mode === "text") update({ response_format: { mode } });
              else if (mode === "json") update({ response_format: { mode } });
              else update({ response_format: { mode, name: "", schema: "{}", strict: false } });
            }}
            className={inputClass}
          >
            <option value="text">{t("agents.form.response_text")}</option>
            <option value="json">{t("agents.form.response_json")}</option>
            <option value="json_schema">{t("agents.form.response_json_schema")}</option>
          </select>
        </Field>
        {jsonSchemaFormat && (
          <div className="space-y-2 mt-2">
            <Field label={t("agents.form.schema_name")}>
              <input
                type="text"
                value={jsonSchemaFormat.name}
                onChange={(e) =>
                  update({
                    response_format: {
                      // Spread rather than rebuild: the state may carry keys
                      // the form does not render (stashed by parse), and an
                      // edit inside the mode must not drop them. Picking a
                      // different mode above does drop them — that is the
                      // operator replacing the format, not editing it.
                      ...jsonSchemaFormat,
                      name: e.target.value,
                    },
                  })
                }
                placeholder={t("agents.form.schema_name_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field
              label={t("agents.form.schema_body")}
              required
              invalid={invalidFields.has("response_format.schema")}
              error={t("agents.form.schema_invalid_error")}
              errorId="agent-manifest-response-schema-error"
            >
              <textarea
                id="agent-manifest-response-schema"
                value={jsonSchemaFormat.schema}
                onChange={(e) =>
                  update({
                    response_format: {
                      ...jsonSchemaFormat,
                      schema: e.target.value,
                    },
                  })
                }
                rows={6}
                className={textareaClass}
                aria-label={t("agents.form.schema_body")}
                aria-invalid={invalidFields.has("response_format.schema") || undefined}
                aria-required="true"
                aria-describedby={
                  invalidFields.has("response_format.schema")
                    ? "agent-manifest-response-schema-error"
                    : undefined
                }
              />
            </Field>
            <Toggle
              label={t("agents.form.strict")}
              checked={jsonSchemaFormat.strict}
              onChange={(checked) =>
                update({
                  response_format: {
                    ...jsonSchemaFormat,
                    strict: checked,
                  },
                })
              }
            />
          </div>
        )}
      </FormSection>

      <FormSection id="lifecycle" shows={shows}
        title={t("agents.form.lifecycle")}
        defaultOpen={false}
        invalid={
          invalidFields.has("max_history_messages") ||
          invalidFields.has("max_concurrent_invocations")
        }
      >
        {/* BASIC: whether this agent runs, and whether an automated invocation
            reuses its session. Everything else in the section — the export and
            search switches, plugins, the history and concurrency caps — folds
            behind Advanced. The exec policy used to be here too; it is a
            permission and now has its own card in that group. */}
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.session_mode")}>
            <select
              value={value.session_mode}
              onChange={(e) =>
                update({ session_mode: e.target.value as ManifestFormState["session_mode"] })
              }
              className={inputClass}
            >
              <option value="persistent">{t("agents.form.session_persistent")}</option>
              <option value="new">{t("agents.form.session_new")}</option>
            </select>
          </Field>
        </div>
        <div className="flex flex-wrap gap-4 pt-2">
          <Toggle
            label={t("agents.form.enabled")}
            checked={value.enabled}
            onChange={(checked) => update({ enabled: checked })}
          />
        </div>
        <AdvancedFields
          invalid={
            invalidFields.has("max_history_messages") ||
            invalidFields.has("max_concurrent_invocations")
          }
        >
          <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.rl_export")}>
            <select
              value={value.rl_export}
              onChange={(e) =>
                update({ rl_export: e.target.value as ManifestFormState["rl_export"] })
              }
              className={inputClass}
            >
              <option value="">{t("agents.form.inherit_default")}</option>
              <option value="true">{t("common.yes")}</option>
              <option value="false">{t("common.no")}</option>
            </select>
          </Field>
          <Field label={t("agents.form.web_search_aug")}>
            <select
              value={value.web_search_augmentation}
              onChange={(e) =>
                update({
                  web_search_augmentation:
                    e.target.value as ManifestFormState["web_search_augmentation"],
                })
              }
              className={inputClass}
            >
              <option value="off">{t("agents.form.web_search_off")}</option>
              <option value="auto">{t("agents.form.web_search_auto")}</option>
              <option value="always">{t("agents.form.web_search_always")}</option>
            </select>
          </Field>
          <Field label={t("config.fld_assignee_wake")}>
            {/* The label is the config page's own string, shared rather than
                copied: it is the same setting seen from the other side — a
                per-agent override of the same global switch — and a second
                copy would be a second thing to keep in step. */}
            <select
              value={value.assignee_wake}
              onChange={(e) =>
                update({
                  assignee_wake: e.target.value as ManifestFormState["assignee_wake"],
                })
              }
              className={inputClass}
            >
              <option value="">{t("agents.form.inherit_default")}</option>
              <option value="true">{t("common.yes")}</option>
              <option value="false">{t("common.no")}</option>
            </select>
          </Field>
          <Field label={t("agents.form.pinned_model")}>
            <ModelPicker
              label={t("agents.form.pinned_model")}
              variant="model"
              allowCustom
              value={asModelName(value.pinned_model)}
              onChange={(next) => update({ pinned_model: next.model })}
              onClear={() => update({ pinned_model: "" })}
              models={filteredModels}
              {...pickerCatalog}
            />
          </Field>
          <Field label={t("agents.form.workspace")}>
            <input
              type="text"
              value={value.workspace}
              onChange={(e) => update({ workspace: e.target.value })}
              placeholder={t("agents.form.auto_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.allowed_plugins")}>
            <TagInput
              value={value.allowed_plugins}
              onChange={(next) => update({ allowed_plugins: next })}
              placeholder={t("agents.form.allowed_plugins_placeholder")}
            />
          </Field>
        </div>
          <div className="flex flex-wrap gap-4 pt-2">
          <Toggle
            label={t("agents.form.skills_disabled")}
            checked={value.skills_disabled}
            onChange={(checked) => update({ skills_disabled: checked })}
          />
          <Toggle
            label={t("agents.form.tools_disabled")}
            checked={value.tools_disabled}
            onChange={(checked) => update({ tools_disabled: checked })}
          />
          <Toggle
            label={t("agents.form.inherit_parent_context")}
            checked={value.inherit_parent_context}
            onChange={(checked) => update({ inherit_parent_context: checked })}
          />
          <Toggle
            label={t("agents.form.generate_identity_files")}
            checked={value.generate_identity_files}
            onChange={(checked) => update({ generate_identity_files: checked })}
          />
          {/* The hint sits beside the toggle rather than inside it: `Toggle`
              renders one inline row, and teaching it to wrap would move the
              five toggles above that do not have one. */}
          <div>
            <Toggle
              label={t("agents.form.show_progress")}
              checked={value.show_progress}
              onChange={(checked) => update({ show_progress: checked })}
            />
            <p className="text-[10px] text-text-dim/70 mt-0.5 ml-6">
              {t("agents.form.show_progress_hint")}
            </p>
          </div>
          <div>
            <Toggle
              label={t("agents.form.cache_context")}
              checked={value.cache_context}
              onChange={(checked) => update({ cache_context: checked })}
            />
            <p className="text-[10px] text-text-dim/70 mt-0.5 ml-6">
              {t("agents.form.cache_context_hint")}
            </p>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-3 mt-2">
          <StepLadderInput
            label={t("agents.form.max_history_messages")}
            value={value.max_history_messages}
            onChange={(next) => update({ max_history_messages: next })}
            ladder={MAX_HISTORY_MESSAGES_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={MIN_HISTORY_MESSAGES}
            invalid={invalidFields.has("max_history_messages")}
            // `min` is not a guard: a pasted `1.5` still reaches the field.
            // The message lives on the ladder — the control that accepted the
            // value — and not on `Field` as well, which would announce it twice.
            error={
              invalidFields.has("max_history_messages")
                ? t("agents.form.whole_number_required")
                : undefined
            }
          />
          <p className="text-[10px] text-text-dim/70 mt-1">
            {t("agents.form.max_history_messages_hint")}
          </p>
          <StepLadderInput
            label={t("agents.form.max_concurrent_invocations")}
            value={value.max_concurrent_invocations}
            onChange={(next) => update({ max_concurrent_invocations: next })}
            ladder={MAX_CONCURRENT_INVOCATIONS_LADDER}
            formatRung={formatCount}
            inheritLabel={t("model_param.inherit")}
            customLabel={t("model_param.custom")}
            min={1}
            invalid={invalidFields.has("max_concurrent_invocations")}
            error={
              invalidFields.has("max_concurrent_invocations")
                ? t("agents.form.whole_number_required")
                : undefined
            }
          />
          <p className="text-[10px] text-text-dim/70 mt-1">
            {t("agents.form.max_concurrent_invocations_hint")}
          </p>
        </div>
        </AdvancedFields>
      </FormSection>

      {/* `exec_policy` is a permission, not a lifecycle setting: it says which
          shell commands this agent may run, and under what limits. It renders
          on a card of its own in the Permissions group rather than folded into
          the lifecycle block, whose session-mode and search switches are not
          permissions. */}
      <FormSection
        id="exec_policy"
        shows={shows}
        title={t("agents.form.exec_policy")}
        defaultOpen={false}
        invalid={
          invalidFields.has("exec_policy.timeout_secs") ||
          invalidFields.has("exec_policy.max_output_bytes") ||
          invalidFields.has("exec_policy.no_output_timeout_secs")
        }
      >
        <p className="text-[10px] text-text-dim/70 mb-2">
          {t("agents.form.exec_policy_hint")}
        </p>
        {/* The whole policy, not just its shorthand: `exec_policy` reads as a
            string or as a table and the form owns both spellings, so the mode
            is still the one-word case and the eight fields beside it are the
            table. */}
      <div className="space-y-2.5">
          <Field label={t("agents.form.exec_policy_mode")}>
            <select
              value={value.exec_policy.mode}
              onChange={(e) =>
                updateExecPolicy({
                  mode: e.target.value as ManifestFormState["exec_policy"]["mode"],
                })
              }
              // The visible label is a `<span>` (Field only draws a real
              // `<label>` when it is given an `htmlFor`), so the control
              // carries its own name. Without it the operator's screen
              // reader announces an unnamed combo box.
              aria-label={t("agents.form.exec_policy_mode")}
              className={inputClass}
            >
              <option value="">{t("agents.form.exec_policy_global")}</option>
              <option value="deny">deny</option>
              <option value="allowlist">allowlist</option>
              <option value="full">full</option>
            </select>
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <Field
                label={t("agents.form.exec_policy_safe_bins")}
                hint={t("agents.form.exec_policy_safe_bins_hint")}
              >
                <TagInput
                  value={value.exec_policy.safe_bins ?? []}
                  onChange={(next) => updateExecPolicy({ safe_bins: next })}
                  placeholder={t("agents.form.exec_policy_safe_bins_placeholder")}
                />
              </Field>
              {/* Only one of the two empties is a statement, and the tag
                  input cannot say which one it is. */}
              {(value.exec_policy.safe_bins ?? []).length === 0 && (
                <TriStateNote
                  absent={value.exec_policy.safe_bins === null}
                  absentText={t("agents.form.exec_policy_safe_bins_default")}
                  emptyText={t("agents.form.exec_policy_safe_bins_empty")}
                  declareEmptyLabel={t("agents.form.exec_policy_safe_bins_declare_empty")}
                  restoreDefaultLabel={t("agents.form.exec_policy_safe_bins_use_default")}
                  onDeclareEmpty={() => updateExecPolicy({ safe_bins: [] })}
                  onRestoreDefault={() => updateExecPolicy({ safe_bins: null })}
                />
              )}
            </div>
            <Field
              label={t("agents.form.exec_policy_allowed_commands")}
              hint={t("agents.form.exec_policy_allowed_commands_hint")}
            >
              <TagInput
                value={value.exec_policy.allowed_commands}
                onChange={(next) => updateExecPolicy({ allowed_commands: next })}
                placeholder={t("agents.form.exec_policy_allowed_commands_placeholder")}
              />
            </Field>
            <Field
              label={t("agents.form.exec_policy_allowed_env_vars")}
              hint={t("agents.form.exec_policy_allowed_env_vars_hint")}
            >
              <TagInput
                value={value.exec_policy.allowed_env_vars}
                onChange={(next) => updateExecPolicy({ allowed_env_vars: next })}
                placeholder={t("agents.form.exec_policy_allowed_env_vars_placeholder")}
              />
            </Field>
            <div className="space-y-2.5">
              <Toggle
                label={t("agents.form.exec_policy_safe_bins_skip")}
                checked={value.exec_policy.safe_bins_skip_approval}
                onChange={(checked) => updateExecPolicy({ safe_bins_skip_approval: checked })}
              />
              <p className="text-[10px] text-text-dim/70">
                {t("agents.form.exec_policy_safe_bins_skip_hint")}
              </p>
              <Toggle
                label={t("agents.form.exec_policy_full_skips_approval")}
                checked={value.exec_policy.full_mode_skips_approval}
                onChange={(checked) => updateExecPolicy({ full_mode_skips_approval: checked })}
              />
              <p className="text-[10px] text-text-dim/70">
                {t("agents.form.exec_policy_full_skips_approval_hint")}
              </p>
            </div>
          </div>
          <div className="grid grid-cols-3 gap-3">
            <StepLadderInput
              label={t("agents.form.exec_policy_timeout")}
              value={value.exec_policy.timeout_secs}
              onChange={(next) => updateExecPolicy({ timeout_secs: next })}
              ladder={EXEC_TIMEOUT_LADDER}
              formatRung={formatSeconds}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              min={1}
            />
            <StepLadderInput
              label={t("agents.form.exec_policy_no_output_timeout")}
              value={value.exec_policy.no_output_timeout_secs}
              onChange={(next) => updateExecPolicy({ no_output_timeout_secs: next })}
              ladder={EXEC_NO_OUTPUT_TIMEOUT_LADDER}
              formatRung={formatSeconds}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              min={0}
            />
            <StepLadderInput
              label={t("agents.form.exec_policy_max_output")}
              value={value.exec_policy.max_output_bytes}
              onChange={(next) => updateExecPolicy({ max_output_bytes: next })}
              ladder={EXEC_OUTPUT_BYTES_LADDER}
              formatRung={formatBytes}
              inheritLabel={t("model_param.inherit")}
              customLabel={t("model_param.custom")}
              min={1}
            />
          </div>
        </div>
      </FormSection>

      <FormSection id="shared_folders" shows={shows}
        title={t("agents.form.shared_folders")}
        defaultOpen={false}
        invalid={value.workspaces.some(
          (ws) =>
            invalidFields.has(`workspaces.${ws._uid}.name`) ||
            invalidFields.has(`workspaces.${ws._uid}.path`),
        )}
      >
        <p className="text-[10px] text-text-dim/70 mb-2">
          {t("agents.form.shared_folders_hint")}
        </p>
        {value.workspaces.map((ws, idx) => {
          const nameInvalid = invalidFields.has(`workspaces.${ws._uid}.name`);
          const pathInvalid = invalidFields.has(`workspaces.${ws._uid}.path`);
          return (
            <div
              key={ws._uid}
              className="rounded-lg border border-border-subtle/60 bg-main/40 p-2 mb-2"
            >
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={ws.name}
                  onChange={(e) => update({ workspaces: patchListItem(value.workspaces, idx, { ...ws, name: e.target.value }) })}
                  placeholder={t("agents.form.folder_name")}
                  className={`${inputClass} flex-1`}
                  aria-invalid={nameInvalid || undefined}
                />
                <input
                  type="text"
                  value={ws.path}
                  onChange={(e) => update({ workspaces: patchListItem(value.workspaces, idx, { ...ws, path: e.target.value }) })}
                  placeholder={t("agents.form.folder_path")}
                  className={`${inputClass} flex-[2]`}
                  aria-invalid={pathInvalid || undefined}
                />
                <select
                  value={ws.mode}
                  onChange={(e) => update({ workspaces: patchListItem(value.workspaces, idx, { ...ws, mode: e.target.value as "rw" | "r" }) })}
                  className={`${inputClass} w-20`}
                >
                  <option value="rw">rw</option>
                  <option value="r">r</option>
                </select>
                <button
                  type="button"
                  onClick={() => update({ workspaces: value.workspaces.filter((_, i) => i !== idx) })}
                  className="text-text-dim hover:text-error"
                  aria-label={t("agents.form.remove_folder")}
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
              {nameInvalid && (
                <p className="text-[10px] text-error mt-1">
                  {t("agents.form.duplicate_folder_name")}
                </p>
              )}
              {pathInvalid && (
                <p className="text-[10px] text-error mt-1">
                  {t("agents.form.folder_path_invalid")}
                </p>
              )}
            </div>
          );
        })}
        <button
          type="button"
          onClick={() =>
            update({
              workspaces: [
                ...value.workspaces,
                { _uid: generateUid(), name: "", path: "", mode: "rw" },
              ],
            })
          }
          className="flex items-center gap-1 text-xs text-brand hover:underline"
        >
          <Plus className="w-3.5 h-3.5" />
          {t("agents.form.add_folder")}
        </button>
      </FormSection>
    </ManifestSections>
    </div>
    </SectionCountsContext.Provider>
    </AdvancedModeContext.Provider>
  );
}

const inputClass =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

const textareaClass = `${inputClass} resize-y font-mono text-xs`;

/** Patch a single item in an immutable list. Pass an object to replace, or a function to transform. */
function patchListItem<T>(list: T[], idx: number, patch: T | ((item: T) => T)): T[] {
  const next = list.slice();
  next[idx] = typeof patch === "function" ? (patch as (item: T) => T)(next[idx]) : patch;
  return next;
}

/**
 * Build the options + description map for the skill/tool finders
 * (#5049). The catalog is union-ed with the currently-selected values
 * so chips for unknown entries stay rendered even before the catalog
 * loads. Returns `null` when no catalog is available so the caller can
 * fall back to a plain tag input.
 *
 * The returned `options` list is sorted for stable rendering order.
 */
/** One assigned skill's declared needs, and the ones the agent does not grant. */
export interface SkillNeedGap {
  name: string;
  needs: string[];
  missing: string[];
}

/**
 * What each assigned skill declares it needs, and which of those the agent's
 * grants do not cover.
 *
 * A skill carries its `[requirements] tools` with it and nothing checks them
 * against the agent it lands on — the check exists but only for hands
 * (`hands_lifecycle`'s `check_requirements`) — so a skill can stay assigned
 * while being unable to run, with no sign that anything is wrong.
 *
 * Three rules keep the answer from ever overstating what it knows:
 *
 * - Only skills the catalog vouches for are reported. An entry the catalog does
 *   not carry has unknown needs, and unknown must not become a notice.
 * - `capabilities.tools` is read the way the kernel reads it
 *   (`isToolAdmittedByCapabilities`): empty means *unrestricted*, so it can
 *   never produce a gap.
 * - `toolsDisabled` is the second way a tool goes missing and the easier one to
 *   overlook: the kernel hands such an agent an empty tool set before any
 *   allowlist is consulted (`available_tools`), so an empty
 *   `capabilities.tools` — which otherwise grants everything — grants nothing
 *   here. Without that branch the warning would be silent in exactly the case
 *   where no skill can run at all.
 *
 * A skill that declares nothing is omitted rather than reported with an empty
 * `needs`: "declares no needs" and "needs nothing" are the same fact, and
 * listing every skill with a blank line would bury the ones that do declare.
 */
export function skillNeedsAgainstGrants(
  skills: readonly string[],
  catalog: readonly ManifestCatalogEntry[] | undefined,
  declaredTools: readonly string[],
  toolsDisabled: boolean,
): SkillNeedGap[] {
  const byName = new Map((catalog ?? []).map((s) => [s.name, s]));
  return skills.flatMap((name) => {
    const needs = byName.get(name)?.required_tools ?? [];
    if (needs.length === 0) return [];
    const missing = toolsDisabled
      ? needs
      : needs.filter((tool) => !isToolAdmittedByCapabilities(tool, declaredTools));
    return [{ name, needs, missing }];
  });
}

function mergeCatalog(
  catalog: ManifestCatalogEntry[] | undefined,
  selected: string[],
): { options: string[]; meta: Record<string, { description?: string }> } | null {
  if (!catalog) return null;
  const meta: Record<string, { description?: string }> = {};
  const seen = new Set<string>();
  for (const entry of catalog) {
    if (!entry?.name || seen.has(entry.name)) continue;
    seen.add(entry.name);
    if (entry.description) meta[entry.name] = { description: entry.description };
  }
  for (const name of selected) {
    if (!name || seen.has(name)) continue;
    seen.add(name);
  }
  const options = Array.from(seen).sort((a, b) => a.localeCompare(b));
  return { options, meta };
}

/**
 * The form's section host, which is also where the caller's order wins.
 *
 * The children come in canonical order (the order `MANIFEST_SECTION_IDS`
 * declares, which is the order the JSX is written in); `order` is the caller's
 * list, already deduplicated and filtered by `AgentManifestForm`. Emitting the
 * elements in `order` is what turns `sections` from a filter into a layout
 * instruction. Each element carries its own id (`Section.id` /
 * `FormSection.id`), and that id is also its React key, so reordering moves
 * the existing DOM instead of remounting it — an open fold stays open, and the
 * inputs keep their state, when a group's list says the sections belong in a
 * different sequence.
 */
function ManifestSections({
  order,
  className,
  children,
}: {
  order: ManifestSectionId[];
  className?: string;
  children: ReactNode;
}) {
  const byId = new Map<string, ReactNode>();
  Children.forEach(children, (child) => {
    if (!isValidElement(child)) return;
    const id = (child.props as { id?: unknown }).id;
    if (typeof id === "string" && !byId.has(id)) byId.set(id, child);
  });

  return (
    <div className={className}>
      {order.map((id) => (
        <Fragment key={id}>{byId.get(id) ?? null}</Fragment>
      ))}
    </div>
  );
}

/**
 * Always-open sibling of `CollapsibleSection`, for the handful of sections
 * that carry the fields an operator edits most (identity, model, limits,
 * grants). Those stay expanded because folding the common case behind a click
 * trades no scroll for a click on every visit.
 *
 * `when` is the same render guard `FormSection` applies — a section this
 * caller did not ask for renders nothing, not an empty frame.
 *
 * An always-open section carries the same field-count badge as the folded
 * ones, measured from its own subtree: a section that happens not to fold
 * should not be the only one the operator cannot size up.
 */
function Section({
  title,
  when = true,
  id,
  children,
}: {
  title: string;
  when?: boolean;
  /** Section identity, emitted as `data-section`. See `CollapsibleSectionProps.sectionId`. */
  id?: string;
  children: React.ReactNode;
}) {
  // Hooks before the `when` return, as React requires. A not-rendered section
  // measures nothing, and `useSectionCount` withdraws it from the total.
  const rootRef = useRef<HTMLDivElement>(null);
  const count = useSectionFieldCount(rootRef, when);
  useSectionCount(id, count, when);
  if (!when) return null;
  return (
    <div
      ref={rootRef}
      data-section={id}
      className="space-y-2.5 rounded-xl border border-border-subtle/60 bg-surface/40 p-3"
    >
      <p className="flex items-center gap-2">
        <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
          {title}
        </span>
        <FieldCountBadge count={count} />
      </p>
      {children}
    </div>
  );
}

/**
 * The rest of a section — the fields beyond the ones an operator sets first.
 *
 * The unified editor's contract, in the user's own words: "todo en el mismo
 * sitio, con un botón de ADVANCED para dar más profundidad a cada sección, y
 * en el modo BASIC que se vea lo primordial de cada sección." A section that
 * opts in renders its essential fields always and folds everything else
 * behind this disclosure. Native details/summary — the same mechanism
 * `CollapsibleSection` uses for the section itself — so the keyboard and
 * toggle behaviour come free, and the fold survives every re-render because
 * there is no per-group state to keep in sync.
 *
 * `invalid` forces the group open: a validation error the operator cannot
 * see is indistinguishable from no error at all. One level down, the same
 * rule the section fold itself applies.
 *
 * `advanced` comes from the form-level switch by context rather than by prop:
 * there are fifteen call sites, and a prop would be fifteen chances to forget
 * one — with the failure invisible, since a forgotten site simply folds in
 * advanced mode where the operator expects everything open.
 */
const AdvancedModeContext = createContext(false);

/**
 * Where a mounted section publishes how many fields it holds, so the form can
 * show the sum once above the section list. By context rather than by prop for
 * the same reason as `AdvancedModeContext`: the sections are written at
 * nineteen call sites, and a prop is nineteen chances to forget one — here the
 * failure is a total that silently undercounts.
 *
 * `report` carries the count, `withdraw` takes it back when the section
 * unmounts (a config-group switch unmounts the sections of the group being
 * left, and their fields must leave the total with them).
 */
interface SectionCountRegistry {
  report: (sectionId: string, count: number) => void;
  withdraw: (sectionId: string) => void;
}

const SectionCountsContext = createContext<SectionCountRegistry | null>(null);

/**
 * Publishes `count` under `sectionId` while `mounted`, and withdraws it
 * otherwise. The count is the section's own badge measurement
 * (`useSectionFieldCount`); this only carries it one level up, so the badge
 * and the total can never disagree.
 */
function useSectionCount(sectionId: string | undefined, count: number, mounted: boolean): void {
  const registry = useContext(SectionCountsContext);
  useEffect(() => {
    if (!registry || !sectionId || !mounted) return;
    registry.report(sectionId, count);
    return () => registry.withdraw(sectionId);
  }, [registry, sectionId, mounted, count]);
}

export function AdvancedFields({
  children,
  invalid,
}: {
  children: React.ReactNode;
  invalid?: boolean;
}) {
  const { t } = useTranslation();
  const advanced = useContext(AdvancedModeContext);
  return (
    <details
      data-advanced
      className="group rounded-lg border border-border-subtle/40 bg-main/30"
      open={advanced || invalid}
    >
      <summary className="flex cursor-pointer list-none items-center justify-between px-2.5 py-1.5 select-none">
        <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim group-open:text-text-main">
          {t("agents.form.advanced")}
        </span>
        <ChevronDown className="h-3.5 w-3.5 text-text-dim transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-2.5 px-2.5 pb-2.5 pt-1">{children}</div>
    </details>
  );
}

/**
 * The controls a folded section's badge tallies: what an operator can type
 * into, choose from or toggle, and nothing else.
 *
 * Bulk buttons are excluded — "add folder" is an affordance, not a field —
 * and hidden inputs are excluded because they carry plumbing rather than
 * anything the section reveals when it opens. But a field is not always an
 * input: the model picker and the quantity ladders are button sets, so they
 * opt in with `data-field` and each counts as the one field it is.
 * `data-no-field` is the opposite opt-out, for the controls a picker renders
 * inside its own popover (search, hand-entry): opening a picker is the
 * operator editing a field, not the section revealing another one.
 *
 * `querySelectorAll` returns an element once even when it matches several
 * arms of this list, so a marked control that were also an input could not be
 * counted twice. None is: `data-field` sits on a button or a `role="group"`,
 * and any input inside them carries `data-no-field`.
 */
const SECTION_FIELD_SELECTOR = [
  'input:not([type="hidden"]):not([data-no-field])',
  'select:not([data-no-field])',
  'textarea:not([data-no-field])',
  '[role="switch"]:not([data-no-field])',
  "[data-field]",
].join(", ");

/**
 * How many fields the section rooted at `ref` currently holds.
 *
 * Measured from the DOM rather than counted from the form state: which fields
 * a section shows is decided by the same conditionals that render it
 * (`enabled && …`, a `.map` over rows), and a state-side count would have to
 * restate every one of them and would drift the moment one changed. A closed
 * `<details>` keeps its children in the DOM, so the inner "Advanced" fold is
 * counted exactly like the fields around it — which is the point, since the
 * badge advertises what opening the section reveals.
 *
 * The observer keeps the badge honest as fields appear and disappear under it
 * (a toggle enabling a block, a workspace row being added), and the initial
 * measurement runs before it is attached. Both are torn down with the section.
 */
function useSectionFieldCount(
  ref: RefObject<HTMLElement | null>,
  enabled: boolean,
): number {
  const [count, setCount] = useState(0);
  // The figure already reported. MutationObserver callbacks are delivered
  // after the commit that caused them, and a callback that finds the same
  // number must not schedule a render for it — that render is invisible, and
  // in a test it is an update outside `act`.
  const reported = useRef(0);

  useEffect(() => {
    const node = ref.current;
    const report = (next: number): void => {
      if (reported.current === next) return;
      reported.current = next;
      setCount(next);
    };
    if (!enabled || !node) {
      report(0);
      return;
    }
    const measure = (): void =>
      report(node.querySelectorAll(SECTION_FIELD_SELECTOR).length);
    measure();
    const observer = new MutationObserver(measure);
    observer.observe(node, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["type", "role"],
    });
    return () => observer.disconnect();
  }, [enabled, ref]);

  return count;
}

/**
 * Folding a section is this caller's choice, so the guard lives here rather than being repeated at every call site.
 * An unshown section renders nothing at all rather than a collapsed shell: a heading the operator cannot open is still a heading they will look for.
 * `shows` stays a prop so the caller keeps one definition of what "shown" means.
 *
 * Declared at module scope on purpose.
 * A component declared inside the render body is a new function on every render, and React compares element types by reference — so it unmounts and remounts its whole subtree on every keystroke.
 * Here that subtree is a form section, so every controlled input inside the twelve sections using this wrapper kept only the first character typed into it: the element the second keystroke was headed for had already been destroyed, and the change event went to a detached node.
 * The seven sections that use the module-scope `Section` never had it, which is what made a wrapper-wide defect read as a per-field oddity.
 */
function FormSection({
  id,
  shows,
  ...props
}: {
  id: ManifestSectionId;
  shows: (id: ManifestSectionId) => boolean;
} & CollapsibleSectionProps) {
  // The whole section opens in advanced mode too, not only its inner folds:
  // "advanced shows everything" has to mean the fold the operator would
  // otherwise click, or the switch leaves most of the group still closed.
  const advanced = useContext(AdvancedModeContext);
  // The badge is this section's own count of the fields behind its fold — the
  // inner Advanced fold included, since those fields are in the DOM too.
  const rootRef = useRef<HTMLDetailsElement>(null);
  const shown = shows(id);
  const count = useSectionFieldCount(rootRef, shown);
  // The same number feeds the form-level total, so the two cannot drift.
  useSectionCount(id, count, shown);
  return shown ? (
    <CollapsibleSection
      sectionId={id}
      {...props}
      rootRef={rootRef}
      count={count}
      defaultOpen={props.defaultOpen || advanced}
    />
  ) : null;
}

/**
 * Inherit / yes / no — the three states an `Option<bool>` manifest key can be in.
 * Five of the seven keys in `[proactive_memory]` are `Option<bool>` and they are the same control three times over; one helper rather than five copies means the absent option cannot be dropped from one of them.
 *
 * Module scope for the same reason as `FormSection` above.
 * This was also the control whose open `<select>` was destroyed under the operator on every re-render — the symptom that led to finding the wrapper.
 */
function TriStateField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: "" | "true" | "false";
  onChange: (next: "" | "true" | "false") => void;
}) {
  const { t } = useTranslation();
  return (
    <Field label={label}>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value as "" | "true" | "false")}
        className={inputClass}
      >
        <option value="">{t("agents.form.inherit_default")}</option>
        <option value="true">{t("common.yes")}</option>
        <option value="false">{t("common.no")}</option>
      </select>
    </Field>
  );
}

/**
 * The note that makes an empty tri-state list say which empty it is.
 *
 * `null` is the absent key — for `safe_bins` the daemon's built-in list, for
 * `plugin_registries` the official registry — while `[]` is a declared empty
 * list, which is a different statement and sometimes a security-relevant one.
 * An empty tag input cannot carry the difference, so the switch is explicit
 * rather than inferred from "the operator deleted every chip".
 *
 * Module scope for the same reason as `FormSection`.
 */
function TriStateNote({
  absent,
  absentText,
  emptyText,
  declareEmptyLabel,
  restoreDefaultLabel,
  onDeclareEmpty,
  onRestoreDefault,
}: {
  /** `true` while the key is absent, which is not the same as declared empty. */
  absent: boolean;
  absentText: string;
  emptyText: string;
  declareEmptyLabel: string;
  restoreDefaultLabel: string;
  onDeclareEmpty: () => void;
  onRestoreDefault: () => void;
}) {
  return (
    <p className={`mt-1 text-[10px] ${absent ? "text-text-dim/70" : "text-warning"}`}>
      {absent ? absentText : emptyText}{" "}
      <button
        type="button"
        className="underline hover:text-brand"
        onClick={absent ? onDeclareEmpty : onRestoreDefault}
      >
        {absent ? declareEmptyLabel : restoreDefaultLabel}
      </button>
    </p>
  );
}

/**
 * A select for a `String` field whose accepted values are a known list.
 *
 * Not the closed-enum treatment `asEnum` gives a Rust enum: `engine` and
 * `hooks.runtime` are `String`s, so a value outside the list is one the daemon
 * accepts and runs with. The current value is appended to the options when it
 * is not in the list, which is what stops the editor from silently rewriting a
 * manifest it did not understand into one it does.
 *
 * Module scope for the same reason as `FormSection`.
 */
function OpenStringSelect({
  value,
  options,
  inheritLabel,
  ariaLabel,
  onChange,
}: {
  value: string;
  options: readonly string[];
  inheritLabel: string;
  ariaLabel?: string;
  onChange: (next: string) => void;
}) {
  const known = value === "" || options.includes(value);
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      aria-label={ariaLabel}
      className={inputClass}
    >
      <option value="">{inheritLabel}</option>
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
      {!known ? <option value={value}>{value}</option> : null}
    </select>
  );
}

/**
 * One hook script path.
 *
 * Nine of these, one per lifecycle hook, and they are the same control nine
 * times over: a labelled box whose placeholder is the shape of a path.
 */
function HookScriptField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (next: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Field label={label}>
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={t("agents.form.context_engine_hook_placeholder")}
        className={inputClass}
      />
    </Field>
  );
}

function ExtrasOverrideHint({ message }: { message: string }) {
  return (
    <div className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/5 px-2.5 py-1.5 text-[11px] text-warning">
      <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
      <span>{message}</span>
    </div>
  );
}

function Toggle({
  label,
  ariaLabel,
  checked,
  onChange,
}: {
  label: string;
  ariaLabel?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-xs cursor-pointer select-none">
      <input
        type="checkbox"
        checked={checked}
        aria-label={ariaLabel}
        onChange={(e) => onChange(e.target.checked)}
        className="h-4 w-4 rounded border-border-subtle accent-brand"
      />
      {label ? <span>{label}</span> : null}
    </label>
  );
}

/** The four accessible names a `JsonRowEditor` needs, resolved by the caller. */
interface JsonRowLabels {
  key: string;
  type: string;
  value: string;
  remove: string;
}

/**
 * One `key = value` row of a JSON-valued table.
 *
 * Shared by `[metadata]` and `[tools.<name>.params]`: both are
 * `HashMap<String, serde_json::Value>`, so both need the same three scalar
 * arms and the same explicit type choice. One component rather than two copies
 * means the type list cannot drift between them.
 *
 * Module scope for the same reason as `FormSection`: a component declared in
 * the render body is a new function on every render, and React compares
 * element types by reference — so the subtree unmounts on every keystroke and
 * each controlled input keeps only its first character.
 *
 * No visible labels. The row is a grid of near-identical controls, so a label
 * above each one would repeat the same word down the column; the accessible
 * name carries the row number instead, which is what tells a screen reader
 * which of the five "Key" boxes it is in.
 */
function JsonRowEditor({
  row,
  index,
  labels,
  types = JSON_ROW_TYPES,
  onChange,
  onRemove,
}: {
  row: { _uid: string; key: string; valueType: JsonRowType; value: string };
  index: number;
  labels: JsonRowLabels;
  /**
   * The types this table's values may take. `[hooks.env_schema]` is a
   * `HashMap<String, String>`, so it passes one and gets no type column: a
   * control that offers a choice the manifest cannot hold is worse than no
   * control.
   */
  types?: readonly JsonRowType[];
  onChange: (next: { _uid: string; key: string; valueType: JsonRowType; value: string }) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="mb-2 flex items-center gap-2">
      <input
        type="text"
        value={row.key}
        onChange={(e) => onChange({ ...row, key: e.target.value })}
        placeholder={labels.key}
        aria-label={`${labels.key} ${index + 1}`}
        // `flex-1 min-w-0` on both the key and the value: `inputClass` carries
        // `w-full`, and three width-100% children in a flex row collapse the
        // two flexible ones to nothing beside the fixed-width type select.
        className={`${inputClass} flex-1 min-w-0`}
      />
      {types.length > 1 && (
        <select
          value={row.valueType}
          onChange={(e) => onChange({ ...row, valueType: e.target.value as JsonRowType })}
          aria-label={`${labels.type} ${index + 1}`}
          // `basis-28` rather than `w-28`: `inputClass` carries `w-full`, and
          // two conflicting width utilities are resolved by their order in the
          // generated stylesheet — which put `w-full` last and gave this
          // select the whole row, collapsing the two inputs beside it to a
          // sliver. `flex-basis` is a different property, so it wins outright.
          className={`${inputClass} basis-28 grow-0 shrink-0`}
        >
          {types.map((type) => (
            <option key={type} value={type}>
              {t(`agents.form.json_type_${type}`)}
            </option>
          ))}
        </select>
      )}
      {row.valueType === "boolean" ? (
        // A boolean row cannot hold free text: `yes`, `1` and `TRUE` are all
        // things an operator types and none of them is a TOML boolean.
        <select
          value={row.value}
          onChange={(e) => onChange({ ...row, value: e.target.value })}
          aria-label={`${labels.value} ${index + 1}`}
          className={`${inputClass} flex-1 min-w-0`}
        >
          <option value="true">{t("common.yes")}</option>
          <option value="false">{t("common.no")}</option>
        </select>
      ) : (
        <input
          type="text"
          value={row.value}
          onChange={(e) => onChange({ ...row, value: e.target.value })}
          placeholder={labels.value}
          aria-label={`${labels.value} ${index + 1}`}
          className={`${inputClass} flex-1 min-w-0`}
        />
      )}
      <button
        type="button"
        onClick={onRemove}
        className="shrink-0 text-text-dim hover:text-error"
        aria-label={`${labels.remove} ${index + 1}`}
      >
        <Trash2 className="w-3.5 h-3.5" />
      </button>
    </div>
  );
}

/**
 * The half of a JSON-valued table a row cannot hold: values that are tables or
 * arrays, shown as the file holds them.
 *
 * Rendered rather than hidden on purpose — a value the operator cannot see is
 * indistinguishable from one the editor ate.
 */
function PreservedValues({
  entries,
  hint,
}: {
  entries: Record<string, unknown>;
  hint: string;
}) {
  const keys = Object.keys(entries);
  if (!keys.length) return null;
  return (
    <div className="mb-2">
      {keys.map((key) => (
        <p key={key} className="font-mono text-[11px] text-text-dim">
          {key} = {formatPreservedValue(entries[key])}
        </p>
      ))}
      <p className="text-[10px] text-text-dim/70">{hint}</p>
    </div>
  );
}

function TagInput({
  value,
  onChange,
  placeholder,
}: {
  value: string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
}) {
  const [inputValue, setInputValue] = useState("");

  const commit = (raw: string): void => {
    const cleaned = raw.trim();
    if (!cleaned) return;
    setInputValue("");
    if (value.includes(cleaned)) return;
    onChange([...value, cleaned]);
  };
  return (
    <div className="flex flex-wrap items-center gap-1.5 rounded-lg border border-border-subtle bg-main px-2 py-1.5 focus-within:border-brand">
      {value.map((tag) => (
        <span
          key={tag}
          className="inline-flex items-center gap-1 rounded-md bg-surface px-1.5 py-0.5 text-[11px] font-medium text-text"
        >
          {tag}
          <button
            type="button"
            onClick={() => onChange(value.filter((t) => t !== tag))}
            className="text-text-dim hover:text-error"
            aria-label={`remove ${tag}`}
          >
            <X className="h-3 w-3" />
          </button>
        </span>
      ))}
      <input
        type="text"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        placeholder={value.length === 0 ? placeholder : undefined}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            commit(inputValue);
          } else if (e.key === "Backspace" && !inputValue && value.length > 0) {
            onChange(value.slice(0, -1));
          }
        }}
        onBlur={() => {
          if (inputValue) {
            commit(inputValue);
          }
        }}
        className="flex-1 min-w-[100px] bg-transparent text-xs outline-none placeholder:text-text-dim/40"
      />
    </div>
  );
}
