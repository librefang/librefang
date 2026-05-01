import { formatRelativeTime } from "../lib/datetime";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import { tabContent } from "../lib/motion";
import {
  type AgentDetail,
  type AgentItem,
  type CronJobItem,
  type PromptVersion,
  type PromptExperiment,
  type ExperimentVariantMetrics,
  type ToolDefinition,
  getAgentTemplateToml,
  resetAgentSession,
  getAgentTools,
  listTools,
  updateAgentTools,
} from "../api";
import { useQueryClient } from "@tanstack/react-query";
import { isProviderAvailable } from "../lib/status";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { MultiSelectCmdk } from "../components/ui/MultiSelectCmdk";
import { Card } from "../components/ui/Card";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Avatar } from "../components/ui/Avatar";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { filterVisible } from "../lib/hiddenModels";
import { Search, Users, MessageCircle, X, Cpu, Wrench, Shield, Plus, Loader2, Pause, Play, Clock, Brain, Zap, FlaskConical, GitBranch, Trash2, Check, BarChart3, Copy, RotateCcw, Pencil, Bot, Database, FileText, MoreHorizontal, Sparkles } from "lucide-react";
import { truncateId } from "../lib/string";
import { getStatusVariant } from "../lib/status";
import { useDashboardSnapshot } from "../lib/queries/overview";
import { useSessions, useSessionDetails } from "../lib/queries/sessions";
import { useAgentKvMemory } from "../lib/queries/memory";
import { useCronJobs } from "../lib/queries/runtime";
import { useAgentTriggers } from "../lib/queries/schedules";
import { useProviders } from "../lib/queries/providers";
import { useModels } from "../lib/queries/models";
import { AgentManifestForm } from "../components/AgentManifestForm";
import {
  emptyManifestExtras,
  emptyManifestForm,
  parseManifestToml,
  serializeManifestForm,
  validateManifestForm,
  type ManifestExtras,
  type ManifestFormState,
} from "../lib/agentManifest";
import { generateManifestMarkdown } from "../lib/agentManifestMarkdown";
import {
  agentQueries,
  useAgentEvents,
  useAgentSessions,
  useAgentStats,
  useAgentTemplates,
  useExperimentMetrics,
  useExperiments,
  usePromptVersions,
} from "../lib/queries/agents";
import {
  useActivatePromptVersion,
  useCloneAgent,
  useCompleteExperiment,
  useCreateExperiment,
  useCreatePromptVersion,
  useDeleteAgent,
  useDeletePromptVersion,
  usePatchAgent,
  usePatchAgentConfig,
  usePatchHandAgentRuntimeConfig,
  usePauseExperiment,
  useResumeAgent,
  useSpawnAgent,
  useStartExperiment,
  useSuspendAgent,
} from "../lib/mutations/agents";

/**
 * Local view type that pairs the strict `AgentDetail` shape from `api.ts`
 * with the additional runtime fields the backend actually returns on
 * `GET /api/agents/{id}` but which haven't been added to the canonical
 * type yet. Keeping this scoped to AgentsPage avoids widening the
 * exported interface for other consumers and removes the need for
 * `(agent as AgentView).field` casts inside the master-detail rendering.
 */
type AgentTriggerSummary = {
  event_pattern?: string;
  name?: string;
  description?: string;
};
type AgentCronSummary = {
  schedule?: string;
  cron?: string;
  expression?: string;
  next_run?: string;
  name?: string;
  id?: string;
};
type AgentView = AgentDetail & {
  state?: string;
  description?: string;
  profile?: string;
  model_name?: string;
  model_provider?: string;
  last_active?: string;
  triggers?: AgentTriggerSummary[];
  cron_jobs?: AgentCronSummary[];
  // The backend ships capabilities.tools / .skills as string arrays on the
  // wire, but the canonical `AgentDetail` interface narrows them to
  // booleans. Override here so consumers of this page can `.length` them
  // without a cast — the canonical type is fixed in the API layer in a
  // follow-up PR.
  capabilities?: AgentDetail["capabilities"] & {
    skills?: string[];
    tools?: string[];
  };
};

/** Two-column row used inside the detail modal's value cards. */
function DetailRow({ label, children }: { label: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex justify-between items-center gap-3 min-h-[28px]">
      <span className="text-text-dim text-sm">{label}</span>
      <span className="text-sm text-right min-w-0">{children}</span>
    </div>
  );
}

/** Collapsible system-prompt card. Long prompts (>6 lines or >400 chars)
 *  start collapsed with an expand toggle; short prompts render as-is. */
function SystemPromptSection({ prompt }: { prompt: string }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const isLong = prompt.split("\n").length > 6 || prompt.length > 400;
  return (
    <section>
      <div className="flex items-center justify-between mb-2">
        <h4 className="text-sm font-semibold">{t("agents.system_prompt")}</h4>
        {isLong && (
          <button
            onClick={() => setExpanded(v => !v)}
            className="text-xs text-brand hover:underline font-medium"
          >
            {expanded
              ? t("common.collapse", { defaultValue: "Collapse" })
              : t("common.expand", { defaultValue: "Expand" })}
          </button>
        )}
      </div>
      <div className="relative">
        <div
          className={`rounded-lg bg-main border border-border-subtle p-4 text-sm text-text leading-relaxed whitespace-pre-wrap ${
            isLong && !expanded ? "max-h-40 overflow-hidden" : ""
          }`}
        >
          {prompt}
        </div>
        {isLong && !expanded && (
          // Fade-out at the bottom so the cut feels intentional rather than
          // a clip, without introducing an inner scroll. The modal's outer
          // scroll is the single source of truth — no nested scrolling.
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-12 rounded-b-lg bg-linear-to-t from-main to-transparent" />
        )}
      </div>
    </section>
  );
}

