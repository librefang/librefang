import { createContext, useContext, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ChevronDown, Plus, RotateCcw, Trash2, X } from "lucide-react";
import {
  AUTO_ROUTE_STRATEGIES,
  CAPABILITY_ROUTING_KEYS,
  DM_POLICIES,
  GROUP_POLICIES,
  OUTPUT_FORMATS,
  PREFIX_STYLES,
  TYPING_MODES,
  USAGE_FOOTERS,
  generateUid,
} from "../lib/agentManifest";
import type { ManifestExtras, ManifestFormState } from "../lib/agentManifest";

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
import { MultiSelectCmdk } from "./ui/MultiSelectCmdk";
import { ModelParamField } from "./ui/ModelParamField";
import { CollapsibleSection } from "./ui/CollapsibleSection";
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
  "model",
  "prompt",
  "limits",
  "capabilities",
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
  "async_tasks",
  "routing",
  "context_injection",
  "response_format",
  "lifecycle",
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
  [/^model\./, "model"],
  [/^schedule\./, "scheduling"],
  [/^response_format\./, "response_format"],
  [/^workspaces\./, "shared_folders"],
  [/^autonomous\./, "autonomous"],
  [/^compaction\./, "compaction"],
  [/^skill_workshop\./, "skill_workshop"],
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
  invalidFields,
  extras,
  skillCatalog,
  toolCatalog,
  mcpCatalog,
  routerProfileCatalog,
  routerProfilesEnabled,
  nameField = "editable",
  sections,
  advanced = false,
}: AgentManifestFormProps) {
  const { t } = useTranslation();

  // `undefined` means "every section" (the create modal). A caller that
  // passes a list gets exactly that list.
  const shows = (id: ManifestSectionId): boolean =>
    sections === undefined || sections.includes(id);

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

  // The picker wants `{ id }` rather than `{ name }`. Memoised because it is
  // passed to every fallback row, and a fresh array each render would defeat
  // the picker's own memoisation of its provider list.
  const providerPickerList = useMemo(
    () => providerOptions.map((p) => ({ id: p.name })),
    [providerOptions],
  );

  // Build {options, meta} pairs for the skill/tool finders (#5049).
  // The catalog is union-ed with the user's current selection so
  // entries the registry doesn't know about (e.g. a skill the user
  // typed in by hand, or one that is staged but not yet installed)
  // remain visible as chips and selectable in the dropdown.
  const skillFinder = useMemo(
    () => mergeCatalog(skillCatalog, value.skills),
    [skillCatalog, value.skills],
  );
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
    <div className="space-y-4">
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
      </Section>

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
          The profile router's per-agent settings. They are manifest fields,
          and this form is their only editor: the routing panel that shared
          them died here — two writers to the same five values, and a form
          save serialized its seeded state verbatim over anything the panel
          had written, both surfaces toasting success. The profile allowlist
          keeps the panel's server-backed catalog; the fixed-mode constraints
          are preserved rather than cleared, as the manifest file format
          allows.
        */}
        <p className="text-[11px] text-text-dim">{t("agents.form.router_hint")}</p>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.router_mode")} hint={t("agents.form.router_mode_hint")}>
            {/* A closed two-state enum with a `#[default]` variant, so unlike
                the tri-state selects below there is no inherit option: the
                state always names a mode, and `fixed` is simply not written. */}
            <select
              value={value.model.mode}
              onChange={(e) =>
                updateModel({ mode: e.target.value as ManifestFormState["model"]["mode"] })
              }
              className={inputClass}
            >
              <option value="fixed">{t("agents.form.router_mode_fixed")}</option>
              <option value="flexible">{t("agents.form.router_mode_flexible")}</option>
            </select>
          </Field>
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
        </div>
        <Toggle
          label={t("agents.form.router_fixed")}
          checked={value.model.router_fixed}
          onChange={(checked) => updateModel({ router_fixed: checked })}
        />
        {routerProfilesEnabled === false && (
          <p className="text-[11px] text-text-dim">
            {t("agents.form.router_kernel_off")}
          </p>
        )}
        <div className="grid grid-cols-2 gap-3">
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

      <Section when={shows("skills")} id="skills" title={t("agents.form.skills")}>
        <Field hint={t("agents.form.skills_hint")} ariaLabel={t("agents.form.skills")}>
          {skillFinder ? (
            <MultiSelectCmdk
              options={skillFinder.options}
              optionMeta={skillFinder.meta}
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
                  customLabel={t("model_param.custom")} min={0} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_rate_limit_per_user")} value={value.channel_overrides.rate_limit_per_user}
                  onChange={(next) => updateChannelOverrides({ rate_limit_per_user: next })} ladder={CHANNEL_RATE_LIMIT_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0} />
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
                  customLabel={t("model_param.custom")} min={0} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_message_debounce_max_ms")} value={value.channel_overrides.message_debounce_max_ms}
                  onChange={(next) => updateChannelOverrides({ message_debounce_max_ms: next })} ladder={CHANNEL_DEBOUNCE_MAX_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_message_debounce_max_buffer")} value={value.channel_overrides.message_debounce_max_buffer}
                  onChange={(next) => updateChannelOverrides({ message_debounce_max_buffer: next })} ladder={CHANNEL_DEBOUNCE_BUFFER_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0} />
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
                  customLabel={t("model_param.custom")} min={0} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_auto_route_confidence_threshold")} value={value.channel_overrides.auto_route_confidence_threshold}
                  onChange={(next) => updateChannelOverrides({ auto_route_confidence_threshold: next })} ladder={CHANNEL_ROUTE_CONFIDENCE_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_auto_route_sticky_bonus")} value={value.channel_overrides.auto_route_sticky_bonus}
                  onChange={(next) => updateChannelOverrides({ auto_route_sticky_bonus: next })} ladder={CHANNEL_ROUTE_BONUS_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0} />
              <div className="col-span-2"></div>
                <StepLadderInput label={t("agents.form.channel_overrides_auto_route_divergence_count")} value={value.channel_overrides.auto_route_divergence_count}
                  onChange={(next) => updateChannelOverrides({ auto_route_divergence_count: next })} ladder={CHANNEL_ROUTE_DIVERGENCE_LADDER}
                  formatRung={formatCount} inheritLabel={t("model_param.inherit")}
                  customLabel={t("model_param.custom")} min={0} />
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
                  customLabel={t("model_param.custom")} min={0} />
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
        invalid={invalidFields.has("skill_workshop.max_pending_age_days")}
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
        <Toggle
          label={t("agents.form.routing_enabled")}
          checked={value.routing.enabled}
          onChange={(checked) => updateRouting({ enabled: checked })}
        />
        {value.routing.enabled && (
          <div className="space-y-2 mt-2">
            <div className="grid grid-cols-3 gap-3">
              <Field label={t("agents.form.simple_model")}>
                <ModelPicker
                  label={t("agents.form.simple_model")}
                  variant="model"
                  allowCustom
                  value={asModelName(value.routing.simple_model)}
                  onChange={(next) => updateRouting({ simple_model: next.model })}
                  models={models}
                />
              </Field>
              <Field label={t("agents.form.medium_model")}>
                <ModelPicker
                  label={t("agents.form.medium_model")}
                  variant="model"
                  allowCustom
                  value={asModelName(value.routing.medium_model)}
                  onChange={(next) => updateRouting({ medium_model: next.model })}
                  models={models}
                />
              </Field>
              <Field label={t("agents.form.complex_model")}>
                <ModelPicker
                  label={t("agents.form.complex_model")}
                  variant="model"
                  allowCustom
                  value={asModelName(value.routing.complex_model)}
                  onChange={(next) => updateRouting({ complex_model: next.model })}
                  models={models}
                />
              </Field>
            </div>
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
        {/* BASIC: the router and its three tiers. Orphan reconciliation
            and the tool profile are depth. */}
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
            search switches, exec policy, plugins, the history and concurrency
            caps — folds behind Advanced. */}
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
          <Field
            label={t("agents.form.exec_policy")}
            hint={
              !value.exec_policy_shorthand && extras.topLevel.exec_policy !== undefined
                ? t("agents.form.exec_policy_extras_hint")
                : undefined
            }
          >
            <select
              value={value.exec_policy_shorthand}
              onChange={(e) =>
                update({
                  exec_policy_shorthand:
                    e.target.value as ManifestFormState["exec_policy_shorthand"],
                })
              }
              className={inputClass}
            >
              <option value="">{t("agents.form.exec_policy_global")}</option>
              <option value="allow">allow</option>
              <option value="deny">deny</option>
              <option value="full">full</option>
              <option value="allowlist">allowlist</option>
            </select>
          </Field>
          <Field label={t("agents.form.pinned_model")}>
            <ModelPicker
              label={t("agents.form.pinned_model")}
              variant="model"
              allowCustom
              value={asModelName(value.pinned_model)}
              onChange={(next) => update({ pinned_model: next.model })}
              models={models}
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
    </div>
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
 * Always-open sibling of `CollapsibleSection`, for the handful of sections
 * that carry the fields an operator edits most (identity, model, limits,
 * grants). Those stay expanded because folding the common case behind a click
 * trades no scroll for a click on every visit.
 *
 * `when` is the same render guard `FormSection` applies — a section this
 * caller did not ask for renders nothing, not an empty frame.
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
  if (!when) return null;
  return (
    <div
      data-section={id}
      className="space-y-2.5 rounded-xl border border-border-subtle/60 bg-surface/40 p-3"
    >
      <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{title}</p>
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
  return shows(id) ? (
    <CollapsibleSection
      sectionId={id}
      {...props}
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
