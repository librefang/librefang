import { formatTime } from "../lib/datetime";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import {
  type AgentDetail,
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
import { Modal } from "../components/ui/Modal";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { MultiSelectCmdk } from "../components/ui/MultiSelectCmdk";
import { Card } from "../components/ui/Card";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Avatar } from "../components/ui/Avatar";
import { useUIStore } from "../lib/store";
import { filterVisible } from "../lib/hiddenModels";
import { Search, Users, MessageCircle, X, Cpu, Wrench, Shield, Plus, Loader2, Pause, Play, Clock, Brain, Zap, FlaskConical, GitBranch, Trash2, Check, BarChart3, Copy, RotateCcw } from "lucide-react";
import { truncateId } from "../lib/string";
import { getStatusVariant } from "../lib/status";
import { useDashboardSnapshot } from "../lib/queries/runtime";
import { useProviders } from "../lib/queries/providers";
import { useModels } from "../lib/queries/models";
import { AgentManifestForm } from "../components/AgentManifestForm";
import {
  emptyManifestForm,
  serializeManifestForm,
  validateManifestForm,
  type ManifestFormState,
} from "../lib/agentManifest";
import {
  agentQueries,
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
  usePatchAgentConfig,
  usePauseExperiment,
  useResumeAgent,
  useSpawnAgent,
  useStartExperiment,
  useSuspendAgent,
} from "../lib/mutations/agents";