export function AgentsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [detailAgent, setDetailAgent] = useState<AgentDetail | null>(null);
  const [, setDetailLoading] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [createMode, setCreateMode] = useState<"form" | "template" | "toml">("form");
  const [templateName, setTemplateName] = useState("");
  const [templateCustomName, setTemplateCustomName] = useState("");
  const [manifestToml, setManifestToml] = useState("");
  const [templateTomlLoading, setTemplateTomlLoading] = useState(false);
  const [formState, setFormState] = useState<ManifestFormState>(emptyManifestForm);
  const [formExtras, setFormExtras] = useState<ManifestExtras>(emptyManifestExtras);
  const [formErrors, setFormErrors] = useState<Set<string>>(new Set());
  const [tomlParseError, setTomlParseError] = useState<string | null>(null);
  const [showPrompts, setShowPrompts] = useState(false);
  const [editingModel, setEditingModel] = useState(false);
  const [modelDraft, setModelDraft] = useState({ provider: "", model: "", max_tokens: "", temperature: "" });
  // Inline-rename state for the detail/edit modal header. The agent name is
  // the primary identifier in the UI and was previously read-only — now
  // clicking the title swaps it for an input and PATCHes /agents/{id}.
  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState("");
  // Destructive-action confirmation dialog. We set this instead of calling
  // window.confirm() so the dialog matches the rest of the dashboard
  // styling and can slide up as a bottom-sheet on mobile.
  const [confirmDialog, setConfirmDialog] = useState<{
    title: string;
    message: string;
    onConfirm: () => void;
    tone?: "default" | "destructive";
  } | null>(null);
  const [showHandAgents, setShowHandAgents] = useState(false);
  const [showToolsEditor, setShowToolsEditor] = useState(false);
  const [toolsEditorAgentId, setToolsEditorAgentId] = useState<string | null>(null);
  const [capabilitiesToolsDraft, setCapabilitiesToolsDraft] = useState<string[]>([]);
  const [toolAllowlistDraft, setToolAllowlistDraft] = useState<string[]>([]);
  const [toolBlocklistDraft, setToolBlocklistDraft] = useState<string[]>([]);
  const [toolsDisabledState, setToolsDisabledState] = useState(false);
  const [toolsEditorLoading, setToolsEditorLoading] = useState(false);
  const [toolsEditorSaving, setToolsEditorSaving] = useState(false);
  const [availableToolNames, setAvailableToolNames] = useState<string[]>([]);
  const [stateFilter, setStateFilter] = useState<"all" | "running" | "suspended">("all");
  const [sortBy, setSortBy] = useState<"name" | "last_active" | "created_at">("name");
  // Tab switcher inside the inline detail panel.  Mirrors the design's
  // five sections (Conversation / Memory / Skills / Schedule / Logs).
  const [agentTab, setAgentTab] = useState<
    "conversation" | "memory" | "skills" | "schedule" | "logs"
  >("conversation");
  // Whether the deep-edit drawer is open. Decoupled from `detailAgent` so
  // selecting an agent in the list shows the inline detail panel without
  // popping a drawer; the drawer is only opened when the user explicitly
  // clicks "Configure" / "Edit" from the detail header's overflow menu.
  const [detailDrawerOpen, setDetailDrawerOpen] = useState(false);
  const addToast = useUIStore((s) => s.addToast);
  useCreateShortcut(() => setShowCreate(true));
  const templatesQuery = useAgentTemplates({
    enabled: showCreate && createMode === "template",
  });
  const localizedTemplates = useMemo(
    () =>
      (templatesQuery.data ?? []).map((template) => ({
        ...template,
        displayName: t(`agents.builtin.${template.name}.name`, { defaultValue: template.name }),
        displayDescription: t(`agents.builtin.${template.name}.description`, {
          defaultValue: template.description || template.name,
        }),
      })),
    [templatesQuery.data, t],
  );
  const selectedTemplate = useMemo(
    () => localizedTemplates.find((template) => template.name === templateName) ?? null,
    [localizedTemplates, templateName],
  );
  const spawnMutation = useSpawnAgent();
  const suspendMutation = useSuspendAgent();
  const resumeMutation = useResumeAgent();
  const patchAgentConfigMutation = usePatchAgentConfig();
  const patchHandAgentRuntimeConfigMutation = usePatchHandAgentRuntimeConfig();
  const patchAgentMutation = usePatchAgent();
  const cloneMutation = useCloneAgent();
  const qc = useQueryClient();

  const rawDeleteMutation = useDeleteAgent();
  const deleteMutation = {
    mutate: (agentId: string) =>
      rawDeleteMutation.mutate(agentId, {
        onSuccess: () => {
          setDetailAgent(prev => {
            if (prev?.id === agentId) {
              setDetailDrawerOpen(false);
              return null;
            }
            return prev;
          });
          addToast(t("agents.delete_success", { defaultValue: "Agent deleted" }), "success");
        },
        onError: (e: Error) =>
          addToast(
            e?.message || t("agents.delete_failed", { defaultValue: "Failed to delete agent" }),
            "error",
          ),
      }),
  };

  function mergeHandFlag(agent: AgentDetail, fallback?: boolean) {
    return { ...agent, is_hand: agent.is_hand ?? fallback };
  }

  function startModelEdit() {
    setModelDraft({
      provider: detailAgent?.model?.provider ?? "",
      model: detailAgent?.model?.model ?? "",
      max_tokens: String(detailAgent?.model?.max_tokens ?? 4096),
      temperature: String(detailAgent?.model?.temperature ?? 0.7),
    });
    setEditingModel(true);
  }

  function cancelModelEdit() {
    setEditingModel(false);
  }

  function closeDetailModal() {
    // Closing the drawer no longer deselects the agent — the inline detail
    // panel remains visible. Use deselectAgent() to fully exit the
    // selection (e.g. when an agent is deleted).
    setDetailDrawerOpen(false);
    setEditingModel(false);
    setEditingName(false);
    closeToolsEditor();
  }

  function startNameEdit() {
    setNameDraft(detailAgent?.name ?? "");
    setEditingName(true);
  }

  function cancelNameEdit() {
    setEditingName(false);
  }

  function saveName() {
    // Re-entrancy guard: Enter pressed twice in quick succession would
    // otherwise queue a second PATCH for the same name, which the kernel's
    // `update_name` rejects with `AgentAlreadyExists` (the name_index
    // entry was just claimed by the first call) — surfacing as a
    // misleading "Failed to rename" toast for the user. The Save button
    // already has the same disable check; mirror it here for keyboard
    // submits.
    if (patchAgentMutation.isPending) return;
    const trimmed = nameDraft.trim();
    if (!detailAgent || !trimmed || trimmed === detailAgent.name) {
      setEditingName(false);
      return;
    }
    // Capture the agent id we're renaming. If the user closes this modal
    // and opens a different agent before the mutation resolves, the
    // onSuccess handler must NOT smuggle this rename's name onto the new
    // agent's local state.
    const targetId = detailAgent.id;
    patchAgentMutation.mutate(
      { agentId: targetId, body: { name: trimmed } },
      {
        onSuccess: () => {
          // Optimistic local update so the header reflects the new name
          // before the agents query refetch lands. Gate on id so a stale
          // mutation completing after the user navigated to another
          // agent doesn't overwrite that other agent's name.
          setDetailAgent(prev =>
            prev?.id === targetId ? { ...prev, name: trimmed } : prev,
          );
          setEditingName(false);
          addToast(t("agents.rename_success", { defaultValue: "Agent renamed" }), "success");
        },
        onError: (e: Error) => {
          addToast(
            e?.message || t("agents.rename_failed", { defaultValue: "Failed to rename agent" }),
            "error",
          );
        },
      },
    );
  }

  async function refreshDetailAgent(agentId: string, fallback?: boolean) {
    try {
      await qc.invalidateQueries({ queryKey: agentQueries.detail(agentId).queryKey });
      const d = await qc.fetchQuery(agentQueries.detail(agentId));
      setDetailAgent(mergeHandFlag(d, fallback));
    } catch {
      // keep current state when refresh fails
    }
  }

  function closeToolsEditor() {
    setShowToolsEditor(false);
    setToolsEditorAgentId(null);
    setToolsEditorLoading(false);
    setToolsEditorSaving(false);
    setAvailableToolNames([]);
    setCapabilitiesToolsDraft([]);
    setToolAllowlistDraft([]);
    setToolBlocklistDraft([]);
    setToolsDisabledState(false);
  }

  useEffect(() => {
    if (!showToolsEditor || !toolsEditorAgentId) return;

    let cancelled = false;

    async function loadToolsEditorState() {
      setToolsEditorLoading(true);
      try {
        const agentId = toolsEditorAgentId;
        if (!agentId) return;
        const [allTools, agentTools] = await Promise.all([
          listTools(),
          getAgentTools(agentId),
        ]);
        if (cancelled) return;
        const names = Array.isArray(allTools)
          ? allTools.map((tool: ToolDefinition) => tool.name).filter(Boolean)
          : [];
        setAvailableToolNames(names);
        setCapabilitiesToolsDraft(Array.isArray(agentTools?.capabilities_tools) ? agentTools.capabilities_tools : []);
        setToolAllowlistDraft(Array.isArray(agentTools?.tool_allowlist) ? agentTools.tool_allowlist : []);
        setToolBlocklistDraft(Array.isArray(agentTools?.tool_blocklist) ? agentTools.tool_blocklist : []);
        setToolsDisabledState(Boolean(agentTools?.disabled));
      } catch (err) {
        if (cancelled) return;
        addToast(toastErr(err, t("agents.tools_load_failed", { defaultValue: "Failed to load tools" })), "error");
      } finally {
        if (!cancelled) {
          setToolsEditorLoading(false);
        }
      }
    }

    void loadToolsEditorState();

    return () => {
      cancelled = true;
    };
  }, [showToolsEditor, toolsEditorAgentId, addToast, t]);

  function saveModelEdit() {
    if (!detailAgent) return;
    const current = detailAgent.model;
    const patch: { max_tokens?: number; model?: string; provider?: string; temperature?: number } = {};

    const trimmedProvider = modelDraft.provider.trim();
    const trimmedModel = modelDraft.model.trim();
    const parsedMaxTokens = parseInt(modelDraft.max_tokens, 10);
    const parsedTemperature = parseFloat(modelDraft.temperature);

    if (!trimmedProvider || !trimmedModel) return;
    if (isNaN(parsedMaxTokens) || parsedMaxTokens <= 0) return;
    if (isNaN(parsedTemperature) || parsedTemperature < 0 || parsedTemperature > 2) return;

    const modelChanged = trimmedModel !== current?.model;
    const providerChanged = trimmedProvider !== current?.provider;

    if (modelChanged || providerChanged) {
      patch.model = trimmedModel;
      patch.provider = trimmedProvider;
    }
    if (parsedMaxTokens !== current?.max_tokens) patch.max_tokens = parsedMaxTokens;
    if (parsedTemperature !== current?.temperature) patch.temperature = parsedTemperature;

    if (Object.keys(patch).length === 0) {
      setEditingModel(false);
      return;
    }

    // Caller picks the mutation based on cached agent-detail knowledge: hand
    // agents go through the hand-runtime-config endpoint (also invalidates
    // handKeys.details()), everyone else hits the standalone /config route.
    const mutation = detailAgent.is_hand
      ? patchHandAgentRuntimeConfigMutation
      : patchAgentConfigMutation;
    mutation.mutate(
      { agentId: detailAgent.id, config: patch },
      {
        onSuccess: async () => {
          setEditingModel(false);
          await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
          addToast(t("agents.model_saved", { defaultValue: "Model updated" }), "success");
        },
        onError: (e) => {
          addToast(
            toastErr(e, t("agents.model_save_failed", { defaultValue: "Failed to update model" })),
            "error",
          );
        },
      },
    );
  }

  // Share the snapshot query with OverviewPage — same cache key means React Query
  // deduplicates the poll when both pages are mounted, and agent counts on the
  // Overview tab stay in sync with this list automatically.
  const agentsQuery = useDashboardSnapshot();
  // Sessions index, used by both the list row's per-agent "sessions · cost"
  // suffix and the detail panel's KPI tiles. Single fetch per render so the
  // 30s refetch interval is the only network cost.
  const sessionsQuery = useSessions();
  // Detail-panel data sources. Memory + audit are global lists filtered
  // client-side by agent id; cron is server-side filtered (its own
  // `enabled` flag gates the network request on `detailAgent?.id`).
  // TanStack Query dedupes / caches across pages so revisiting an agent
  // is free.
  // Per-agent KV memory (matches the design canvas's key/value/age rows).
  // The previous useMemorySearchOrList(\"\") query returned global proactive
  // memory, which is empty unless [proactive_memory] is enabled — so the
  // tab read empty even when the agent had KV pairs.
  const agentKvMemoryQuery = useAgentKvMemory(detailAgent?.id ?? "");
  const cronJobsQuery = useCronJobs(detailAgent?.id);
  // Per-agent KPI rollup (#4246) — replaces a global /api/sessions scan
  // that was capped by pagination and missed agents whose sessions
  // weren't in the latest N rows.
  const agentStatsQuery = useAgentStats(detailAgent?.id ?? "");
  // Per-agent triggers — GET /api/triggers?agent_id=… so the Schedule
  // tab's event-trigger cards don't depend on agent detail embedding
  // them (which it currently doesn't).
  const agentTriggersQuery = useAgentTriggers(detailAgent?.id ?? "");
  // Per-agent recent turn events — backs the Logs tab. usage_events is
  // turn-level (model dispatch, latency, tokens, cost) — exactly what
  // the design's stderr-style log feed wants. The previous source
  // (global audit) only had admin lifecycle entries, leaving the tab
  // blank for almost every agent.
  const agentEventsQuery = useAgentEvents(detailAgent?.id ?? "", 30);
  // Per-agent session list — Conversation tab uses this directly. The
  // global /api/sessions used previously was paginated to 50, so the
  // agent's latest session was often not in the page.
  const agentSessionsQuery = useAgentSessions(detailAgent?.id ?? "");
  const latestSessionForAgent = useMemo(() => {
    const list = agentSessionsQuery.data ?? [];
    if (list.length === 0) return undefined;
    let best: { session_id: string; ts: number } | undefined;
    for (const s of list) {
      const ts = s.created_at ? Date.parse(s.created_at) : 0;
      if (!best || ts > best.ts) best = { session_id: s.session_id, ts };
    }
    return best?.session_id;
  }, [agentSessionsQuery.data]);
  const sessionDetailQuery = useSessionDetails(latestSessionForAgent ?? "");
  // Row-level aggregate only — detail-panel KPI reads from the per-agent
  // /stats endpoint (useAgentStats) which doesn't suffer from the global
  // /api/sessions pagination cap.
  const sessionsByAgent = useMemo(() => {
    const map = new Map<string, { sessions24h: number; cost24h: number }>();
    const cutoff = Date.now() - 24 * 60 * 60 * 1000;
    for (const s of sessionsQuery.data ?? []) {
      const id = s.agent_id;
      if (!id) continue;
      const ts = s.created_at ? Date.parse(s.created_at) : 0;
      if (ts < cutoff) continue;
      const entry = map.get(id) ?? { sessions24h: 0, cost24h: 0 };
      entry.sessions24h += 1;
      entry.cost24h += typeof s.cost_usd === "number" ? s.cost_usd : 0;
      map.set(id, entry);
    }
    return map;
  }, [sessionsQuery.data]);


  const modelsQuery = useModels(
    { provider: modelDraft.provider },
    { enabled: !!modelDraft.provider.trim() },
  );

  // Separate models query for the create-form's chosen provider. We don't
  // reuse modelsQuery because that one is gated on the inline-edit widget's
  // selection, which is unrelated to the create modal.
  const formModelsQuery = useModels(
    { provider: formState.model.provider },
    { enabled: showCreate && createMode === "form" && !!formState.model.provider.trim() },
  );

  const providersQuery = useProviders();

  const configuredProviders = useMemo(
    () => (providersQuery.data ?? []).filter(p => isProviderAvailable(p.auth_status)),
    [providersQuery.data],
  );

  // Form-mode option lists (only providers that have credentials configured).
  const formProviderOptions = useMemo(
    () => configuredProviders.map((p) => ({ name: p.id })),
    [configuredProviders],
  );
  const formModelOptions = useMemo(
    () =>
      (formModelsQuery.data?.models ?? []).map((m) => ({
        provider: m.provider,
        id: m.id,
      })),
    [formModelsQuery.data?.models],
  );
  const serializedFormToml = useMemo(
    () => serializeManifestForm(formState, formExtras),
    [formState, formExtras],
  );
  const serializedFormMarkdown = useMemo(
    () => generateManifestMarkdown(formState, formExtras),
    [formState, formExtras],
  );
  const [previewTab, setPreviewTab] = useState<"toml" | "markdown">("toml");

  // Single close path for the create modal so the X button, the
  // Cancel button, and the onSuccess handler after spawn all clear the
  // same transient state. Template selection + custom name are cleared
  // here because they're per-attempt picks; form/TOML drafts persist so
  // users can reopen the modal and resume where they left off.
  const closeCreateModal = () => {
    setShowCreate(false);
    setFormErrors(new Set());
    setTomlParseError(null);
    setTemplateName("");
    setTemplateCustomName("");
    // Don't reset while a spawn is in flight — reset() flips isPending
    // back to false, and since the fetch isn't actually aborted the user
    // could reopen the modal and submit again before the first response
    // lands, producing a duplicate-spawn "already exists" error (the
    // exact bug #2741 was meant to fix). Once the original request
    // settles, isPending goes false on its own.
    if (!spawnMutation.isPending) {
      spawnMutation.reset();
    }
  };

  // Bidirectional Form ⇄ TOML sync. Going Form→TOML pushes the form's
  // serialized output into the textarea so advanced users can keep editing.
  // Going TOML→Form parses what's in the textarea, populating the form
  // and stashing unmapped fields ([thinking], [tools.*], etc.) in extras
  // so they survive a re-serialize.
  const switchCreateMode = (next: "form" | "template" | "toml") => {
    if (next === createMode) return;
    if (next === "form" && manifestToml.trim() && manifestToml !== serializedFormToml) {
      const parsed = parseManifestToml(manifestToml);
      if (!parsed.ok) {
        setTomlParseError(
          parsed.line !== undefined
            ? `Line ${parsed.line}:${parsed.column ?? 0} — ${parsed.message}`
            : parsed.message,
        );
        return;
      }
      setFormState(parsed.form);
      setFormExtras(parsed.extras);
      setTomlParseError(null);
    }
    if (next === "toml" && createMode === "form") {
      setManifestToml(serializedFormToml);
      setTomlParseError(null);
    }
    setCreateMode(next);
  };

  const hiddenModelKeys = useUIStore((s) => s.hiddenModelKeys);
  const hiddenSet = useMemo(() => new Set(hiddenModelKeys), [hiddenModelKeys]);

  const visibleModels = useMemo(
    () => filterVisible(modelsQuery.data?.models ?? [], hiddenSet),
    [modelsQuery.data?.models, hiddenSet],
  );

  const agents = agentsQuery.data?.agents ?? [];
  const visibleAgents = useMemo(
    () => showHandAgents ? agents : agents.filter(a => !a.is_hand),
    [agents, showHandAgents],
  );
  // Counts for the filter chips so operators can see "5 running / 2
  // suspended" without running through the filter first.
  const agentCounts = useMemo(() => {
    const visible = visibleAgents;
    const running = visible.filter(a => (a.state || "").toLowerCase() === "running").length;
    const suspended = visible.filter(a => (a.state || "").toLowerCase() === "suspended").length;
    return { all: visible.length, running, suspended };
  }, [visibleAgents]);
  const filteredAgents = useMemo(() => visibleAgents
    .filter(a => {
      if (stateFilter === "all") return true;
      return (a.state || "").toLowerCase() === stateFilter;
    })
    .filter(a => a.name.toLowerCase().includes(search.toLowerCase()) || a.id.toLowerCase().includes(search.toLowerCase()))
    .sort((a, b) => {
      // Suspended always last regardless of primary sort — otherwise a
      // "sort by recent" view would bury running agents behind stale
      // suspended ones that happened to be touched recently.
      const aSusp = (a.state || "").toLowerCase() === "suspended" ? 1 : 0;
      const bSusp = (b.state || "").toLowerCase() === "suspended" ? 1 : 0;
      if (aSusp !== bSusp) return aSusp - bSusp;
      if (sortBy === "last_active") {
        const aT = a.last_active ? Date.parse(a.last_active) : 0;
        const bT = b.last_active ? Date.parse(b.last_active) : 0;
        return bT - aT; // most recent first
      }
      if (sortBy === "created_at") {
        const aT = a.created_at ? Date.parse(a.created_at) : 0;
        const bT = b.created_at ? Date.parse(b.created_at) : 0;
        return bT - aT; // newest first
      }
      return a.name.localeCompare(b.name);
    }), [visibleAgents, search, stateFilter, sortBy]);

  const coreAgents = filteredAgents;
  const conflictingToolNames = useMemo(
    () => toolAllowlistDraft.filter((name) => toolBlocklistDraft.includes(name)),
    [toolAllowlistDraft, toolBlocklistDraft],
  );

  const selectAgent = async (agent: AgentItem) => {
    setAgentTab("conversation");
    setDetailLoading(true);
    try {
      const d = await qc.fetchQuery(agentQueries.detail(agent.id));
      setDetailAgent(mergeHandFlag(d, agent.is_hand));
    } catch {
      setDetailAgent({ name: agent.name, id: agent.id, is_hand: agent.is_hand } as AgentDetail);
    }
    setDetailLoading(false);
  };

  // Auto-select the first agent on desktop so the detail panel isn't blank
  // on first paint. Skipped on mobile because the detail view is a full-
  // screen overlay there — auto-opening it would block access to the list.
  useEffect(() => {
    if (detailAgent) return;
    if (filteredAgents.length === 0) return;
    if (typeof window !== "undefined" && !window.matchMedia("(min-width: 1024px)").matches) return;
    void selectAgent(filteredAgents[0]);
    // selectAgent is recreated each render; depending on filteredAgents+detailAgent is sufficient.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filteredAgents, detailAgent]);

  const renderAgentRow = (agent: AgentItem) => {
    const isSelected = detailAgent?.id === agent.id;
    // Prefer the row-embedded stats from /api/agents (single grouped SQL
    // pass). Fall back to the global aggregation only if the backend is
    // older and didn't ship the field.
    const sessions24h = typeof agent.sessions_24h === "number"
      ? agent.sessions_24h
      : (sessionsByAgent.get(agent.id)?.sessions24h ?? 0);
    const cost24h = typeof agent.cost_24h === "number"
      ? agent.cost_24h
      : (sessionsByAgent.get(agent.id)?.cost24h ?? 0);
    const stats = { sessions24h, cost24h };
    const stateLower = (agent.state || "").toLowerCase();
    return (
      <button
        key={agent.id}
        type="button"
        onClick={() => void selectAgent(agent)}
        className={`w-full text-left px-3.5 py-2.5 border-l-2 border-b border-border-subtle/40 transition-colors cursor-pointer ${
          isSelected
            ? "border-l-brand bg-brand/5"
            : "border-l-transparent bg-transparent hover:bg-main/40"
        } ${stateLower === "suspended" ? "opacity-70" : ""}`}
      >
        <div className="flex items-center gap-2 min-w-0">
          <Badge variant={getStatusVariant(agent.state)} dot className="shrink-0">
            <span className="sr-only">{agent.state || "idle"}</span>
          </Badge>
          <span className="font-mono text-[13px] truncate flex-1 min-w-0 text-text-main">
            {t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}
          </span>
          {agent.is_hand && (
            <span className="shrink-0 text-[9px] font-bold px-1.5 py-px rounded bg-brand/10 text-brand">
              {t("agents.hand_badge", { defaultValue: "HAND" })}
            </span>
          )}
          <span className="font-mono text-[10.5px] text-text-dim/80 shrink-0 tabular-nums">
            {agent.last_active ? formatRelativeTime(agent.last_active) : "—"}
          </span>
        </div>
        <div className="font-mono text-[10.5px] text-text-dim flex items-center gap-2 pl-[22px] mt-1">
          <span className="truncate min-w-0">{agent.model_name || agent.model_provider || "—"}</span>
          <span className="text-text-dim/60">·</span>
          <span className="truncate min-w-0">
            {agent.schedule || t("agents.schedule_manual", { defaultValue: "manual" })}
          </span>
          <span className="ml-auto shrink-0 tabular-nums">
            {stats.sessions24h} · ${stats.cost24h.toFixed(2)}
          </span>
        </div>
      </button>
    );
  };

  // Inline detail panel — replaces the old card-grid + drawer mix.  The
  // drawer is kept around for deep edits (rename, model, tools, prompts)
  // and opened from the panel's overflow menu.  Five tabs mirror the
  // design: Conversation / Memory / Skills / Schedule / Logs.
  const renderDetailPanel = (agent: AgentDetail) => {
    const detailState = ((agent as AgentView).state || "").toLowerCase();
    const isSuspended = detailState === "suspended";
    const isCrashed = detailState === "crashed";
    const detailCaps = (agent as AgentView).capabilities;
    const toolsCount = Array.isArray(detailCaps?.tools) ? detailCaps.tools.length : 0;
    const tabs: Array<{ id: typeof agentTab; label: string; Icon: typeof Bot }> = [
      { id: "conversation", label: t("agents.tab.conversation", { defaultValue: "Conversation" }), Icon: MessageCircle },
      { id: "memory",       label: t("agents.tab.memory",       { defaultValue: "Memory" }),       Icon: Database },
      { id: "skills",       label: t("agents.tab.skills",       { defaultValue: "Skills" }),       Icon: Sparkles },
      { id: "schedule",     label: t("agents.tab.schedule",     { defaultValue: "Schedule" }),     Icon: Clock },
      { id: "logs",         label: t("agents.tab.logs",         { defaultValue: "Logs" }),         Icon: FileText },
    ];

    return (
      <Card padding="none" className="surface-lit overflow-hidden flex flex-col min-h-0 lg:min-h-[640px] h-full lg:h-auto">
        {/* Header */}
        <div className="px-3 sm:px-5 pt-3 sm:pt-4 pb-3 border-b border-border-subtle">
          <div className="flex items-center gap-2 sm:gap-3">
            {/* Mobile-only "back to list" affordance — closes the detail
                overlay without deselecting state for lg+. */}
            <button
              type="button"
              onClick={() => setDetailAgent(null)}
              className="lg:hidden -ml-1 p-1.5 rounded-md text-text-dim hover:text-text-main hover:bg-main/40 shrink-0"
              aria-label={t("common.back", { defaultValue: "Back" })}
            >
              <X className="w-4 h-4" />
            </button>
            <div className="w-9 h-9 rounded-lg bg-brand/10 border border-brand/30 grid place-items-center text-brand shrink-0">
              <Bot className="w-[18px] h-[18px]" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 min-w-0">
                <h2 className="font-mono font-semibold text-base truncate text-text-main">
                  {t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}
                </h2>
                <Badge variant={getStatusVariant((agent as AgentView).state)} dot className="shrink-0">
                  {(agent as AgentView).state
                    ? t(`common.${((agent as AgentView).state || "").toLowerCase()}`, { defaultValue: (agent as AgentView).state })
                    : t("common.idle")}
                </Badge>
              </div>
              <p className="font-mono text-[11.5px] text-text-dim/80 truncate mt-0.5">
                {truncateId(agent.id)}
                {agent.model?.model || (agent as AgentView).model_name
                  ? ` · ${agent.model?.model || (agent as AgentView).model_name}`
                  : ""}
                {(agent as AgentView).profile ? ` · ${(agent as AgentView).profile}` : ""}
              </p>
            </div>
            {/* Action cluster — labels collapse on mobile so the row keeps
                three icon-buttons + back arrow on a 390px viewport. */}
            <div className="flex items-center gap-1 sm:gap-1.5 shrink-0">
              {isSuspended ? (
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={<Play className="w-3.5 h-3.5" />}
                  aria-label={t("agents.resume", { defaultValue: "Resume" })}
                  title={t("agents.resume", { defaultValue: "Resume" })}
                  onClick={async () => {
                    try { await resumeMutation.mutateAsync(agent.id); } catch (e) {
                      addToast(toastErr(e, t("agents.resume_failed", { defaultValue: "Failed to resume agent" })), "error");
                    }
                  }}
                >
                  <span className="hidden sm:inline">{t("agents.resume", { defaultValue: "Resume" })}</span>
                </Button>
              ) : (
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={<Pause className="w-3.5 h-3.5" />}
                  aria-label={t("agents.suspend", { defaultValue: "Pause" })}
                  title={t("agents.suspend", { defaultValue: "Pause" })}
                  onClick={async () => {
                    try { await suspendMutation.mutateAsync(agent.id); } catch (e) {
                      addToast(toastErr(e, t("agents.suspend_failed", { defaultValue: "Failed to suspend agent" })), "error");
                    }
                  }}
                >
                  <span className="hidden sm:inline">{t("agents.suspend", { defaultValue: "Pause" })}</span>
                </Button>
              )}
              <Button
                variant="secondary"
                size="sm"
                leftIcon={<MessageCircle className="w-3.5 h-3.5" />}
                aria-label={t("common.interact", { defaultValue: "Chat" })}
                title={t("common.interact", { defaultValue: "Chat" })}
                onClick={() => navigate({ to: "/chat", search: { agentId: agent.id } })}
              >
                <span className="hidden sm:inline">{t("common.interact", { defaultValue: "Chat" })}</span>
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setDetailDrawerOpen(true)}
                title={t("agents.configure", { defaultValue: "Configure" })}
                aria-label={t("agents.configure", { defaultValue: "Configure" })}
              >
                <MoreHorizontal className="w-4 h-4" />
              </Button>
            </div>
          </div>

          {/* KPI tiles — Sessions · Cost · P95 · Tools (matches design canvas).
              Backed by GET /api/agents/{id}/stats so values are accurate even
              when the agent hasn't appeared in the global session list page. */}
          {(() => {
            const live = agentStatsQuery.data;
            const sessions24h = live?.sessions_24h ?? 0;
            const cost24h = live?.cost_24h ?? 0;
            const p95Ms = live?.p95_latency_ms ?? 0;
            const samples = live?.samples ?? 0;
            const activeNow = live?.active_now ?? 0;
            const prev = live?.prev;
            const skillNames: string[] = Array.isArray((agent as AgentView).skills)
              ? ((agent as AgentView).skills as string[])
              : Array.isArray((agent as AgentView).capabilities?.skills)
                ? (((agent as AgentView).capabilities!.skills) as string[])
                : [];
            const toolsMeta = skillNames.length > 0
              ? skillNames.slice(0, 3).join(" · ")
              : toolsCount > 0
                ? `${toolsCount} configured`
                : "—";

            // Trend deltas vs the prior 24h. Percent change for counts,
            // signed dollar for cost, signed milliseconds for latency.
            // When the prior period is empty we surface "new" instead of
            // a divide-by-zero-induced "+∞%".
            const pctDelta = (cur: number, p: number): string => {
              if (p === 0) return cur > 0 ? "new" : "—";
              const d = ((cur - p) / p) * 100;
              const sign = d >= 0 ? "+" : "−";
              return `${sign}${Math.abs(d).toFixed(0)}%`;
            };
            const usdDelta = (cur: number, p: number): string => {
              const d = cur - p;
              // Treat sub-cent moves as no-change rather than rendering
              // "−$0.00" / "+$0.00", which looks like a real signal.
              if (Math.abs(d) < 0.01) return cur === 0 && p === 0 ? "—" : "≈$0.00";
              const sign = d >= 0 ? "+" : "−";
              return `${sign}$${Math.abs(d).toFixed(2)}`;
            };
            const msDelta = (cur: number, p: number): string => {
              if (cur === 0 && p === 0) return "—";
              if (p === 0) return "new";
              const d = cur - p;
              const sign = d >= 0 ? "+" : "−";
              return Math.abs(d) >= 1000
                ? `${sign}${(Math.abs(d) / 1000).toFixed(1)}s`
                : `${sign}${Math.abs(Math.round(d))}ms`;
            };

            return (
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 mt-4">
                {[
                  {
                    l: t("agents.kpi.sessions", { defaultValue: "Sessions · 24h" }),
                    v: String(sessions24h),
                    m: prev
                      ? activeNow > 0
                        ? `${activeNow} live · ${pctDelta(sessions24h, prev.sessions_24h)}`
                        : pctDelta(sessions24h, prev.sessions_24h)
                      : activeNow > 0 ? `${activeNow} live` : "—",
                  },
                  {
                    l: t("agents.kpi.cost", { defaultValue: "Cost · 24h" }),
                    v: `$${cost24h.toFixed(2)}`,
                    m: prev ? usdDelta(cost24h, prev.cost_24h) : "—",
                  },
                  {
                    l: t("agents.kpi.p95", { defaultValue: "P95 latency" }),
                    v: p95Ms > 0
                      ? p95Ms >= 1000
                        ? `${(p95Ms / 1000).toFixed(2)}s`
                        : `${Math.round(p95Ms)}ms`
                      : "—",
                    m: prev && (samples > 0 || prev.p95_latency_ms > 0)
                      ? msDelta(p95Ms, prev.p95_latency_ms)
                      : samples > 0
                        ? t("agents.kpi.samples", { count: samples, defaultValue: "{{count}} samples" })
                        : "—",
                  },
                  {
                    l: t("agents.kpi.tools", { defaultValue: "Tools" }),
                    v: String(toolsCount || "—"),
                    m: toolsMeta,
                  },
                ].map((s) => (
                  <div key={s.l} className="px-3 py-2 rounded-md bg-main/60 border border-border-subtle">
                    <div className="text-[10px] uppercase font-semibold text-text-dim tracking-[0.08em]">{s.l}</div>
                    <div className="font-mono font-semibold text-[17px] mt-1 truncate tabular-nums text-text-main">{s.v}</div>
                    <div className="text-[10.5px] text-text-dim/80 mt-0.5 truncate">{s.m}</div>
                  </div>
                ))}
              </div>
            );
          })()}

          {/* Tabs */}
          <div className="flex gap-1 mt-4 -mb-3 border-b border-border-subtle overflow-x-auto">
            {tabs.map((tab) => {
              const active = agentTab === tab.id;
              const Icon = tab.Icon;
              return (
                <button
                  key={tab.id}
                  onClick={() => setAgentTab(tab.id)}
                  className={`px-3 py-2 text-[12.5px] flex items-center gap-1.5 border-b-2 -mb-px shrink-0 transition-colors cursor-pointer ${
                    active
                      ? "border-brand text-text-main font-medium"
                      : "border-transparent text-text-dim hover:text-text-main"
                  }`}
                >
                  <Icon className="w-[13px] h-[13px]" />
                  {tab.label}
                </button>
              );
            })}
          </div>
        </div>

        {/* Tab content */}
        <div className="flex-1 overflow-y-auto px-3 sm:px-5 py-3 sm:py-4">
          {renderTabContent(agent, isCrashed)}
        </div>
      </Card>
    );
  };

  const renderTabContent = (agent: AgentDetail, isCrashed: boolean) => {
    if (isCrashed && agentTab === "conversation") {
      return (
        <EmptyState
          title={t("agents.detail.crashed_title", { defaultValue: `${agent.name} is in error state` })}
          icon={<X className="h-6 w-6 text-error" />}
          action={
            <Button variant="primary" size="sm" leftIcon={<RotateCcw className="h-3.5 w-3.5" />} onClick={async () => {
              try { await resumeMutation.mutateAsync(agent.id); } catch (e) {
                addToast(toastErr(e, t("agents.resume_failed", { defaultValue: "Failed to resume" })), "error");
              }
            }}>
              {t("agents.resume", { defaultValue: "Resume" })}
            </Button>
          }
        />
      );
    }
    switch (agentTab) {
      case "conversation":      return renderConversationTab(agent);
      case "memory":            return renderMemoryTab(agent);
      case "skills":            return renderSkillsTab(agent);
      case "schedule":          return renderScheduleTab(agent);
      case "logs":              return renderLogsTab(agent);
    }
  };

  // ---------- Conversation tab — chat-bubble preview of latest session
  const renderConversationTab = (agent: AgentDetail) => {
    const sessionData = sessionDetailQuery.data as
      | { messages?: Array<{ role?: string; content?: unknown }> }
      | undefined;
    const allMessages = Array.isArray(sessionData?.messages) ? sessionData!.messages! : [];
    const visibleMessages = allMessages
      .filter((m) => m.role === "user" || m.role === "assistant")
      .slice(-5);
    const messageText = (m: { content?: unknown }): string => {
      if (typeof m.content === "string") return m.content;
      if (Array.isArray(m.content)) {
        return (m.content as Array<{ type?: string; text?: string }>)
          .filter((b) => b.type === "text" || b.text)
          .map((b) => b.text ?? "")
          .join(" ");
      }
      return "";
    };
    return (
      <div className="flex flex-col gap-2.5">
        <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim mb-1">
          {t("agents.detail.live_conversation", { defaultValue: "Live conversation" })}
        </div>
        {sessionDetailQuery.isLoading && latestSessionForAgent ? (
          <div className="text-[12px] text-text-dim italic">{t("common.loading", { defaultValue: "Loading…" })}</div>
        ) : visibleMessages.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_conversation", {
              defaultValue: "No conversation yet — open the chat to send the first message.",
            })}
          </div>
        ) : (
          visibleMessages.map((m, i) => {
            const isUser = m.role === "user";
            const txt = messageText(m).trim();
            if (!txt) return null;
            // Truncate first, then render. The full conversation lives
            // behind the "Open chat" button — this is a preview only.
            const preview = txt.length > 280 ? `${txt.slice(0, 280)}…` : txt;
            return (
              <div key={i} className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                <div
                  className={`max-w-[78%] rounded-lg px-3 py-2 text-[12.5px] break-words border ${
                    isUser
                      ? "bg-brand/10 border-brand/30 text-text-main"
                      : "bg-main/60 border-border-subtle text-text-main"
                  }`}
                >
                  {isUser ? (
                    <span className="whitespace-pre-wrap">{preview}</span>
                  ) : (
                    <MarkdownContent>{preview}</MarkdownContent>
                  )}
                </div>
              </div>
            );
          })
        )}
        <div className="flex justify-start mt-1">
          <Button
            variant="primary"
            size="sm"
            leftIcon={<MessageCircle className="h-3.5 w-3.5" />}
            onClick={() => navigate({ to: "/chat", search: { agentId: agent.id } })}
          >
            {t("agents.detail.open_chat", { defaultValue: "Open chat" })}
          </Button>
        </div>
      </div>
    );
  };

  // ---------- Memory tab — per-agent KV row layout per design canvas
  const renderMemoryTab = (agent: AgentDetail) => {
    const kv = agentKvMemoryQuery.data ?? [];

    // The kv_store backs both real KV (`user.preferences.tone` →
    // `"concise"`) and the proactive-memory cache (key = `memory:<uuid>`,
    // value = the full MemoryItem JSON). Render those two cases
    // differently so the proactive entries don't dump 600-char JSON
    // blobs into the row.
    type View = { key: string; value: string; ageIso?: string };
    const projected: View[] = kv.map((r) => {
      const value = r.value as unknown;
      if (
        typeof r.key === "string" &&
        r.key.startsWith("memory:") &&
        value &&
        typeof value === "object" &&
        !Array.isArray(value)
      ) {
        const obj = value as Record<string, unknown>;
        const content = typeof obj.content === "string" ? obj.content : "";
        const category = typeof obj.category === "string" ? obj.category : "memory";
        const createdAt = typeof obj.created_at === "string" ? obj.created_at : undefined;
        return { key: category, value: content || "—", ageIso: createdAt };
      }
      // Plain KV: show value as a string. Avoid JSON.stringify wrapping
      // strings with extra quotes.
      const valueStr = typeof value === "string"
        ? value
        : value == null
          ? "—"
          : (() => {
              try {
                return JSON.stringify(value);
              } catch {
                return String(value);
              }
            })();
      return {
        key: r.key,
        value: valueStr,
        ageIso: r.created_at,
      };
    });
    const rows = projected.slice(0, 8);
    return (
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
            {t("agents.detail.memory_label", { defaultValue: "Memory · sqlite" })} · {kv.length}
          </div>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={<Database className="h-3.5 w-3.5" />}
            onClick={() => navigate({ to: "/memory", search: { agentId: agent.id } as never })}
          >
            {t("agents.detail.open_memory", { defaultValue: "Open" })}
          </Button>
        </div>
        {agentKvMemoryQuery.isLoading ? (
          <div className="text-[12px] text-text-dim italic">{t("common.loading", { defaultValue: "Loading…" })}</div>
        ) : rows.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_memory", { defaultValue: "No memory entries yet for this agent." })}
          </div>
        ) : (
          <div className="flex flex-col gap-1.5">
            {rows.map((r, i) => (
              <div
                key={`${r.key}-${i}`}
                className="flex flex-col sm:flex-row sm:items-center gap-1 sm:gap-2.5 px-3 py-2 rounded-md border border-border-subtle bg-main/40"
              >
                <div className="flex items-center justify-between gap-2 sm:contents">
                  <span className="font-mono text-[12px] text-brand sm:min-w-[180px] truncate sm:shrink-0 min-w-0">{r.key}</span>
                  <span className="font-mono text-[10.5px] text-text-dim/70 sm:order-3 sm:shrink-0 tabular-nums shrink-0">
                    {r.ageIso ? formatRelativeTime(r.ageIso) : "—"}
                  </span>
                </div>
                <span className="font-mono text-[12px] text-text-dim sm:flex-1 min-w-0 truncate sm:order-2" title={r.value}>
                  {r.value}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  };

  // ---------- Skills tab — 2-col card grid per design canvas
  const renderSkillsTab = (agent: AgentDetail) => {
    const view = agent as AgentView;
    const skills: string[] = Array.isArray(view.skills)
      ? view.skills
      : Array.isArray(view.capabilities?.skills)
        ? view.capabilities!.skills!
        : [];
    // skills_mode: 'none' (skills_disabled), 'all' (no allowlist — uses
    // every skill in the registry, the default), or 'allowlist' (manifest
    // pinned a list). Each needs a different empty-state copy; the
    // previous code collapsed them all to "0 installed".
    const skillsMode = (agent as AgentDetail).skills_mode;
    const usesAllSkills = skillsMode === "all" && skills.length === 0;
    const skillsDisabled = skillsMode === "none";
    return (
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
            {t("agents.detail.installed_skills", { defaultValue: "Installed skills" })}
            {" · "}
            {usesAllSkills
              ? t("agents.detail.skills_all", { defaultValue: "all" })
              : skills.length}
          </div>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={<Plus className="h-3.5 w-3.5" />}
            onClick={() => navigate({ to: "/skills" })}
          >
            {t("agents.detail.install_skill", { defaultValue: "Install" })}
          </Button>
        </div>
        {skillsDisabled ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 flex items-start gap-3">
            <X className="w-4 h-4 text-text-dim shrink-0 mt-0.5" />
            <div className="min-w-0 flex-1">
              <div className="font-mono text-[12.5px] font-medium text-text-main">
                {t("agents.detail.skills_disabled_title", { defaultValue: "Skills disabled" })}
              </div>
              <div className="font-mono text-[10.5px] text-text-dim/80 mt-0.5">
                {t("agents.detail.skills_disabled_desc", {
                  defaultValue: "manifest pinned skills_disabled = true — the agent runs without skill dispatch",
                })}
              </div>
            </div>
          </div>
        ) : usesAllSkills ? (
          <div
            onClick={() => navigate({ to: "/skills" })}
            className="rounded-md border border-border-subtle bg-main/40 p-4 flex items-start gap-3 cursor-pointer hover:border-brand/40 transition-colors"
          >
            <Sparkles className="w-4 h-4 text-brand/80 shrink-0 mt-0.5" />
            <div className="min-w-0 flex-1">
              <div className="font-mono text-[12.5px] font-medium text-text-main">
                {t("agents.detail.skills_all_title", { defaultValue: "Using all available skills" })}
              </div>
              <div className="font-mono text-[10.5px] text-text-dim/80 mt-0.5">
                {t("agents.detail.skills_all_desc", {
                  defaultValue: "manifest doesn't pin an allowlist — every skill in the registry is available",
                })}
              </div>
            </div>
          </div>
        ) : skills.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_skills", { defaultValue: "No skills installed for this agent." })}
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
            {skills.map((s) => (
              <div
                key={s}
                onClick={() => navigate({ to: "/skills" })}
                className="px-3 py-2.5 rounded-md border border-border-subtle bg-main/40 cursor-pointer hover:border-brand/40 transition-colors flex items-start justify-between gap-2"
              >
                <div className="min-w-0 flex-1">
                  <div className="font-mono text-[12.5px] font-medium text-text-main truncate">{s}</div>
                  <div className="font-mono text-[10.5px] text-text-dim/80 mt-0.5 truncate">
                    {t("agents.detail.skill_meta", { defaultValue: "installed" })}
                  </div>
                </div>
                <Sparkles className="w-3.5 h-3.5 text-brand/70 shrink-0 mt-0.5" />
              </div>
            ))}
          </div>
        )}
      </div>
    );
  };

  // ---------- Schedule tab — trigger card + 14-run bar chart per design canvas
  const renderScheduleTab = (agent: AgentDetail) => {
    const cron = cronJobsQuery.data ?? [];
    // GET /api/agents/{id} doesn't embed triggers, so we hit the
    // dedicated /api/triggers?agent_id=... endpoint here. Falling back
    // to the (legacy) embedded-on-detail field if a future backend
    // version ships it.
    const liveTriggers = (agentTriggersQuery.data ?? []) as Array<{
      id?: string;
      pattern?: unknown;
      prompt_template?: string;
      enabled?: boolean;
    }>;
    const triggers: AgentTriggerSummary[] = liveTriggers.map((tr) => {
      // Render the trigger pattern compactly. The full TriggerPattern shape
      // is rich (event filters / regex / etc.); the detail panel only
      // needs a one-liner — full pattern lives on the dedicated page.
      const patternStr = (() => {
        if (!tr.pattern) return undefined;
        if (typeof tr.pattern === "string") return tr.pattern;
        try {
          return JSON.stringify(tr.pattern);
        } catch {
          return undefined;
        }
      })();
      return {
        event_pattern: patternStr,
        name: tr.id,
        description: tr.prompt_template,
      };
    });
    // Synthetic "last 14 runs" — backend doesn't expose per-fire history
    // through a single agent-scoped endpoint yet, so we visualise an
    // agent-id-seeded waveform as a placeholder. Wire up real run
    // telemetry in the next pass.
    const seed = agent.id.charCodeAt(0) || 1;
    const bars = Array.from({ length: 14 }, (_, i) =>
      Math.round(35 + Math.sin((i + seed) / 2.1) * 12 + i * 1.5),
    );
    const hasSchedule = cron.length > 0 || triggers.length > 0;
    return (
      <div className="flex flex-col gap-3">
        <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
          {t("agents.detail.trigger", { defaultValue: "Trigger" })}
        </div>
        {hasSchedule ? (
          <>
            {cron.map((c: CronJobItem & AgentCronSummary, i: number) => (
              <div
                key={`c-${i}`}
                className="px-3.5 py-3 rounded-lg border border-border-subtle bg-main/40 flex items-center gap-3"
              >
                <div className="w-8 h-8 rounded-md bg-accent/10 text-accent grid place-items-center shrink-0">
                  <Clock className="w-4 h-4" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="font-mono text-[13px] truncate text-text-main">
                    {c.schedule || c.cron || c.expression || "cron"}
                  </div>
                  <div className="text-[11px] text-text-dim/80 mt-0.5 truncate">
                    {c.next_run
                      ? `${t("agents.detail.next_run", { defaultValue: "Next run" })} · ${formatRelativeTime(c.next_run)}`
                      : c.name || c.id || "cron job"}
                  </div>
                </div>
                <Button variant="ghost" size="sm" onClick={() => navigate({ to: "/scheduler" })}>
                  {t("common.edit", { defaultValue: "Edit" })}
                </Button>
              </div>
            ))}
            {triggers.map((trig: AgentTriggerSummary, i: number) => (
              <div
                key={`t-${i}`}
                className="px-3.5 py-3 rounded-lg border border-border-subtle bg-main/40 flex items-center gap-3"
              >
                <div className="w-8 h-8 rounded-md bg-warning/10 text-warning grid place-items-center shrink-0">
                  <Zap className="w-4 h-4" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="font-mono text-[13px] truncate text-text-main">
                    {trig.event_pattern || trig.name || "event trigger"}
                  </div>
                  <div className="text-[11px] text-text-dim/80 mt-0.5 truncate">
                    {trig.description || t("agents.detail.event_driven", { defaultValue: "event-driven" })}
                  </div>
                </div>
              </div>
            ))}
          </>
        ) : (
          // Honest empty state — most agents are reactive (no cron, no
          // triggers) and the panel was previously rendering "Manual" as
          // if it were a misconfiguration. Surface the manifest's actual
          // schedule mode (also returned by GET /api/agents on list, but
          // we re-derive from the detail response defensively).
          (() => {
            const mode = (agent as AgentDetail).schedule;
            const isReactive = !mode || mode === "manual";
            return (
              <div className="rounded-md border border-border-subtle bg-main/40 p-4 flex items-start gap-3">
                <Zap className="w-4 h-4 text-brand/80 shrink-0 mt-0.5" />
                <div className="min-w-0 flex-1">
                  <div className="font-mono text-[12.5px] font-medium text-text-main">
                    {isReactive
                      ? t("agents.detail.schedule_reactive_title", { defaultValue: "Reactive" })
                      : mode}
                  </div>
                  <div className="font-mono text-[10.5px] text-text-dim/80 mt-0.5">
                    {isReactive
                      ? t("agents.detail.schedule_reactive_desc", {
                          defaultValue: "wakes on incoming messages and events — no cron or trigger pinned",
                        })
                      : t("agents.detail.schedule_no_triggers", {
                          defaultValue: "schedule mode set but no cron jobs or event triggers configured",
                        })}
                  </div>
                </div>
              </div>
            );
          })()
        )}

        <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim mt-2">
          {t("agents.detail.last_runs", { defaultValue: "Last 14 runs" })}
        </div>
        <div className="flex gap-[3px] items-end h-16 px-1">
          {bars.map((v, i) => {
            const isLast = i === bars.length - 1;
            return (
              <div
                key={i}
                className={`flex-1 rounded-t-[2px] ${isLast ? "bg-brand" : "bg-brand/40"}`}
                style={{
                  height: `${Math.max(8, Math.min(100, v))}%`,
                  boxShadow: isLast ? "0 0 8px rgba(56,189,248,0.6)" : "none",
                  minHeight: 6,
                }}
                aria-hidden="true"
              />
            );
          })}
        </div>
      </div>
    );
  };

  // ---------- Logs tab — terminal-style turn feed per design canvas
  // Sourced from /api/agents/{id}/events (usage_events) so each row is
  // a real LLM turn — model / latency / tokens / cost — instead of the
  // global audit ledger, which is mostly admin lifecycle entries.
  const renderLogsTab = (_agent: AgentDetail) => {
    const events = agentEventsQuery.data ?? [];
    const fmtTime = (s?: string): string => {
      if (!s) return "—";
      try {
        const d = new Date(s);
        const hh = String(d.getHours()).padStart(2, "0");
        const mm = String(d.getMinutes()).padStart(2, "0");
        const ss = String(d.getSeconds()).padStart(2, "0");
        const ms = String(d.getMilliseconds()).padStart(3, "0");
        return `${hh}:${mm}:${ss}.${ms}`;
      } catch {
        return s;
      }
    };
    // Token shorthand (1.2k tok) — keeps the line fitting the design.
    const fmtTokens = (n: number): string =>
      n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
    const formatLine = (e: typeof events[number]): string =>
      `turn · ${e.model} · in=${fmtTokens(e.input_tokens)} out=${fmtTokens(e.output_tokens)} · ${e.latency_ms}ms · $${e.cost_usd.toFixed(4)}`;
    return (
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
            {t("agents.detail.events_tail", { defaultValue: "events · tail" })} · {events.length}
          </div>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={<Copy className="h-3.5 w-3.5" />}
            onClick={() => {
              const text = events
                .map((e) => `${fmtTime(e.timestamp)} INFO ${e.provider || "—"} ${formatLine(e)}`)
                .join("\n");
              void navigator.clipboard?.writeText(text);
              addToast(t("common.copied", { defaultValue: "Copied" }), "success");
            }}
          >
            {t("common.copy", { defaultValue: "Copy" })}
          </Button>
        </div>
        {agentEventsQuery.isLoading ? (
          <div className="text-[12px] text-text-dim italic">{t("common.loading", { defaultValue: "Loading…" })}</div>
        ) : events.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_logs", { defaultValue: "No turns recorded yet for this agent." })}
          </div>
        ) : (
          <div
            className="rounded-md border border-border-subtle p-3 font-mono text-[11.5px] leading-[1.6] max-h-60 overflow-auto -mx-3 sm:mx-0"
            style={{ background: "rgba(2,6,23,0.6)" }}
          >
            {events.map((e, i) => (
              <div key={`${e.timestamp}-${i}`} className="flex gap-2.5 min-w-max sm:min-w-0">
                <span className="text-text-dim/60 shrink-0">{fmtTime(e.timestamp)}</span>
                <span className="text-success w-12 shrink-0">INFO</span>
                <span className="text-accent w-24 shrink-0 truncate">{e.provider || "agent"}</span>
                <span className="text-text-dim min-w-0 truncate" title={formatLine(e)}>
                  {formatLine(e)}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="flex flex-col gap-4 sm:gap-6 transition-colors duration-300">
      <div className="flex flex-col sm:flex-row justify-between items-start sm:items-end gap-3">
        <PageHeader
          badge={t("common.kernel_runtime")}
          title={t("agents.title")}
          subtitle={t("agents.subtitle")}
          isFetching={agentsQuery.isFetching}
          onRefresh={() => void agentsQuery.refetch()}
          icon={<Users className="h-4 w-4" />}
          helpText={t("agents.help")}
        />
        <Button variant="primary" onClick={() => setShowCreate(true)} className="shrink-0" title={t("agents.create_agent") + " (n)"}>
          <Plus className="w-4 h-4" />
          <span>{t("agents.create_agent")}</span>
          <kbd className="hidden sm:inline-flex h-5 min-w-[20px] items-center justify-center rounded border border-white/30 bg-white/10 px-1 text-[9px] font-mono font-semibold">n</kbd>
        </Button>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-[360px_1fr] gap-4 lg:min-h-[640px]">
        {/* Left list panel — search + filter pills + sort + scroll body.
            On mobile the list owns the viewport; the detail panel is
            promoted to a full-screen overlay (see below) so we never
            stack two scrollers on a 390px phone. */}
        <Card
          padding="none"
          className={`surface-lit overflow-hidden flex flex-col h-[calc(100vh-200px)] min-h-[480px] ${detailAgent ? "hidden lg:flex" : "flex"}`}
        >
          <div className="px-3 pt-3 pb-2.5 border-b border-border-subtle flex flex-col gap-2 flex-shrink-0">
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("common.search")}
              leftIcon={<Search className="h-4 w-4" />}
              data-shortcut-search
            />
            <div className="flex flex-wrap gap-1.5 items-center">
              {(["all", "running", "suspended"] as const).map((key) => {
                const isActive = stateFilter === key;
                const count = agentCounts[key];
                const label = t(`agents.filter_${key}`, {
                  defaultValue: key === "all" ? "All" : key === "running" ? "Running" : "Suspended",
                });
                return (
                  <button
                    key={key}
                    onClick={() => setStateFilter(key)}
                    className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10.5px] font-semibold transition-colors ${
                      isActive
                        ? "border-brand/30 bg-brand/10 text-brand"
                        : "border-border-subtle bg-surface text-text-dim hover:border-brand/20 hover:text-brand"
                    }`}
                  >
                    <span>{label}</span>
                    <span
                      className={`inline-flex items-center justify-center rounded-full px-1 min-w-[16px] h-[14px] text-[9px] font-mono ${
                        isActive ? "bg-brand/20" : "bg-main"
                      }`}
                    >
                      {count}
                    </span>
                  </button>
                );
              })}
              <button
                onClick={() => setShowHandAgents((value) => !value)}
                aria-pressed={showHandAgents}
                className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[10.5px] font-semibold transition-colors ${
                  showHandAgents
                    ? "border-brand/30 bg-brand/10 text-brand"
                    : "border-border-subtle bg-surface text-text-dim hover:border-brand/20 hover:text-brand"
                }`}
              >
                {t("agents.show_hand_agents", { defaultValue: "Hand" })}
              </button>
              <select
                value={sortBy}
                onChange={(e) => setSortBy(e.target.value as typeof sortBy)}
                className="ml-auto rounded-full border border-border-subtle bg-surface px-2 py-0.5 text-[10.5px] font-semibold text-text-dim outline-none focus:border-brand hover:border-brand/20 cursor-pointer"
                aria-label={t("common.sort_by", { defaultValue: "Sort by" })}
              >
                <option value="name">{t("common.sort_name", { defaultValue: "Name" })}</option>
                <option value="last_active">{t("common.sort_last_active", { defaultValue: "Active" })}</option>
                <option value="created_at">{t("common.sort_created", { defaultValue: "Created" })}</option>
              </select>
            </div>
          </div>
          <div className="flex-1 overflow-y-auto">
            {agentsQuery.isLoading ? (
              <div className="p-3 flex flex-col gap-2">
                {[1, 2, 3, 4, 5].map((i) => <CardSkeleton key={i} />)}
              </div>
            ) : filteredAgents.length === 0 ? (
              search || stateFilter !== "all" || showHandAgents ? (
                <EmptyState
                  title={t("agents.no_matching")}
                  icon={<Search className="h-6 w-6" />}
                  action={
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => {
                        setSearch("");
                        setStateFilter("all");
                        setShowHandAgents(false);
                      }}
                    >
                      {t("common.clear_filters", { defaultValue: "Clear filters" })}
                    </Button>
                  }
                />
              ) : (
                <EmptyState title={t("common.no_data")} icon={<Users className="h-6 w-6" />} />
              )
            ) : (
              <div className="flex flex-col">
                {coreAgents.map((agent) => renderAgentRow(agent))}
              </div>
            )}
          </div>
        </Card>

        {/* Right detail panel — header + KPI tiles + 5 tabs.
            Mobile: rendered as a fixed full-viewport overlay above the
            list (top inset 0, bottom inset 14 reserves the global tab
            bar's ~56px so it never gets covered). lg+: collapses back
            into the master-detail grid. */}
        {detailAgent ? (
          <div className="fixed inset-x-0 top-0 bottom-[calc(56px+env(safe-area-inset-bottom))] z-30 bg-surface lg:static lg:inset-auto lg:bottom-auto lg:z-auto lg:bg-transparent overflow-hidden flex flex-col">
            {renderDetailPanel(detailAgent)}
          </div>
        ) : (
          // Placeholder is desktop-only — on mobile the list fills the
          // viewport when no agent is selected, so this empty-state would
          // just be wasted vertical space.
          <Card padding="lg" className="surface-lit hidden lg:grid place-items-center text-center min-h-[480px]">
            <div className="max-w-xs">
              <div className="w-12 h-12 mx-auto rounded-xl bg-brand/10 border border-brand/30 grid place-items-center text-brand mb-3">
                <Bot className="w-6 h-6" />
              </div>
              <h3 className="text-sm font-semibold text-text-main mb-1">
                {t("agents.select_an_agent", { defaultValue: "Select an agent" })}
              </h3>
              <p className="text-xs text-text-dim">
                {t("agents.select_an_agent_hint", {
                  defaultValue: "Choose an agent on the left to inspect its sessions, memory, skills, and live logs.",
                })}
              </p>
            </div>
          </Card>
        )}
      </div>
      {/* Agent Detail Drawer. Right-side inspector pattern (Linear / Figma):
          the agents list stays interactive while the drawer is open, so
          clicking another agent in the list updates the drawer's content
          in place — no close-then-reopen needed. Sticky header / footer
          keep identity and primary actions pinned while the inspectable
          sections scroll in the middle. */}
      {detailAgent && detailDrawerOpen && (() => {
        const detailState = ((detailAgent as AgentView).state || "").toLowerCase();
        const isDetailSuspended = detailState === "suspended";
        const isDetailCrashed = detailState === "crashed";
        const statusColor = isDetailSuspended ? "bg-warning" : isDetailCrashed ? "bg-error" : "bg-success";
        // Only hand-managed sub-agents are locked from rename — their names
        // are referenced by the parent hand and changing them would orphan
        // the parent's call sites. Built-in templates (`assistant`, etc.)
        // ARE renameable: a fresh agent spawned from a template is a regular
        // user-owned agent.
        const lockRename = !!detailAgent.is_hand;
        // Pick the config mutation that matches this agent's role; mirrors
        // the branching in saveModelEdit / web-search toggle below.
        const activeConfigMutation = detailAgent.is_hand
          ? patchHandAgentRuntimeConfigMutation
          : patchAgentConfigMutation;
        const saveModelDisabled =
          activeConfigMutation.isPending
          || !modelDraft.provider.trim()
          || !modelDraft.model.trim()
          || isNaN(parseInt(modelDraft.max_tokens, 10))
          || parseInt(modelDraft.max_tokens, 10) <= 0
          || isNaN(parseFloat(modelDraft.temperature))
          || parseFloat(modelDraft.temperature) < 0
          || parseFloat(modelDraft.temperature) > 2;
        return (
        <DrawerPanel
          isOpen
          onClose={closeDetailModal}
          size="xl"
          hideCloseButton
        >
            {/* Header — sticky, identity + state. */}
            <div className="px-6 py-4 border-b border-border-subtle sticky top-0 bg-surface z-10">
              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-3 min-w-0 flex-1">
                  <div className="relative shrink-0">
                    <Avatar fallback={detailAgent.name} size="lg" />
                    <span className={`absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full ${statusColor} border-2 border-surface ${!isDetailSuspended && !isDetailCrashed ? "animate-pulse" : ""}`} role="img" aria-label={isDetailSuspended ? "Agent suspended" : isDetailCrashed ? "Agent crashed" : "Agent active"} />
                  </div>
                  <div className="min-w-0 flex-1">
                    {editingName ? (
                      <div className="flex items-center gap-2">
                        <input
                          type="text"
                          autoFocus
                          value={nameDraft}
                          onChange={e => setNameDraft(e.target.value)}
                          onKeyDown={e => {
                            // `isComposing` guard: in CJK IMEs Enter
                            // confirms the candidate (pinyin → hanzi);
                            // submitting on it would hijack composition.
                            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                              saveName();
                            } else if (e.key === "Escape") {
                              // stopPropagation so Escape cancels the inline
                              // edit only — the Modal's window Escape listener
                              // would otherwise close the whole modal too.
                              e.stopPropagation();
                              cancelNameEdit();
                            }
                          }}
                          className="px-2 py-1 rounded-lg border border-brand bg-main text-base font-bold outline-none focus:ring-2 focus:ring-brand/30 min-w-0 flex-1"
                          aria-label={t("agents.edit_name", { defaultValue: "Agent name" })}
                          maxLength={64}
                        />
                        <button
                          onClick={saveName}
                          disabled={patchAgentMutation.isPending || !nameDraft.trim() || nameDraft.trim() === detailAgent.name}
                          className="px-3 py-1 rounded-lg text-xs font-semibold bg-brand text-white hover:bg-brand/90 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
                        >
                          {patchAgentMutation.isPending ? t("common.saving") : t("common.save")}
                        </button>
                        <button
                          onClick={cancelNameEdit}
                          className="px-3 py-1 rounded-lg text-xs font-semibold bg-main hover:bg-main/80 text-text-dim border border-border-subtle shrink-0"
                        >
                          {t("common.cancel")}
                        </button>
                      </div>
                    ) : (
                      <button
                        type="button"
                        onClick={lockRename ? undefined : startNameEdit}
                        disabled={lockRename}
                        className={`group inline-flex items-center gap-2 max-w-full ${lockRename ? "cursor-default" : "cursor-text hover:text-brand transition-colors"}`}
                        title={lockRename
                          ? t("agents.rename_hand_disabled", { defaultValue: "Hand-managed agents cannot be renamed" })
                          : t("agents.rename_hint", { defaultValue: "Click to rename" })}
                      >
                        <h3 className="text-base font-bold truncate">
                          {t(`agents.builtin.${detailAgent.name}.name`, { defaultValue: detailAgent.name })}
                        </h3>
                        {!lockRename && (
                          <Pencil className="w-3.5 h-3.5 text-text-dim opacity-0 group-hover:opacity-100 transition-opacity shrink-0" />
                        )}
                      </button>
                    )}
                    {(detailAgent as AgentView).description && (
                      <p className="text-xs text-text-dim mt-1 leading-relaxed">{(detailAgent as AgentView).description}</p>
                    )}
                    <div className="flex items-center gap-2 mt-1.5 flex-wrap">
                      <span className="text-[11px] text-text-dim/70 font-mono">{truncateId(detailAgent.id, 16)}</span>
                      {detailAgent.is_hand && <Badge variant="info">{t("agents.hand_badge", { defaultValue: "HAND" })}</Badge>}
                      <Badge variant={isDetailSuspended ? "warning" : isDetailCrashed ? "error" : "success"} dot>
                        {(detailAgent as AgentView).state ? t(`common.${detailState}`, { defaultValue: (detailAgent as AgentView).state }) : t("common.running")}
                      </Badge>
                    </div>
                  </div>
                </div>
                <button onClick={closeDetailModal} className="p-2 rounded-lg hover:bg-main transition-colors shrink-0" aria-label={t("common.close", { defaultValue: "Close" })}>
                  <X className="w-4 h-4" />
                </button>
              </div>
            </div>
            {/* Body — scrollable inspectable sections. */}
            <div className="px-6 py-5 space-y-5">

              {/* Model */}
              {detailAgent.model && (
                <section>
                  <div className="flex items-center justify-between mb-2">
                    <h4 className="text-sm font-semibold flex items-center gap-2">
                      <Cpu className="w-3.5 h-3.5 text-brand" />
                      {t("agents.model")}
                    </h4>
                    {!editingModel && (
                      <button
                        onClick={startModelEdit}
                        className="text-xs text-brand hover:underline font-medium"
                      >
                        {t("common.edit")}
                      </button>
                    )}
                  </div>
                  <div className="rounded-lg bg-main border border-border-subtle p-4 space-y-2">
                    {editingModel ? (
                      <>
                        <DetailRow label={t("agents.provider")}>
                          <select
                            value={modelDraft.provider}
                            onChange={e => setModelDraft(d => ({ ...d, provider: e.target.value, model: "" }))}
                            className="w-44 px-2 py-1 rounded-md border border-border-subtle bg-surface text-sm font-mono outline-none focus:border-brand text-right"
                            disabled={providersQuery.isLoading}
                          >
                            {providersQuery.isLoading && <option value="">Loading...</option>}
                            {providersQuery.error && <option value="">Error loading</option>}
                            {!providersQuery.isLoading && configuredProviders.length === 0 && <option value="">No providers</option>}
                            {modelDraft.provider && !configuredProviders.some(p => p.id === modelDraft.provider) && (
                              <option value={modelDraft.provider}>{modelDraft.provider}</option>
                            )}
                            {configuredProviders.map(p => (
                              <option key={p.id} value={p.id}>{p.display_name || p.id}</option>
                            ))}
                          </select>
                        </DetailRow>
                        <DetailRow label={t("agents.model")}>
                          <select
                            value={modelDraft.model}
                            onChange={e => setModelDraft(d => ({ ...d, model: e.target.value }))}
                            className="w-44 px-2 py-1 rounded-md border border-border-subtle bg-surface text-sm font-mono outline-none focus:border-brand text-right"
                            disabled={modelsQuery.isLoading || !modelDraft.provider.trim()}
                          >
                            {!modelDraft.provider.trim() && <option value="">Select provider first</option>}
                            {modelDraft.provider.trim() && modelsQuery.isLoading && <option value="">Loading...</option>}
                            {modelDraft.provider.trim() && !modelsQuery.isLoading && visibleModels.length === 0 && <option value="">No models</option>}
                            {modelDraft.model && !visibleModels.some(m => m.id === modelDraft.model) && (
                              <option value={modelDraft.model}>{modelDraft.model}</option>
                            )}
                            {visibleModels.map(m => (
                              <option key={m.id} value={m.id}>{m.display_name || m.id}</option>
                            ))}
                          </select>
                        </DetailRow>
                        <DetailRow label={t("agents.max_tokens")}>
                          <input
                            type="number"
                            min={1}
                            max={200000}
                            value={modelDraft.max_tokens}
                            onChange={e => setModelDraft(d => ({ ...d, max_tokens: e.target.value }))}
                            className="w-44 px-2 py-1 rounded-md border border-border-subtle bg-surface text-sm font-mono outline-none focus:border-brand text-right"
                          />
                        </DetailRow>
                        <DetailRow label={t("agents.temperature")}>
                          <input
                            type="number"
                            min={0}
                            max={2}
                            step={0.1}
                            value={modelDraft.temperature}
                            onChange={e => setModelDraft(d => ({ ...d, temperature: e.target.value }))}
                            className="w-44 px-2 py-1 rounded-md border border-border-subtle bg-surface text-sm font-mono outline-none focus:border-brand text-right"
                          />
                        </DetailRow>
                        <div className="flex justify-end gap-2 pt-1">
                          <button
                            onClick={cancelModelEdit}
                            className="px-3 py-1 rounded-md text-xs font-semibold bg-main hover:bg-main/80 text-text-dim border border-border-subtle"
                          >
                            {t("common.cancel")}
                          </button>
                          <button
                            onClick={saveModelEdit}
                            disabled={saveModelDisabled}
                            className="px-3 py-1 rounded-md text-xs font-semibold bg-brand hover:bg-brand/90 text-white disabled:opacity-50"
                          >
                            {activeConfigMutation.isPending ? t("common.saving") : t("common.save")}
                          </button>
                        </div>
                      </>
                    ) : (
                      <>
                        <DetailRow label={t("agents.provider")}>
                          <span className="font-mono text-brand">{detailAgent.model.provider}</span>
                        </DetailRow>
                        <DetailRow label={t("agents.model")}>
                          <span className="font-mono">{detailAgent.model.model}</span>
                        </DetailRow>
                        <DetailRow label={t("agents.max_tokens")}>
                          <span className="font-mono">{(detailAgent.model.max_tokens ?? 4096).toLocaleString()}</span>
                        </DetailRow>
                        {detailAgent.model.temperature != null && (
                          <DetailRow label={t("agents.temperature")}>
                            <span className="font-mono">{detailAgent.model.temperature}</span>
                          </DetailRow>
                        )}
                      </>
                    )}
                  </div>
                </section>
              )}

              {/* Web Search Augmentation */}
              <section>
                <h4 className="text-sm font-semibold mb-2">
                  {t("agents.web_search", { defaultValue: "Web Search" })}
                </h4>
                <div className="rounded-lg bg-main border border-border-subtle p-4">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="text-sm">{t("agents.web_search_augmentation", { defaultValue: "Search Augmentation" })}</p>
                      <p className="text-xs text-text-dim mt-0.5 leading-relaxed">
                        {t("agents.web_search_augmentation_hint", { defaultValue: "Auto-search the web and inject results into context before LLM call" })}
                      </p>
                    </div>
                    <select
                      value={detailAgent.web_search_augmentation || "off"}
                      onChange={e => {
                        const mode = e.target.value as "off" | "auto" | "always";
                        // Branch in the caller, not the hook — only the
                        // caller knows from the cached detail whether this
                        // agent is a hand role.
                        const mutation = detailAgent.is_hand
                          ? patchHandAgentRuntimeConfigMutation
                          : patchAgentConfigMutation;
                        mutation.mutate(
                          { agentId: detailAgent.id, config: { web_search_augmentation: mode } },
                          {
                            onSuccess: async () => {
                              await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                            },
                          },
                        );
                      }}
                      className="w-28 px-2 py-1 rounded-md border border-border-subtle bg-surface text-sm font-mono outline-none focus:border-brand text-right shrink-0"
                    >
                      <option value="off">{t("common.off", { defaultValue: "Off" })}</option>
                      <option value="auto">{t("common.auto", { defaultValue: "Auto" })}</option>
                      <option value="always">{t("common.always", { defaultValue: "Always" })}</option>
                    </select>
                  </div>
                </div>
              </section>

              {/* Capabilities */}
              {detailAgent.capabilities && (
                <section>
                  <h4 className="text-sm font-semibold mb-2 flex items-center gap-2">
                    <Wrench className="w-3.5 h-3.5 text-success" />
                    {t("agents.capabilities")}
                  </h4>
                  <div className="flex flex-wrap gap-2">
                    {detailAgent.capabilities.tools && (
                      <button
                        type="button"
                        className="inline-flex"
                        aria-label={t("agents.tools_edit_aria", { defaultValue: "Edit tools" })}
                        onClick={(e: React.MouseEvent) => {
                          e.stopPropagation();
                          setToolsEditorAgentId(detailAgent.id);
                          setShowToolsEditor(true);
                        }}
                      >
                        <Badge variant="brand" dot className="hover:bg-brand/20 transition-colors">
                          {`${t("agents.tools_cap")} ✎`}
                        </Badge>
                      </button>
                    )}
                    {detailAgent.capabilities.network && <Badge variant="brand" dot>{t("agents.network")}</Badge>}
                  </div>
                </section>
              )}

              {/* System Prompt — collapsible */}
              {detailAgent.system_prompt && (
                <SystemPromptSection prompt={detailAgent.system_prompt} />
              )}

              {/* Skills */}
              {detailAgent.skills && detailAgent.skills.length > 0 && (
                <section>
                  <h4 className="text-sm font-semibold mb-2">{t("agents.skills")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.skills.map((s: string, i: number) => (
                      <Badge key={i} variant="default">{s}</Badge>
                    ))}
                  </div>
                </section>
              )}

              {/* Tags */}
              {detailAgent.tags && detailAgent.tags.length > 0 && (
                <section>
                  <h4 className="text-sm font-semibold mb-2">{t("agents.tags")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.tags.map((tag: string, i: number) => (
                      <span
                        key={i}
                        className="text-xs px-2.5 py-1 rounded-md bg-main border border-border-subtle text-text-dim"
                      >
                        {tag}
                      </span>
                    ))}
                  </div>
                </section>
              )}

              {/* Mode */}
              {detailAgent.mode && (
                <section className="flex items-center gap-3 rounded-lg bg-main border border-border-subtle px-4 py-3">
                  <Shield className="w-4 h-4 text-warning shrink-0" />
                  <span className="text-sm font-semibold flex-1">{t("agents.mode")}</span>
                  <Badge variant="warning">{detailAgent.mode}</Badge>
                </section>
              )}

              {/* Thinking / Extended Reasoning */}
              {detailAgent.thinking && (
                <section>
                  <h4 className="text-sm font-semibold mb-2 flex items-center gap-2">
                    <Brain className="w-3.5 h-3.5 text-purple-500" />
                    {t("agents.thinking")}
                  </h4>
                  <div className="rounded-lg bg-main border border-border-subtle p-4 space-y-2">
                    <DetailRow label={t("agents.thinking_enabled")}>
                      <Badge variant={(detailAgent.thinking.budget_tokens ?? 0) > 0 ? "success" : "default"}>
                        {(detailAgent.thinking.budget_tokens ?? 0) > 0 ? t("common.yes") : t("common.no")}
                      </Badge>
                    </DetailRow>
                    <DetailRow label={t("agents.budget_tokens")}>
                      <span className="font-mono">{detailAgent.thinking.budget_tokens?.toLocaleString() ?? 0}</span>
                    </DetailRow>
                    <DetailRow label={t("agents.stream_thinking")}>
                      <Badge variant={detailAgent.thinking.stream_thinking ? "brand" : "default"}>
                        {detailAgent.thinking.stream_thinking ? t("common.yes") : t("common.no")}
                      </Badge>
                    </DetailRow>
                    <p className="text-xs text-text-dim flex items-center gap-1.5 pt-1">
                      <Zap className="w-3 h-3" />
                      {t("agents.thinking_hint")}
                    </p>
                  </div>
                </section>
              )}
            </div>

            {/* Footer — sticky, primary + secondary actions reachable on long specs. */}
            <div className="sticky bottom-0 px-6 py-4 border-t border-border-subtle bg-surface space-y-2.5">
              <Button
                variant="primary"
                size="md"
                className="w-full"
                onClick={() => { closeDetailModal(); navigate({ to: "/chat", search: { agentId: detailAgent.id } }); }}
              >
                <MessageCircle className="w-4 h-4 mr-2" />
                {t("common.interact")}
              </Button>

              <div className="flex flex-wrap gap-2">
                {isDetailSuspended ? (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="flex-1 min-w-[88px]"
                    onClick={async () => {
                      try {
                        await resumeMutation.mutateAsync(detailAgent.id);
                        await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                      } catch (err) {
                        addToast(toastErr(err, t("agents.resume_failed", { defaultValue: "Failed to resume agent" })), "error");
                      }
                    }}
                  >
                    <Play className="w-3.5 h-3.5 mr-1.5" />
                    {t("agents.resume")}
                  </Button>
                ) : (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="flex-1 min-w-[88px]"
                    onClick={async () => {
                      try {
                        await suspendMutation.mutateAsync(detailAgent.id);
                        await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                      } catch (err) {
                        addToast(toastErr(err, t("agents.suspend_failed", { defaultValue: "Failed to suspend agent" })), "error");
                      }
                    }}
                  >
                    <Pause className="w-3.5 h-3.5 mr-1.5" />
                    {t("agents.suspend")}
                  </Button>
                )}
                <Button
                  variant="secondary"
                  size="sm"
                  className="flex-1 min-w-[88px]"
                  onClick={async () => {
                    try {
                      await cloneMutation.mutateAsync(detailAgent.id);
                    } catch (err) {
                      addToast(toastErr(err, t("agents.clone_failed", { defaultValue: "Failed to clone agent" })), "error");
                    }
                  }}
                >
                  <Copy className="w-3.5 h-3.5 mr-1.5" />
                  {t("agents.clone")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  className="flex-1 min-w-[88px]"
                  onClick={() =>
                    setConfirmDialog({
                      title: t("agents.reset_title", { defaultValue: "Reset session?" }),
                      message: t("agents.reset_confirm"),
                      onConfirm: async () => {
                        await resetAgentSession(detailAgent.id);
                        await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                      },
                    })
                  }
                >
                  <RotateCcw className="w-3.5 h-3.5 mr-1.5" />
                  {t("agents.reset")}
                </Button>
                {!detailAgent.is_hand && (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="flex-1 min-w-[88px] text-error/80 hover:text-error"
                    onClick={() =>
                      setConfirmDialog({
                        title: t("agents.delete_title", { defaultValue: "Delete agent?" }),
                        message: t("agents.delete_confirm", { name: detailAgent.name }),
                        tone: "destructive",
                        onConfirm: () => deleteMutation.mutate(detailAgent.id),
                      })
                    }
                  >
                    <Trash2 className="w-3.5 h-3.5 mr-1.5" />
                    {t("common.delete")}
                  </Button>
                )}
              </div>

              <Button
                variant="secondary"
                size="sm"
                className="w-full"
                onClick={() => setShowPrompts(true)}
              >
                <FlaskConical className="w-3.5 h-3.5 mr-1.5" />
                {t("agents.prompts")}
              </Button>
            </div>
        </DrawerPanel>
        );
      })()}

      {/* Tools Editor Modal */}
      {showToolsEditor && toolsEditorAgentId && (
        <DrawerPanel isOpen={showToolsEditor} onClose={closeToolsEditor} title={t("agents.tools_editor_title", { defaultValue: "Agent Tools" })} size="lg">
          <div className="p-6 space-y-5">
            <div>
              <p className="text-[11px] text-text-dim/70">
                {t("agents.tools_editor_desc", { defaultValue: "Review and manage the agent's tools. Declared tools are the primary set; allowlist/blocklist are additional filters." })}
              </p>
              {!toolsEditorLoading && (
                <p className="mt-2 text-[10px] text-text-dim/50 font-mono">
                  {capabilitiesToolsDraft.length} {t("agents.tools_declared_count", { defaultValue: "declared" })} · {availableToolNames.length} {t("agents.tools_available", { defaultValue: "tools available" })} · {toolAllowlistDraft.length} {t("agents.tools_allowed_count", { defaultValue: "allowed" })} · {toolBlocklistDraft.length} {t("agents.tools_blocked_count", { defaultValue: "blocked" })}
                </p>
              )}
            </div>

            {toolsEditorLoading ? (
              <div className="flex items-center gap-2 text-xs text-text-dim py-8 justify-center">
                <Loader2 className="w-4 h-4 animate-spin" /> {t("common.loading")}
              </div>
            ) : (
              <>
                <div className="rounded-xl border border-border-subtle bg-main/40 px-4 py-3">
                  <div>
                    <div className="text-sm font-bold text-text">{t("agents.tools_disabled_label", { defaultValue: "Disable all tools" })}</div>
                    <p className="mt-1 text-[11px] text-text-dim/70">
                      {toolsDisabledState
                        ? t("agents.tools_disabled_hint_active", { defaultValue: "Tools are disabled for this agent; editing allow/block filters is blocked here. Re-enable tools in the agent config to manage filters." })
                        : t("agents.tools_disabled_hint", { defaultValue: "Tools are currently enabled. Allowlist and blocklist below control which tools remain available." })}
                    </p>
                  </div>
                </div>

                {toolsDisabledState && (
                  <div className="rounded-xl border border-warning/30 bg-warning/10 px-4 py-3 text-[11px] text-warning">
                    {t("agents.tools_disabled_save_blocked", { defaultValue: "All tools are disabled for this agent. To re-enable tools, edit the agent manifest or config directly — this editor only manages allow/block filters." })}
                  </div>
                )}

                <div className="space-y-2">
                  <div>
                    <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-2">
                      {t("agents.tools_declared_title", { defaultValue: "Declared Tools" })}
                    </h4>
                    <p className="text-[11px] text-text-dim/70 mb-3">
                      {t("agents.tools_declared_desc", { defaultValue: "Tools this agent can use. Leave empty for unrestricted access to all tools." })}
                    </p>
                  </div>
                  <MultiSelectCmdk
                    options={availableToolNames}
                    value={capabilitiesToolsDraft}
                    onChange={setCapabilitiesToolsDraft}
                    placeholder={t("agents.tools_search_placeholder", { defaultValue: "Search tools..." })}
                    disabled={toolsDisabledState}
                  />
                </div>

                <div className="space-y-2">
                  <div>
                    <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-2">
                      {t("agents.tools_allowlist_title", { defaultValue: "Allowlist" })}
                    </h4>
                    <p className="text-[11px] text-text-dim/70 mb-3">
                      {t("agents.tools_allowlist_desc", { defaultValue: "Additional filter: only these tools remain available. Leave empty to skip this filter." })}
                    </p>
                  </div>
                  <MultiSelectCmdk
                    options={availableToolNames}
                    value={toolAllowlistDraft}
                    onChange={setToolAllowlistDraft}
                    placeholder={t("agents.tools_search_placeholder", { defaultValue: "Search tools..." })}
                    disabled={toolsDisabledState}
                  />
                </div>

                <div className="space-y-2">
                  <div>
                    <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-2">
                      {t("agents.tools_blocklist_title", { defaultValue: "Blocklist" })}
                    </h4>
                    <p className="text-[11px] text-text-dim/70 mb-3">
                      {t("agents.tools_blocklist_desc", { defaultValue: "These tools are blocked even if they are present in the allowlist." })}
                    </p>
                  </div>
                  <MultiSelectCmdk
                    options={availableToolNames}
                    value={toolBlocklistDraft}
                    onChange={setToolBlocklistDraft}
                    placeholder={t("agents.tools_search_placeholder", { defaultValue: "Search tools..." })}
                    disabled={toolsDisabledState}
                  />
                </div>

                {conflictingToolNames.length > 0 && (
                  <div className="rounded-xl border border-warning/30 bg-warning/10 px-4 py-3 text-[11px] text-warning">
                    {t("agents.tools_conflict_warning", {
                      defaultValue: "{{count}} tools are in both lists. Blocklist wins and those tools will be removed from the allowlist when you save.",
                      count: conflictingToolNames.length,
                    })}
                  </div>
                )}
              </>
            )}

            <div className="flex flex-col gap-2 pt-2">
              {toolsDisabledState && (
                <p className="text-center text-[10px] text-text-dim/50">
                  {t("agents.tools_disabled_save_hint", { defaultValue: "Re-enable tools in the agent config to modify filters" })}
                </p>
              )}
              <div className="flex gap-2">
              <Button variant="primary" size="sm" className="flex-1" disabled={toolsEditorLoading || toolsEditorSaving || toolsDisabledState} onClick={async () => {
                if (!toolsEditorAgentId) return;
                setToolsEditorSaving(true);
                try {
                  const resolvedAllowlist = toolAllowlistDraft.filter((name) => !toolBlocklistDraft.includes(name));
                  await updateAgentTools(toolsEditorAgentId, {
                    capabilities_tools: capabilitiesToolsDraft,
                    tool_allowlist: resolvedAllowlist,
                    tool_blocklist: toolBlocklistDraft,
                  });
                  addToast(
                    conflictingToolNames.length > 0
                      ? t("agents.tools_saved_conflicts", { defaultValue: "Tools updated. Conflicts were resolved in favor of the blocklist." })
                      : t("agents.tools_saved", { defaultValue: "Tools updated" }),
                    "success",
                  );
                  qc.invalidateQueries({ queryKey: agentQueries.detail(toolsEditorAgentId).queryKey });
                  if (detailAgent?.id === toolsEditorAgentId) {
                    void refreshDetailAgent(toolsEditorAgentId);
                  }
                  closeToolsEditor();
                } catch (err) {
                  addToast(toastErr(err, t("agents.tools_save_failed", { defaultValue: "Failed to update tools" })), "error");
                } finally {
                  setToolsEditorSaving(false);
                }
              }}>
                {toolsEditorSaving ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : null}
                {toolsEditorSaving ? t("common.saving") : t("common.save")}
              </Button>
              <Button variant="secondary" size="sm" onClick={closeToolsEditor}>
                {t("common.cancel")}
              </Button>
              </div>
            </div>
          </div>
        </DrawerPanel>
      )}

      {/* Create Agent Modal */}
      <DrawerPanel
        isOpen={showCreate}
        onClose={closeCreateModal}
        title={t("agents.create_agent")}
        size="2xl"
      >
        <div className="p-5 space-y-4">
          {/* Mode tabs — switching between Form and TOML round-trips the
              manifest in both directions. We only re-parse when content
              actually differs, so re-clicking the same tab is a no-op. */}
          <div className="flex gap-2">
            <button onClick={() => switchCreateMode("form")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "form" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_form")}
            </button>
            <button onClick={() => switchCreateMode("template")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "template" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_template")}
            </button>
            <button onClick={() => switchCreateMode("toml")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "toml" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_toml")}
            </button>
          </div>
          {tomlParseError && (
            // The error is set when leaving TOML→Form fails, so the user
            // is bounced back to TOML; the message must show on the TOML
            // tab too, otherwise the rejected switch is invisible.
            <p className="text-xs text-error">
              {t("agents.form.toml_parse_error", { msg: tomlParseError })}
            </p>
          )}

          {createMode === "form" ? (
            <div className="grid grid-cols-1 lg:grid-cols-[1fr_360px] gap-4 max-h-[60vh] overflow-y-auto pr-1">
              <AgentManifestForm
                value={formState}
                onChange={setFormState}
                providers={formProviderOptions}
                models={formModelOptions}
                invalidFields={formErrors}
                extras={formExtras}
              />
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <div className="flex gap-1">
                    <button
                      type="button"
                      onClick={() => setPreviewTab("toml")}
                      className={`text-[10px] font-bold uppercase px-2 py-1 rounded ${
                        previewTab === "toml"
                          ? "bg-brand text-white"
                          : "text-text-dim hover:text-text"
                      }`}
                    >
                      {t("agents.form.preview_toml")}
                    </button>
                    <button
                      type="button"
                      onClick={() => setPreviewTab("markdown")}
                      className={`text-[10px] font-bold uppercase px-2 py-1 rounded ${
                        previewTab === "markdown"
                          ? "bg-brand text-white"
                          : "text-text-dim hover:text-text"
                      }`}
                    >
                      {t("agents.form.preview_markdown")}
                    </button>
                  </div>
                  <div className="flex gap-2 items-center">
                    <button
                      type="button"
                      onClick={() => {
                        const text =
                          previewTab === "toml" ? serializedFormToml : serializedFormMarkdown;
                        void navigator.clipboard.writeText(text).then(() =>
                          addToast(t("agents.form.copied"), "success"),
                        );
                      }}
                      className="text-[10px] font-bold text-text-dim hover:text-brand"
                      title={t("agents.form.copy")}
                    >
                      <Copy className="w-3.5 h-3.5" />
                    </button>
                    {previewTab === "toml" && (
                      <button
                        type="button"
                        onClick={() => switchCreateMode("toml")}
                        className="text-[10px] font-bold text-brand hover:underline"
                      >
                        {t("agents.form.switch_to_toml")}
                      </button>
                    )}
                  </div>
                </div>
                <pre className="rounded-xl border border-border-subtle bg-main px-3 py-2 text-[11px] font-mono text-text-dim overflow-auto max-h-[55vh] whitespace-pre-wrap break-all">
                  {previewTab === "toml" ? serializedFormToml : serializedFormMarkdown}
                </pre>
              </div>
            </div>
          ) : createMode === "template" ? (
            <div className="space-y-3">
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.template_name")}</label>
                <select value={templateName}
                  onChange={e => setTemplateName(e.target.value)}
                  className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand">
                  <option value="">{t("agents.template_placeholder")}</option>
                  {localizedTemplates.map(tmpl => (
                    <option key={tmpl.name} value={tmpl.name}>{tmpl.displayName}</option>
                  ))}
                </select>
                {selectedTemplate && (
                  <div className="mt-2 rounded-xl border border-border-subtle/60 bg-surface/60 px-3 py-2">
                    <p className="text-xs font-bold text-text">{selectedTemplate.displayName}</p>
                    <p className="mt-1 text-[11px] leading-relaxed text-text-dim">{selectedTemplate.displayDescription}</p>
                  </div>
                )}
              </div>
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">
                  {t("agents.template_custom_name", { defaultValue: "Agent Name (optional)" })}
                </label>
                <input
                  type="text"
                  value={templateCustomName}
                  onChange={e => setTemplateCustomName(e.target.value)}
                  placeholder={
                    selectedTemplate?.name ??
                    t("agents.template_custom_name_placeholder", {
                      defaultValue: "Leave blank to use template default",
                    })
                  }
                  className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand"
                />
                <p className="text-[10px] text-text-dim mt-1">
                  {t("agents.template_custom_name_hint", {
                    defaultValue: "Override the template's default name so you can run multiple agents from the same template.",
                  })}
                </p>
              </div>
              <button
                type="button"
                disabled={!templateName || templateTomlLoading}
                onClick={async () => {
                  if (!templateName) return;
                  setTemplateTomlLoading(true);
                  try {
                    const toml = await getAgentTemplateToml(templateName);
                    // Carry the user's custom name across when dropping into
                    // TOML mode — otherwise the input they just typed gets
                    // silently discarded and the template's original name wins.
                    const customName = templateCustomName.trim();
                    const patched = customName
                      ? toml.replace(
                          /^name\s*=\s*(?:"[^"]*"|'[^']*')/m,
                          `name = "${customName.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`,
                        )
                      : toml;
                    setManifestToml(patched);
                    setCreateMode("toml");
                  } catch {
                    addToast(
                      t("agents.loading_template_toml_failed", {
                        defaultValue: "Failed to load template TOML",
                      }),
                      "error",
                    );
                  } finally {
                    setTemplateTomlLoading(false);
                  }
                }}
                className="text-[10px] font-bold text-brand hover:underline disabled:text-text-dim disabled:no-underline disabled:cursor-not-allowed"
              >
                {templateTomlLoading ? (
                  <span className="inline-flex items-center gap-1">
                    <Loader2 className="w-3 h-3 animate-spin" />
                    {t("agents.loading_template_toml", { defaultValue: "Loading template…" })}
                  </span>
                ) : (
                  t("agents.edit_template_toml", {
                    defaultValue: "Edit TOML for advanced customization →",
                  })
                )}
              </button>
            </div>
          ) : (
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.manifest_toml")}</label>
              <textarea value={manifestToml} onChange={e => {
                  setManifestToml(e.target.value);
                  // Clear stale parse error so the user gets fresh feedback
                  // on their next switch attempt instead of seeing a message
                  // that may already be addressed.
                  if (tomlParseError) setTomlParseError(null);
                }}
                placeholder={'[agent]\nname = "my-agent"\n\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n\n[thinking]\nbudget_tokens = 10000\nstream_thinking = false'}
                rows={12}
                className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs font-mono outline-none focus:border-brand resize-none" />
              <p className="text-[9px] text-text-dim/50 mt-1 flex items-center gap-1">
                <Brain className="w-3 h-3" />
                {t("agents.thinking_toml_hint")}
              </p>
            </div>
          )}

          {spawnMutation.error && (
            <p className="text-xs text-error">{toastErr(spawnMutation.error, String(spawnMutation.error))}</p>
          )}

          <div className="flex gap-2 pt-2">
            <Button variant="primary" className="flex-1"
              onClick={() => {
                const onSuccess = () => {
                  addToast(
                    t("agents.agent_created", { defaultValue: "Agent created" }),
                    "success",
                  );
                  closeCreateModal();
                };
                const onError = (e: Error) => {
                  addToast(
                    e?.message ||
                      t("agents.create_failed", { defaultValue: "Failed to create agent" }),
                    "error",
                  );
                };
                if (createMode === "form") {
                  const errors = validateManifestForm(formState);
                  setFormErrors(new Set(errors));
                  if (errors.length > 0) return;
                  spawnMutation.mutate(
                    { manifest_toml: serializedFormToml },
                    { onSuccess, onError },
                  );
                  return;
                }
                const customName = templateCustomName.trim();
                spawnMutation.mutate(
                  createMode === "template"
                    ? { template: templateName, ...(customName ? { name: customName } : {}) }
                    : { manifest_toml: manifestToml },
                  { onSuccess, onError },
                );
              }}
              disabled={
                spawnMutation.isPending ||
                templateTomlLoading ||
                (createMode === "form"
                  ? !formState.name.trim() ||
                    !formState.model.provider.trim() ||
                    !formState.model.model.trim()
                  : createMode === "template"
                    ? !templateName.trim()
                    : !manifestToml.trim())
              }>
              {spawnMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
              {t("agents.create_agent")}
            </Button>
            <Button variant="secondary" onClick={closeCreateModal}>{t("common.cancel")}</Button>
          </div>
        </div>
      </DrawerPanel>

      {/* Prompts & Experiments Modal */}
      {showPrompts && detailAgent && (
        <PromptsExperimentsModal
          agentId={detailAgent.id}
          agentName={t(`agents.builtin.${detailAgent.name}.name`, { defaultValue: detailAgent.name })}
          onClose={() => setShowPrompts(false)}
        />
      )}
      <ConfirmDialog
        isOpen={confirmDialog !== null}
        title={confirmDialog?.title ?? ""}
        message={confirmDialog?.message ?? ""}
        tone={confirmDialog?.tone}
        onConfirm={() => confirmDialog?.onConfirm()}
        onClose={() => setConfirmDialog(null)}
      />
    </div>
  );
}

function PromptsExperimentsModal({ agentId, agentName, onClose }: { agentId: string; agentName: string; onClose: () => void }) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<"versions" | "experiments">("versions");
  const [showCreateVersion, setShowCreateVersion] = useState(false);
  const [showCreateExperiment, setShowCreateExperiment] = useState(false);
  const [newPromptSystemPrompt, setNewPromptSystemPrompt] = useState("");
  const [newPromptDescription, setNewPromptDescription] = useState("");
  const [newExperimentName, setNewExperimentName] = useState("");
  const [selectedMetrics, setSelectedMetrics] = useState<string | null>(null);
  const [selectedVariantIds, setSelectedVariantIds] = useState<string[]>([]);

  const versionsQuery = usePromptVersions(agentId);
  const experimentsQuery = useExperiments(activeTab === "experiments" ? agentId : "");
  const metricsQuery = useExperimentMetrics(selectedMetrics ?? "");

  const createVersionMutation = useCreatePromptVersion();
  const createExperimentMutation = useCreateExperiment();
  const activateMutation = useActivatePromptVersion();
  const startExpMutation = useStartExperiment();
  const pauseExpMutation = usePauseExperiment();
  const completeExpMutation = useCompleteExperiment();
  const deleteVersionMutation = useDeletePromptVersion();

  const versions = versionsQuery.data ?? [];
  const experiments = experimentsQuery.data ?? [];
  const metrics = metricsQuery.data ?? [];

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/40 backdrop-blur-xl" onClick={onClose}>
      <div role="dialog" aria-modal="true" aria-labelledby="prompts-experiments-dialog-title" className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[640px] sm:max-w-[90vw] max-h-[85vh] overflow-hidden flex flex-col" onClick={e => e.stopPropagation()}>
        <div className="px-6 py-4 border-b border-border-subtle flex items-center justify-between shrink-0">
          <div>
            <h3 id="prompts-experiments-dialog-title" className="text-lg font-black">{agentName}</h3>
            <p className="text-xs text-text-dim">Prompts & Experiments</p>
          </div>
          <button onClick={onClose} className="p-2 rounded-xl hover:bg-main" aria-label={t("common.close", { defaultValue: "Close dialog" })}><X className="w-4 h-4" /></button>
        </div>
        
        <div role="tablist" aria-label="Prompts &amp; Experiments" className="px-6 py-3 border-b border-border-subtle flex gap-2 shrink-0">
          <button
            id="agents-tab-versions"
            role="tab"
            aria-selected={activeTab === "versions"}
            aria-controls="agents-panel-versions"
            tabIndex={activeTab === "versions" ? 0 : -1}
            onClick={() => setActiveTab("versions")}
            className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${activeTab === "versions" ? "bg-brand text-white" : "bg-main text-text-dim"}`}
          >
            <FlaskConical className="w-3 h-3 inline mr-1" /> Versions
          </button>
          <button
            id="agents-tab-experiments"
            role="tab"
            aria-selected={activeTab === "experiments"}
            aria-controls="agents-panel-experiments"
            tabIndex={activeTab === "experiments" ? 0 : -1}
            onClick={() => setActiveTab("experiments")}
            className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${activeTab === "experiments" ? "bg-brand text-white" : "bg-main text-text-dim"}`}
          >
            <GitBranch className="w-3 h-3 inline mr-1" /> Experiments
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-6">
          <AnimatePresence mode="wait">
          <motion.div key={activeTab} variants={tabContent} initial="initial" animate="animate" exit="exit">
          {activeTab === "versions" && (
            <div id="agents-panel-versions" role="tabpanel" aria-labelledby="agents-tab-versions" className="space-y-4">
              <div className="flex justify-end">
                <Button variant="primary" size="sm" onClick={() => setShowCreateVersion(true)}>
                  <Plus className="w-3 h-3 mr-1" /> New Version
                </Button>
              </div>
              
              {versionsQuery.isLoading ? <CardSkeleton /> : versions.length === 0 ? (
                <EmptyState title="No prompt versions yet" icon={<FlaskConical className="h-6 w-6" />} />
              ) : (
                <div className="space-y-2">
                  {versions.map((v: PromptVersion) => (
                    <div key={v.id} className={`p-4 rounded-xl border ${v.is_active ? "border-success bg-success/5" : "border-border-subtle bg-main/30"}`}>
                      <div className="flex items-center justify-between mb-2">
                        <div className="flex items-center gap-2">
                          <span className="font-bold text-sm">v{v.version}</span>
                          {v.is_active && <Badge variant="success">Active</Badge>}
                          {v.description && <span className="text-xs text-text-dim">- {v.description}</span>}
                        </div>
                        <div className="flex gap-2">
                          {!v.is_active && (
                            <Button variant="secondary" size="sm" onClick={() => activateMutation.mutate({ versionId: v.id, agentId })}>
                              <Check className="w-3 h-3 mr-1" /> Activate
                            </Button>
                          )}
                          {!v.is_active && (
                            <Button variant="secondary" size="sm" onClick={() => deleteVersionMutation.mutate({ versionId: v.id, agentId })}>
                              <Trash2 className="w-3 h-3" />
                            </Button>
                          )}
                        </div>
                      </div>
                      <pre className="text-xs text-text-dim whitespace-pre-wrap max-h-24 overflow-y-auto">{v.system_prompt.slice(0, 200)}...</pre>
                      <p className="text-[10px] text-text-dim mt-2">Created: {new Date(v.created_at).toLocaleDateString()}</p>
                    </div>
                  ))}
                </div>
              )}

              {showCreateVersion && (
                <div className="fixed inset-0 z-60 flex items-end sm:items-center justify-center bg-black/50 p-0 sm:p-4" onClick={() => setShowCreateVersion(false)}>
                  <div role="dialog" aria-modal="true" aria-labelledby="create-version-dialog-title" className="bg-surface rounded-t-2xl sm:rounded-xl shadow-2xl border border-border-subtle p-6 w-full max-w-lg" onClick={e => e.stopPropagation()}>
                    <h4 id="create-version-dialog-title" className="font-bold mb-4">Create Prompt Version</h4>
                    <div className="space-y-4">
                      <div>
                        <label className="text-xs text-text-dim">System Prompt</label>
                        <textarea value={newPromptSystemPrompt} onChange={e => setNewPromptSystemPrompt(e.target.value)} rows={6}
                          className="w-full mt-1 rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs font-mono" placeholder="You are a helpful AI assistant..." />
                      </div>
                      <div>
                        <label className="text-xs text-text-dim">Description (optional)</label>
                        <input value={newPromptDescription} onChange={e => setNewPromptDescription(e.target.value)}
                          className="w-full mt-1 rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs" placeholder="What's different in this version?" />
                      </div>
                    </div>
                    <div className="flex gap-2 mt-4">
                      <Button variant="primary" className="flex-1" isLoading={createVersionMutation.isPending} onClick={() => createVersionMutation.mutate({ agentId, version: { system_prompt: newPromptSystemPrompt, description: newPromptDescription, version: (versionsQuery.data?.length || 0) + 1, content_hash: "", tools: [], variables: [], created_by: "dashboard" } }, { onSuccess: () => { setShowCreateVersion(false); setNewPromptSystemPrompt(""); setNewPromptDescription(""); } })} disabled={!newPromptSystemPrompt.trim() || createVersionMutation.isPending}>
                        {createVersionMutation.isPending ? "Creating..." : "Create"}
                      </Button>
                      <Button variant="secondary" onClick={() => setShowCreateVersion(false)}>Cancel</Button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}

          {activeTab === "experiments" && (
            <div id="agents-panel-experiments" role="tabpanel" aria-labelledby="agents-tab-experiments" className="space-y-4">
              <div className="flex justify-end">
                <Button variant="primary" size="sm" onClick={() => setShowCreateExperiment(true)}>
                  <Plus className="w-3 h-3 mr-1" /> New Experiment
                </Button>
              </div>

              {experimentsQuery.isLoading ? <CardSkeleton /> : experiments.length === 0 ? (
                <EmptyState title="No experiments yet" icon={<GitBranch className="h-6 w-6" />} />
              ) : (
                <div className="space-y-2">
                  {experiments.map((exp: PromptExperiment) => (
                    <div key={exp.id} className="p-4 rounded-xl border border-border-subtle bg-main/30">
                      <div className="flex items-center justify-between mb-2">
                        <div className="flex items-center gap-2">
                          <span className="font-bold text-sm">{exp.name}</span>
                          <Badge variant={exp.status === "running" ? "success" : exp.status === "completed" ? "default" : "warning"}>{exp.status}</Badge>
                        </div>
                        <div className="flex gap-2">
                          {exp.status === "draft" && <Button variant="secondary" size="sm" onClick={() => startExpMutation.mutate({ experimentId: exp.id, agentId })}><Play className="w-3 h-3 mr-1" />Start</Button>}
                          {exp.status === "running" && <Button variant="secondary" size="sm" onClick={() => pauseExpMutation.mutate({ experimentId: exp.id, agentId })}><Pause className="w-3 h-3 mr-1" />Pause</Button>}
                          {(exp.status === "running" || exp.status === "paused") && (
                            <Button variant="secondary" size="sm" onClick={() => completeExpMutation.mutate({ experimentId: exp.id, agentId })}>
                              <Check className="w-3 h-3 mr-1" />Complete
                            </Button>
                          )}
                          {(exp.status === "running" || exp.status === "paused") && (
                            <Button variant="secondary" size="sm" onClick={() => setSelectedMetrics(exp.id)}>
                              <BarChart3 className="w-3 h-3 mr-1" />Metrics
                            </Button>
                          )}
                        </div>
                      </div>
                      <p className="text-xs text-text-dim">{exp.variants?.length || 0} variants</p>
                    </div>
                  ))}
                </div>
              )}

              {selectedMetrics && metricsQuery.data && (
                <div className="mt-4 p-4 rounded-xl bg-main/50 border border-border-subtle">
                  <h5 className="text-xs font-bold mb-3">Experiment Metrics</h5>
                  <div className="space-y-2">
                    {metrics.map((m: ExperimentVariantMetrics) => (
                      <div key={m.variant_id} className="p-3 rounded-lg bg-surface border border-border-subtle">
                        <div className="flex items-center justify-between mb-2">
                          <span className="font-bold text-xs">{m.variant_name}</span>
                          <Badge variant={m.success_rate >= 80 ? "success" : m.success_rate >= 50 ? "warning" : "default"}>
                            {m.success_rate?.toFixed(1)}%
                          </Badge>
                        </div>
                        <div className="grid grid-cols-3 gap-2 text-[10px] text-text-dim">
                          <div>
                            <span className="block text-text-dim/60">Requests</span>
                            <span className="font-mono">{m.total_requests} ({m.successful_requests} ok / {m.failed_requests} err)</span>
                          </div>
                          <div>
                            <span className="block text-text-dim/60">Avg Latency</span>
                            <span className="font-mono">{m.avg_latency_ms?.toFixed(0)}ms</span>
                          </div>
                          <div>
                            <span className="block text-text-dim/60">Avg Cost</span>
                            <span className="font-mono">${m.avg_cost_usd?.toFixed(4)}</span>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                  <Button variant="secondary" size="sm" className="mt-3 w-full" onClick={() => setSelectedMetrics(null)}>Close Metrics</Button>
                </div>
              )}

              {showCreateExperiment && (
                <div className="fixed inset-0 z-60 flex items-end sm:items-center justify-center bg-black/50 p-0 sm:p-4" onClick={() => setShowCreateExperiment(false)}>
                  <div role="dialog" aria-modal="true" aria-labelledby="create-experiment-dialog-title" className="bg-surface rounded-t-2xl sm:rounded-xl shadow-2xl border border-border-subtle p-6 w-full max-w-lg" onClick={e => e.stopPropagation()}>
                    <h4 id="create-experiment-dialog-title" className="font-bold mb-4">Create Experiment</h4>
                    <div className="space-y-4">
                      <div>
                        <label className="text-xs text-text-dim">Experiment Name</label>
                        <input value={newExperimentName} onChange={e => setNewExperimentName(e.target.value)}
                          className="w-full mt-1 rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs" placeholder="My A/B Test" />
                      </div>
                      <div>
                        <label className="text-xs text-text-dim mb-2 block">Select Prompt Versions (min 2)</label>
                        {versions.length < 2 ? (
                          <p className="text-xs text-warning">Create at least 2 prompt versions first.</p>
                        ) : (
                          <div className="space-y-1 max-h-40 overflow-y-auto">
                            {versions.map((v: PromptVersion) => (
                              <label key={v.id} className={`flex items-center gap-2 p-2 rounded-lg cursor-pointer text-xs ${selectedVariantIds.includes(v.id) ? "bg-brand/10 border border-brand" : "bg-main/30 border border-border-subtle"}`}>
                                <input type="checkbox" checked={selectedVariantIds.includes(v.id)}
                                  onChange={e => {
                                    if (e.target.checked) setSelectedVariantIds([...selectedVariantIds, v.id]);
                                    else setSelectedVariantIds(selectedVariantIds.filter(id => id !== v.id));
                                  }} className="rounded" />
                                <span className="font-bold">v{v.version}</span>
                                {v.is_active && <Badge variant="success">Active</Badge>}
                                <span className="text-text-dim truncate">{v.description || v.system_prompt.slice(0, 40) + "..."}</span>
                              </label>
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                    <div className="flex gap-2 mt-4">
                      <Button variant="primary" className="flex-1" isLoading={createExperimentMutation.isPending} onClick={() => createExperimentMutation.mutate({ agentId, experiment: { name: newExperimentName, status: "draft", traffic_split: selectedVariantIds.map(() => Math.floor(100 / selectedVariantIds.length)), success_criteria: { require_user_helpful: true, require_no_tool_errors: true, require_non_empty: true }, variants: selectedVariantIds.map((vId, i) => { const ver = versions.find(v => v.id === vId); return { name: i === 0 ? "Control" : `Variant ${String.fromCharCode(65 + i)}`, prompt_version_id: vId, description: ver ? `v${ver.version}` : undefined }; }) } }, { onSuccess: () => { setShowCreateExperiment(false); setNewExperimentName(""); setSelectedVariantIds([]); } })} disabled={!newExperimentName.trim() || selectedVariantIds.length < 2 || createExperimentMutation.isPending}>
                        {createExperimentMutation.isPending ? "Creating..." : `Create (${selectedVariantIds.length} variants)`}
                      </Button>
                      <Button variant="secondary" onClick={() => { setShowCreateExperiment(false); setSelectedVariantIds([]); }}>Cancel</Button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}
          </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
}