export function AgentsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [detailAgent, setDetailAgent] = useState<AgentDetail | null>(null);
  const [, setDetailLoading] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [createMode, setCreateMode] = useState<"form" | "template" | "toml">("form");
  const [templateName, setTemplateName] = useState("");
  const [manifestToml, setManifestToml] = useState("");
  const [templateTomlLoading, setTemplateTomlLoading] = useState(false);
  const [formState, setFormState] = useState<ManifestFormState>(emptyManifestForm);
  const [formErrors, setFormErrors] = useState<Set<string>>(new Set());
  const [showPrompts, setShowPrompts] = useState(false);
  const [editingModel, setEditingModel] = useState(false);
  const [modelDraft, setModelDraft] = useState({ provider: "", model: "", max_tokens: "", temperature: "" });
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
  const [toolAllowlistDraft, setToolAllowlistDraft] = useState<string[]>([]);
  const [toolBlocklistDraft, setToolBlocklistDraft] = useState<string[]>([]);
  const [toolsDisabledState, setToolsDisabledState] = useState(false);
  const [toolsEditorLoading, setToolsEditorLoading] = useState(false);
  const [toolsEditorSaving, setToolsEditorSaving] = useState(false);
  const [availableToolNames, setAvailableToolNames] = useState<string[]>([]);
  const [stateFilter, setStateFilter] = useState<"all" | "running" | "suspended">("all");
  const [sortBy, setSortBy] = useState<"name" | "last_active" | "created_at">("name");
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
  const cloneMutation = useCloneAgent();
  const qc = useQueryClient();

  const rawDeleteMutation = useDeleteAgent();
  const deleteMutation = {
    mutate: (agentId: string) =>
      rawDeleteMutation.mutate(agentId, {
        onSuccess: () => {
          setDetailAgent(null);
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
    setDetailAgent(null);
    setEditingModel(false);
    closeToolsEditor();
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
        setToolAllowlistDraft(Array.isArray(agentTools?.tool_allowlist) ? agentTools.tool_allowlist : []);
        setToolBlocklistDraft(Array.isArray(agentTools?.tool_blocklist) ? agentTools.tool_blocklist : []);
        setToolsDisabledState(Boolean(agentTools?.disabled));
      } catch (err: any) {
        if (cancelled) return;
        addToast(err?.message || t("agents.tools_load_failed", { defaultValue: "Failed to load tools" }), "error");
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

    patchAgentConfigMutation.mutate(
      { agentId: detailAgent.id, config: patch },
      {
        onSuccess: async () => {
          setEditingModel(false);
          await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
          addToast(t("agents.model_saved", { defaultValue: "Model updated" }), "success");
        },
        onError: (e: any) => {
          addToast(
            e?.message || t("agents.model_save_failed", { defaultValue: "Failed to update model" }),
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
    () => serializeManifestForm(formState),
    [formState],
  );

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

  const renderAgentCard = (agent: any) => {
    const isSuspended = (agent.state || "").toLowerCase() === "suspended";
    return (
      <Card key={agent.id} hover padding="lg" className={`cursor-pointer overflow-hidden min-w-0 ${isSuspended ? "opacity-60" : ""}`} onClick={async () => {
        setDetailLoading(true);
        try { const d = await qc.fetchQuery(agentQueries.detail(agent.id)); setDetailAgent(mergeHandFlag(d, agent.is_hand)); } catch { setDetailAgent({ name: agent.name, id: agent.id, is_hand: agent.is_hand } as AgentDetail); }
        setDetailLoading(false);
      }}>
        <div className="flex flex-col gap-3 mb-5">
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-center gap-3 min-w-0 flex-1">
              <div className="relative shrink-0">
                <Avatar fallback={agent.name} size="lg" />
                {!isSuspended && <span className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full bg-success border-2 border-surface animate-pulse" />}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 min-w-0">
                  <h2 className="text-sm sm:text-base font-black tracking-tight truncate">{t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}</h2>
                  {agent.is_hand && <Badge variant="info" className="shrink-0">{t("agents.hand_badge", { defaultValue: "HAND" })}</Badge>}
                </div>
                <p className="text-[10px] font-mono text-text-dim/50 truncate mt-0.5">{truncateId(agent.id)}</p>
              </div>
            </div>
            <Badge variant={getStatusVariant(agent.state)} dot className="shrink-0">
              {agent.state ? t(`common.${agent.state.toLowerCase()}`, { defaultValue: agent.state }) : t("common.idle")}
            </Badge>
          </div>
        </div>
        <div className="space-y-2.5 mb-5">
          <div className="flex items-center gap-3 text-xs">
            <div className="w-5 h-5 rounded bg-brand/10 flex items-center justify-center shrink-0"><Cpu className="w-3 h-3 text-brand" /></div>
            <span className="text-text-dim flex-1">{t("agents.model")}</span>
            <span className="font-black text-sm">{agent.model_name || t("common.unknown")}</span>
          </div>
          <div className="flex items-center gap-3 text-xs">
            <div className="w-5 h-5 rounded bg-success/10 flex items-center justify-center shrink-0"><Shield className="w-3 h-3 text-success" /></div>
            <span className="text-text-dim flex-1">{t("agents.provider")}</span>
            <span className="font-black text-brand text-sm">{agent.model_provider || t("common.local")}</span>
          </div>
          <div className="flex items-center gap-3 text-xs">
            <div className="w-5 h-5 rounded bg-warning/10 flex items-center justify-center shrink-0"><Clock className="w-3 h-3 text-warning" /></div>
            <span className="text-text-dim flex-1">{t("agents.last_active")}</span>
            <span className="font-mono text-[10px]">{agent.last_active ? formatTime(agent.last_active) : t("common.never")}</span>
          </div>
        </div>
        <div className="pt-4 border-t border-border-subtle/30 flex flex-wrap gap-2">
          {isSuspended ? (
            <Button variant="secondary" size="sm" className="flex-1 min-w-[100px]" onClick={async (e) => { e.stopPropagation(); try { await resumeMutation.mutateAsync(agent.id); } catch (err: any) { addToast(err?.message || t("agents.resume_failed", { defaultValue: "Failed to resume agent" }), "error"); } }}>
              <Play className="h-3.5 w-3.5 mr-1 shrink-0" /> <span className="truncate">{t("agents.resume")}</span>
            </Button>
          ) : (
            <Button variant="secondary" size="sm" className="flex-1 min-w-[100px]" onClick={async (e) => { e.stopPropagation(); try { await suspendMutation.mutateAsync(agent.id); } catch (err: any) { addToast(err?.message || t("agents.suspend_failed", { defaultValue: "Failed to suspend agent" }), "error"); } }}>
              <Pause className="h-3.5 w-3.5 mr-1 shrink-0" /> <span className="truncate">{t("agents.suspend")}</span>
            </Button>
          )}
          <Button variant="primary" size="sm" className="flex-1 min-w-[100px]" onClick={(e) => { e.stopPropagation(); navigate({ to: "/chat", search: { agentId: agent.id } }); }}>
            <MessageCircle className="h-3.5 w-3.5 mr-1 shrink-0" /> <span className="truncate">{t("common.interact")}</span>
          </Button>
          {!agent.is_hand && (
            <Button
              variant="secondary"
              size="sm"
              className="shrink-0"
              onClick={(e) => {
                e.stopPropagation();
                setConfirmDialog({
                  title: t("agents.delete_title", { defaultValue: "Delete agent?" }),
                  message: t("agents.delete_confirm", { name: agent.name }),
                  tone: "destructive",
                  onConfirm: () => deleteMutation.mutate(agent.id),
                });
              }}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
      </Card>
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

      <Input
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder={t("common.search")}
        leftIcon={<Search className="h-4 w-4" />}
        data-shortcut-search
      />

      <div className="flex items-center gap-2 -mt-2 flex-wrap">
        <button
          onClick={() => setShowHandAgents((value) => !value)}
          aria-pressed={showHandAgents}
          className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-[11px] font-bold transition-colors ${
            showHandAgents
              ? "border-brand/30 bg-brand/10 text-brand"
              : "border-border-subtle bg-surface text-text-dim hover:border-brand/20 hover:text-brand"
          }`}
        >
          <span>{t("agents.show_hand_agents", { defaultValue: "Show hand agents" })}</span>
        </button>
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
              className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-[11px] font-bold transition-colors ${
                isActive
                  ? "border-brand/30 bg-brand/10 text-brand"
                  : "border-border-subtle bg-surface text-text-dim hover:border-brand/20 hover:text-brand"
              }`}
            >
              <span>{label}</span>
              <span
                className={`inline-flex items-center justify-center rounded-full px-1.5 min-w-[18px] h-[18px] text-[9px] font-mono ${
                  isActive ? "bg-brand/20" : "bg-main"
                }`}
              >
                {count}
              </span>
            </button>
          );
        })}
        <div className="ml-auto flex items-center gap-1.5">
          <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim/60">
            {t("common.sort_by", { defaultValue: "Sort" })}
          </span>
          <select
            value={sortBy}
            onChange={(e) => setSortBy(e.target.value as typeof sortBy)}
            className="rounded-full border border-border-subtle bg-surface px-3 py-1 text-[11px] font-bold text-text-dim outline-none focus:border-brand hover:border-brand/20 cursor-pointer"
          >
            <option value="name">{t("common.sort_name", { defaultValue: "Name" })}</option>
            <option value="last_active">{t("common.sort_last_active", { defaultValue: "Last active" })}</option>
            <option value="created_at">{t("common.sort_created", { defaultValue: "Created" })}</option>
          </select>
        </div>
      </div>

      {agentsQuery.isLoading ? (
        <div className="grid gap-4 grid-cols-[repeat(auto-fill,minmax(280px,1fr))]">
          {[1, 2, 3, 4, 5, 6].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : filteredAgents.length === 0 ? (
        search || stateFilter !== "all" || showHandAgents ? (
          <EmptyState
            title={t("agents.no_matching")}
            icon={<Search className="h-6 w-6" />}
            action={
              (search || stateFilter !== "all" || showHandAgents) && (
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
              )
            }
          />
        ) : (
          <EmptyState
            title={t("common.no_data")}
            icon={<Users className="h-6 w-6" />}
          />
        )
      ) : (
        <div className="grid gap-4 grid-cols-[repeat(auto-fill,minmax(280px,1fr))] stagger-children">
          {coreAgents.map(agent => renderAgentCard(agent))}
        </div>
      )}
      {/* Agent Detail Modal */}
      {detailAgent && (() => {
        const detailState = ((detailAgent as any).state || "").toLowerCase();
        const isDetailSuspended = detailState === "suspended";
        const statusColor = isDetailSuspended ? "bg-warning" : detailState === "crashed" ? "bg-error" : "bg-success";
        return (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/40 backdrop-blur-sm" onClick={closeDetailModal}>
          <div className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[560px] sm:max-w-[90vw] max-h-[85vh] sm:max-h-[80vh] overflow-y-auto animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            {/* Modal Header */}
            <div className="px-6 py-5 border-b border-border-subtle sticky top-0 bg-surface/95 backdrop-blur-sm z-10">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-4">
                  <div className="relative">
                    <Avatar fallback={detailAgent.name} size="lg" />
                    <span className={`absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full ${statusColor} border-2 border-surface ${!isDetailSuspended && detailState !== "crashed" ? "animate-pulse" : ""}`} />
                  </div>
                  <div>
                    <h3 className="text-lg font-black tracking-tight">{t(`agents.builtin.${detailAgent.name}.name`, { defaultValue: detailAgent.name })}</h3>
                    <div className="flex items-center gap-2 mt-0.5">
                      <p className="text-[10px] text-text-dim font-mono">{truncateId(detailAgent.id, 16)}</p>
                      {detailAgent.is_hand && <Badge variant="info">{t("agents.hand_badge", { defaultValue: "HAND" })}</Badge>}
                      <Badge variant={isDetailSuspended ? "warning" : "success"} dot>
                        {(detailAgent as any).state ? t(`common.${detailState}`, { defaultValue: (detailAgent as any).state }) : t("common.running")}
                      </Badge>
                    </div>
                  </div>
                </div>
                <button onClick={closeDetailModal} className="p-2 rounded-xl hover:bg-main transition-colors" aria-label={t("common.close", { defaultValue: "Close" })}><X className="w-4 h-4" /></button>
              </div>
            </div>
            <div className="p-6 space-y-5">

              {/* Description */}
              {(detailAgent as any).description && (
                <p className="text-xs text-text-dim leading-relaxed">{(detailAgent as any).description}</p>
              )}
              {/* Model */}
              {detailAgent.model && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3 flex items-center gap-2">
                    <div className="w-5 h-5 rounded bg-brand/10 flex items-center justify-center"><Cpu className="w-3 h-3 text-brand" /></div>
                    {t("agents.model")}
                  </h4>
                  <div className="p-4 rounded-xl bg-main/50 border border-border-subtle/50 space-y-2.5 text-xs">
                    {detailAgent.is_hand && (
                      <p className="rounded-lg border border-brand/15 bg-brand/5 px-3 py-2 text-[11px] leading-relaxed text-text-dim">
                        {t("agents.hand_agent_hint", { defaultValue: "You are editing the active runtime agent created by a hand." })}
                      </p>
                    )}
                    {editingModel ? (
                      <>
                        <div className="flex justify-between items-center gap-2">
                          <span className="text-text-dim">{t("agents.provider")}</span>
                          <select
                            value={modelDraft.provider}
                            onChange={e => setModelDraft(d => ({ ...d, provider: e.target.value, model: "" }))}
                            className="w-40 px-2 py-1 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand text-right"
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
                        </div>
                        <div className="flex justify-between items-center gap-2">
                          <span className="text-text-dim">{t("agents.model")}</span>
                          <select
                            value={modelDraft.model}
                            onChange={e => setModelDraft(d => ({ ...d, model: e.target.value }))}
                            className="w-40 px-2 py-1 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand text-right"
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
                        </div>
                        <div className="flex justify-between items-center gap-2">
                          <span className="text-text-dim">{t("agents.max_tokens")}</span>
                          <input
                            type="number"
                            min={1}
                            max={200000}
                            value={modelDraft.max_tokens}
                            onChange={e => setModelDraft(d => ({ ...d, max_tokens: e.target.value }))}
                            className="w-40 px-2 py-1 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand text-right"
                          />
                        </div>
                        <div className="flex justify-between items-center gap-2">
                          <span className="text-text-dim">{t("agents.temperature")}</span>
                          <input
                            type="number"
                            min={0}
                            max={2}
                            step={0.1}
                            value={modelDraft.temperature}
                            onChange={e => setModelDraft(d => ({ ...d, temperature: e.target.value }))}
                            className="w-40 px-2 py-1 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand text-right"
                          />
                        </div>
                        <div className="flex justify-end gap-1 pt-1">
                          <button
                            onClick={cancelModelEdit}
                            className="px-3 py-1 rounded text-xs font-bold bg-main hover:bg-main/80 text-text-dim border border-border-subtle"
                          >
                            {t("common.cancel")}
                          </button>
                          <button
                            onClick={saveModelEdit}
                            disabled={patchAgentConfigMutation.isPending || !modelDraft.provider.trim() || !modelDraft.model.trim() || isNaN(parseInt(modelDraft.max_tokens, 10)) || parseInt(modelDraft.max_tokens, 10) <= 0 || isNaN(parseFloat(modelDraft.temperature)) || parseFloat(modelDraft.temperature) < 0 || parseFloat(modelDraft.temperature) > 2}
                            className="px-3 py-1 rounded text-xs font-bold bg-brand hover:bg-brand/90 text-white disabled:opacity-50"
                          >
                            {patchAgentConfigMutation.isPending ? t("common.saving") : t("common.save")}
                          </button>
                        </div>
                      </>
                    ) : (
                      <>
                        <div className="flex justify-between items-center"><span className="text-text-dim">{t("agents.provider")}</span><span className="font-black text-brand">{detailAgent.model.provider}</span></div>
                        <div className="flex justify-between items-center"><span className="text-text-dim">{t("agents.model")}</span><span className="font-black">{detailAgent.model.model}</span></div>
                        <div className="flex justify-between items-center"><span className="text-text-dim">{t("agents.max_tokens")}</span><span className="font-black">{(detailAgent.model.max_tokens ?? 4096).toLocaleString()}</span></div>
                        {detailAgent.model.temperature != null && (
                          <div className="flex justify-between items-center"><span className="text-text-dim">{t("agents.temperature")}</span><span className="font-black">{detailAgent.model.temperature}</span></div>
                        )}
                        <div className="flex justify-end pt-1">
                          <button onClick={startModelEdit} className="px-3 py-1 rounded text-xs font-bold bg-brand/10 hover:bg-brand/20 text-brand">{t("common.edit")}</button>
                        </div>
                      </>
                    )}
                  </div>
                </div>
              )}

              {/* Web Search Augmentation */}
              <div>
                <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3">{t("agents.web_search", { defaultValue: "Web Search" })}</h4>
                <div className="p-4 rounded-xl bg-main/50 border border-border-subtle/50">
                  <div className="flex justify-between items-center gap-2">
                    <div>
                      <span className="text-xs text-text-dim">{t("agents.web_search_augmentation", { defaultValue: "Search Augmentation" })}</span>
                      <p className="text-[10px] text-text-dim/60 mt-0.5">{t("agents.web_search_augmentation_hint", { defaultValue: "Auto-search the web and inject results into context before LLM call" })}</p>
                    </div>
                    <select
                      value={detailAgent.web_search_augmentation || "off"}
                      onChange={e => {
                        const mode = e.target.value as "off" | "auto" | "always";
                        patchAgentConfigMutation.mutate(
                          { agentId: detailAgent.id, config: { web_search_augmentation: mode } },
                          {
                            onSuccess: async () => {
                              await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                            },
                          },
                        );
                      }}
                      className="w-28 px-2 py-1 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand text-right"
                    >
                      <option value="off">{t("common.off", { defaultValue: "Off" })}</option>
                      <option value="auto">{t("common.auto", { defaultValue: "Auto" })}</option>
                      <option value="always">{t("common.always", { defaultValue: "Always" })}</option>
                    </select>
                  </div>
                </div>
              </div>

              {/* System Prompt */}
              {detailAgent.system_prompt && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3">{t("agents.system_prompt")}</h4>
                  <pre className="p-4 rounded-xl bg-main/50 border border-border-subtle/50 text-xs text-text-dim whitespace-pre-wrap max-h-40 overflow-y-auto leading-relaxed font-mono">{detailAgent.system_prompt}</pre>
                </div>
              )}

              {/* Capabilities */}
              {detailAgent.capabilities && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3 flex items-center gap-2">
                    <div className="w-5 h-5 rounded bg-success/10 flex items-center justify-center"><Wrench className="w-3 h-3 text-success" /></div>
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
                </div>
              )}

              {/* Skills */}
              {detailAgent.skills && detailAgent.skills.length > 0 && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3">{t("agents.skills")}</h4>
                  <div className="flex flex-wrap gap-2">
                    {detailAgent.skills.map((s: string, i: number) => (
                      <Badge key={i} variant="default">{s}</Badge>
                    ))}
                  </div>
                </div>
              )}

              {/* Tags */}
              {detailAgent.tags && detailAgent.tags.length > 0 && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3">{t("agents.tags")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.tags.map((tag: string, i: number) => (
                      <span key={i} className="text-[10px] px-2.5 py-1 rounded-lg bg-main border border-border-subtle/50 text-text-dim font-medium">{tag}</span>
                    ))}
                  </div>
                </div>
              )}

              {/* Mode */}
              {detailAgent.mode && (
                <div className="flex items-center gap-3 p-3 rounded-xl bg-main/50 border border-border-subtle/50">
                  <div className="w-5 h-5 rounded bg-warning/10 flex items-center justify-center"><Shield className="w-3 h-3 text-warning" /></div>
                  <span className="text-xs font-bold flex-1">{t("agents.mode")}</span>
                  <Badge variant="warning">{detailAgent.mode}</Badge>
                </div>
              )}

              {/* Thinking / Extended Reasoning */}
              {detailAgent.thinking && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3 flex items-center gap-2">
                    <div className="w-5 h-5 rounded bg-purple-500/10 flex items-center justify-center"><Brain className="w-3 h-3 text-purple-500" /></div>
                    {t("agents.thinking")}
                  </h4>
                  <div className="p-4 rounded-xl bg-main/50 border border-border-subtle/50 space-y-2.5 text-xs">
                    <div className="flex justify-between items-center">
                      <span className="text-text-dim">{t("agents.thinking_enabled")}</span>
                      <Badge variant={(detailAgent.thinking.budget_tokens ?? 0) > 0 ? "success" : "default"}>
                        {(detailAgent.thinking.budget_tokens ?? 0) > 0 ? t("common.yes") : t("common.no")}
                      </Badge>
                    </div>
                    <div className="flex justify-between items-center">
                      <span className="text-text-dim">{t("agents.budget_tokens")}</span>
                      <span className="font-black text-sm">{detailAgent.thinking.budget_tokens?.toLocaleString() ?? 0}</span>
                    </div>
                    <div className="flex justify-between items-center">
                      <span className="text-text-dim">{t("agents.stream_thinking")}</span>
                      <Badge variant={detailAgent.thinking.stream_thinking ? "brand" : "default"}>
                        {detailAgent.thinking.stream_thinking ? t("common.yes") : t("common.no")}
                      </Badge>
                    </div>
                    <p className="text-[10px] text-text-dim/50 flex items-center gap-1 pt-1">
                      <Zap className="w-3 h-3" />
                      {t("agents.thinking_hint")}
                    </p>
                  </div>
                </div>
              )}

              {/* Actions */}
              <div className="space-y-3 pt-3 border-t border-border-subtle">
                {/* Primary action */}
                <Button variant="primary" size="sm" className="w-full" onClick={() => { closeDetailModal(); navigate({ to: "/chat", search: { agentId: detailAgent.id } }); }}>
                  <MessageCircle className="w-3.5 h-3.5 mr-1.5" />
                  {t("common.interact")}
                </Button>

                {/* Management actions */}
                <div className="grid grid-cols-4 gap-2">
                  {isDetailSuspended ? (
                    <Button variant="secondary" size="sm" className="flex-col gap-1 py-2.5 h-auto" onClick={async () => { try { await resumeMutation.mutateAsync(detailAgent.id); await refreshDetailAgent(detailAgent.id, detailAgent.is_hand); } catch (err: any) { addToast(err?.message || t("agents.resume_failed", { defaultValue: "Failed to resume agent" }), "error"); } }}>
                      <Play className="w-4 h-4" />
                      <span className="text-[9px]">{t("agents.resume")}</span>
                    </Button>
                  ) : (
                    <Button variant="secondary" size="sm" className="flex-col gap-1 py-2.5 h-auto" onClick={async () => { try { await suspendMutation.mutateAsync(detailAgent.id); await refreshDetailAgent(detailAgent.id, detailAgent.is_hand); } catch (err: any) { addToast(err?.message || t("agents.suspend_failed", { defaultValue: "Failed to suspend agent" }), "error"); } }}>
                      <Pause className="w-4 h-4" />
                      <span className="text-[9px]">{t("agents.suspend")}</span>
                    </Button>
                  )}
                  <Button variant="secondary" size="sm" className="flex-col gap-1 py-2.5 h-auto" onClick={async () => { try { await cloneMutation.mutateAsync(detailAgent.id); } catch (err: any) { addToast(err?.message || t("agents.clone_failed", { defaultValue: "Failed to clone agent" }), "error"); } }}>
                    <Copy className="w-4 h-4" />
                    <span className="text-[9px]">{t("agents.clone")}</span>
                  </Button>
                  <Button
                    variant="secondary"
                    size="sm"
                    className="flex-col gap-1 py-2.5 h-auto"
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
                    <RotateCcw className="w-4 h-4" />
                    <span className="text-[9px]">{t("agents.reset")}</span>
                  </Button>
                  {!detailAgent.is_hand && (
                    <Button
                      variant="secondary"
                      size="sm"
                      className="flex-col gap-1 py-2.5 h-auto text-error/70 hover:text-error"
                      onClick={() =>
                        setConfirmDialog({
                          title: t("agents.delete_title", { defaultValue: "Delete agent?" }),
                          message: t("agents.delete_confirm", { name: detailAgent.name }),
                          tone: "destructive",
                          onConfirm: () => deleteMutation.mutate(detailAgent.id),
                        })
                      }
                    >
                      <Trash2 className="w-4 h-4" />
                      <span className="text-[9px]">{t("common.delete")}</span>
                    </Button>
                  )}
                </div>

                {/* Prompts link */}
                <Button variant="secondary" size="sm" className="w-full" onClick={() => setShowPrompts(true)}>
                  <FlaskConical className="w-3.5 h-3.5 mr-1.5" />
                  {t("agents.prompts")}
                </Button>
              </div>
            </div>
          </div>
        </div>
        );
      })()}

      {/* Tools Editor Modal */}
      {showToolsEditor && toolsEditorAgentId && (
        <Modal isOpen={showToolsEditor} onClose={closeToolsEditor} title={t("agents.tools_editor_title", { defaultValue: "Agent Tools" })} size="lg" zIndex={60} overflowVisible>
          <div className="p-6 space-y-5">
            <div>
              <p className="text-[11px] text-text-dim/70">
                {t("agents.tools_editor_desc", { defaultValue: "Review the current tools state for this agent. Empty allowlist means no allow restriction; blocklist still removes selected tools." })}
              </p>
              {!toolsEditorLoading && (
                <p className="mt-2 text-[10px] text-text-dim/50 font-mono">
                  {availableToolNames.length} {t("agents.tools_available", { defaultValue: "tools available" })} · {toolAllowlistDraft.length} {t("agents.tools_allowed_count", { defaultValue: "allowed" })} · {toolBlocklistDraft.length} {t("agents.tools_blocked_count", { defaultValue: "blocked" })}
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
                      {t("agents.tools_allowlist_title", { defaultValue: "Allowlist" })}
                    </h4>
                    <p className="text-[11px] text-text-dim/70 mb-3">
                      {t("agents.tools_allowlist_desc", { defaultValue: "These tools are explicitly allowed. Leave empty to allow all tools except blocked ones." })}
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
                } catch (err: any) {
                  addToast(err?.message || t("agents.tools_save_failed", { defaultValue: "Failed to update tools" }), "error");
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
        </Modal>
      )}

      {/* Create Agent Modal */}
      <Modal isOpen={showCreate} onClose={() => setShowCreate(false)} title={t("agents.create_agent")} size="lg">
        <div className="p-5 space-y-4">
          {/* Mode tabs */}
          <div className="flex gap-2">
            <button onClick={() => setCreateMode("form")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "form" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_form")}
            </button>
            <button onClick={() => setCreateMode("template")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "template" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_template")}
            </button>
            <button onClick={() => setCreateMode("toml")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "toml" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_toml")}
            </button>
          </div>

          {createMode === "form" ? (
            <div className="grid grid-cols-1 lg:grid-cols-[1fr_360px] gap-4 max-h-[60vh] overflow-y-auto pr-1">
              <AgentManifestForm
                value={formState}
                onChange={setFormState}
                providers={formProviderOptions}
                models={formModelOptions}
                invalidFields={formErrors}
              />
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-[10px] font-bold text-text-dim uppercase">
                    {t("agents.form.live_toml")}
                  </label>
                  <button
                    type="button"
                    onClick={() => {
                      setManifestToml(serializedFormToml);
                      setCreateMode("toml");
                    }}
                    className="text-[10px] font-bold text-brand hover:underline"
                  >
                    {t("agents.form.switch_to_toml")}
                  </button>
                </div>
                <pre className="rounded-xl border border-border-subtle bg-main px-3 py-2 text-[11px] font-mono text-text-dim overflow-auto max-h-[55vh] whitespace-pre-wrap break-all">
                  {serializedFormToml}
                </pre>
              </div>
            </div>
          ) : createMode === "template" ? (
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.template_name")}</label>
              <select value={templateName}
                onChange={async e => {
                  const selected = e.target.value;
                  setTemplateName(selected);
                  if (!selected) return;
                  setTemplateTomlLoading(true);
                  try {
                    const toml = await getAgentTemplateToml(selected);
                    setManifestToml(toml);
                    setCreateMode("toml");
                  } catch {
                    // Fetch failed — stay on template tab, fall back to template-name submit
                  } finally {
                    setTemplateTomlLoading(false);
                  }
                }}
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
              {templateTomlLoading && (
                <p className="text-[10px] text-text-dim mt-1 flex items-center gap-1">
                  <Loader2 className="w-3 h-3 animate-spin" />
                  {t("agents.loading_template_toml", { defaultValue: "Loading template…" })}
                </p>
              )}
            </div>
          ) : (
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.manifest_toml")}</label>
              <textarea value={manifestToml} onChange={e => setManifestToml(e.target.value)}
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
            <p className="text-xs text-error">{(spawnMutation.error as any)?.message || String(spawnMutation.error)}</p>
          )}

          <div className="flex gap-2 pt-2">
            <Button variant="primary" className="flex-1"
              onClick={() => {
                if (createMode === "form") {
                  const errors = validateManifestForm(formState);
                  setFormErrors(new Set(errors));
                  if (errors.length > 0) return;
                  spawnMutation.mutate({ manifest_toml: serializedFormToml });
                  return;
                }
                spawnMutation.mutate(
                  createMode === "template"
                    ? { template: templateName }
                    : { manifest_toml: manifestToml },
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
            <Button variant="secondary" onClick={() => setShowCreate(false)}>{t("common.cancel")}</Button>
          </div>
        </div>
      </Modal>

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
      <div className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[640px] sm:max-w-[90vw] max-h-[85vh] overflow-hidden flex flex-col" onClick={e => e.stopPropagation()}>
        <div className="px-6 py-4 border-b border-border-subtle flex items-center justify-between shrink-0">
          <div>
            <h3 className="text-lg font-black">{agentName}</h3>
            <p className="text-xs text-text-dim">Prompts & Experiments</p>
          </div>
          <button onClick={onClose} className="p-2 rounded-xl hover:bg-main" aria-label={t("common.close", { defaultValue: "Close" })}><X className="w-4 h-4" /></button>
        </div>
        
        <div className="px-6 py-3 border-b border-border-subtle flex gap-2 shrink-0">
          <button onClick={() => setActiveTab("versions")} className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${activeTab === "versions" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
            <FlaskConical className="w-3 h-3 inline mr-1" /> Versions
          </button>
          <button onClick={() => setActiveTab("experiments")} className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${activeTab === "experiments" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
            <GitBranch className="w-3 h-3 inline mr-1" /> Experiments
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-6">
          {activeTab === "versions" && (
            <div className="space-y-4">
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
                  <div className="bg-surface rounded-t-2xl sm:rounded-xl shadow-2xl border border-border-subtle p-6 w-full max-w-lg" onClick={e => e.stopPropagation()}>
                    <h4 className="font-bold mb-4">Create Prompt Version</h4>
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
                      <Button variant="primary" className="flex-1" onClick={() => createVersionMutation.mutate({ agentId, version: { system_prompt: newPromptSystemPrompt, description: newPromptDescription, version: (versionsQuery.data?.length || 0) + 1, content_hash: "", tools: [], variables: [], created_by: "dashboard" } }, { onSuccess: () => { setShowCreateVersion(false); setNewPromptSystemPrompt(""); setNewPromptDescription(""); } })} disabled={!newPromptSystemPrompt.trim()}>
                        Create
                      </Button>
                      <Button variant="secondary" onClick={() => setShowCreateVersion(false)}>Cancel</Button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}

          {activeTab === "experiments" && (
            <div className="space-y-4">
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
                  <div className="bg-surface rounded-t-2xl sm:rounded-xl shadow-2xl border border-border-subtle p-6 w-full max-w-lg" onClick={e => e.stopPropagation()}>
                    <h4 className="font-bold mb-4">Create Experiment</h4>
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
                      <Button variant="primary" className="flex-1" onClick={() => createExperimentMutation.mutate({ agentId, experiment: { name: newExperimentName, status: "draft", traffic_split: selectedVariantIds.map(() => Math.floor(100 / selectedVariantIds.length)), success_criteria: { require_user_helpful: true, require_no_tool_errors: true, require_non_empty: true }, variants: selectedVariantIds.map((vId, i) => { const ver = versions.find(v => v.id === vId); return { name: i === 0 ? "Control" : `Variant ${String.fromCharCode(65 + i)}`, prompt_version_id: vId, description: ver ? `v${ver.version}` : undefined }; }) } }, { onSuccess: () => { setShowCreateExperiment(false); setNewExperimentName(""); setSelectedVariantIds([]); } })} disabled={!newExperimentName.trim() || selectedVariantIds.length < 2}>
                        Create ({selectedVariantIds.length} variants)
                      </Button>
                      <Button variant="secondary" onClick={() => { setShowCreateExperiment(false); setSelectedVariantIds([]); }}>Cancel</Button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
