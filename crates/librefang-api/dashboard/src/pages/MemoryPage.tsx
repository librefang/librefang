import { formatDateTime } from "../lib/datetime";
import { useState, useEffect, useMemo, useDeferredValue } from "react";
import { useTranslation } from "react-i18next";
import { useQueries, type UseQueryResult } from "@tanstack/react-query";
import { type MemoryStatsResponse } from "../api";
import {
  useMemoryStats,
  useMemoryConfig,
  useMemoryHealth,
  useMemorySearchOrList,
  agentKvMemoryQueryOptions,
} from "../lib/queries/memory";
import { useAgents } from "../lib/queries/agents";
import { useAddMemory, useUpdateMemory, useDeleteMemory, useCleanupMemories, useUpdateMemoryConfig } from "../lib/mutations/memory";
import { useStorageStatus } from "../lib/queries/storage";
import type { AgentItem, AgentKvPair } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Database, Search, Trash2, Plus, X, Sparkles, Zap, Clock, Edit2, Loader2, Settings } from "lucide-react";
import { StaggerList } from "../components/ui/StaggerList";

// Add Memory Dialog
function AddMemoryDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [content, setContent] = useState("");
  const [agentId, setAgentId] = useState("");
  const [level, setLevel] = useState("session");

  const addMutation = useAddMemory();

  const handleAdd = () => {
    addMutation.mutate(
      { content, level, agentId: agentId || undefined },
      {
        onSuccess: () => onClose(),
        onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
      }
    );
  };

  return (
    <DrawerPanel isOpen={true} onClose={onClose} title={t("memory.add_memory")} size="md">
      <div className="p-4 sm:p-6">
        <div className="space-y-4">
          <div>
            <label className="text-xs font-bold text-text-dim mb-1 block">{t("memory.content")}</label>
            <textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder={t("memory.content_placeholder")}
              rows={4}
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-primary focus:ring-1 focus:ring-primary/20 outline-none resize-none"
            />
          </div>

          <div>
            <label className="text-xs font-bold text-text-dim mb-1 block">{t("memory.level", { defaultValue: "Level" })}</label>
            <select
              value={level}
              onChange={(e) => setLevel(e.target.value)}
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-primary focus:ring-1 focus:ring-primary/20 outline-none"
            >
              <option value="user">{t("memory.user", { defaultValue: "user" })}</option>
              <option value="session">{t("memory.session", { defaultValue: "session" })}</option>
              <option value="agent">{t("memory.agent", { defaultValue: "agent" })}</option>
            </select>
          </div>

          <div>
            <label className="text-xs font-bold text-text-dim mb-1 block">{t("memory.agent_id")}</label>
            <input
              type="text"
              value={agentId}
              onChange={(e) => setAgentId(e.target.value)}
              placeholder={t("memory.agent_optional")}
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-primary focus:ring-1 focus:ring-primary/20 outline-none"
            />
          </div>
        </div>

        <div className="flex gap-3 mt-6">
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
          <Button variant="primary" className="flex-1" onClick={handleAdd} disabled={!content.trim() || addMutation.isPending}>
            {addMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
            {t("common.save")}
          </Button>
        </div>
      </div>
    </DrawerPanel>
  );
}

// Edit Memory Dialog
function EditMemoryDialog({ memory, onClose }: { memory: { id: string; content?: string }; onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [content, setContent] = useState(memory.content || "");

  const editMutation = useUpdateMemory();

  const handleSave = () => {
    editMutation.mutate(
      { id: memory.id, content },
      {
        onSuccess: () => onClose(),
        onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
      }
    );
  };

  return (
    <DrawerPanel isOpen={true} onClose={onClose} title={t("memory.edit_memory")} size="md">
      <div className="p-4 sm:p-6">
        <div>
          <label className="text-xs font-bold text-text-dim mb-1 block">{t("memory.content")}</label>
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            rows={6}
            className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-primary focus:ring-1 focus:ring-primary/20 outline-none resize-none"
          />
        </div>

        <div className="flex gap-3 mt-6">
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
          <Button variant="primary" className="flex-1" onClick={handleSave} disabled={!content.trim() || editMutation.isPending}>
            {editMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : t("common.save")}
          </Button>
        </div>
      </div>
    </DrawerPanel>
  );
}

// Memory Stats Card
function MemoryStats({ stats }: { stats: MemoryStatsResponse | null }) {
  const { t } = useTranslation();

  const kpis = useMemo(() => [
    { icon: Database, label: t("memory.total_memories"), value: stats?.total ?? 0, color: "text-primary", bg: "bg-primary/10" },
    { icon: Sparkles, label: t("memory.user", { defaultValue: "User" }), value: stats?.user_count ?? 0, color: "text-success", bg: "bg-success/10" },
    { icon: Clock, label: t("memory.session", { defaultValue: "Session" }), value: stats?.session_count ?? 0, color: "text-accent", bg: "bg-accent/10" },
    { icon: Zap, label: t("memory.agent", { defaultValue: "Agent" }), value: stats?.agent_count ?? 0, color: "text-warning", bg: "bg-warning/10" },
  ], [stats, t]);

  if (!stats) return null;

  return (
    <StaggerList className="grid grid-cols-2 md:grid-cols-4 gap-4">
      {kpis.map((kpi, i) => (
        <Card key={i} hover padding="md">
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
            <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}><kpi.icon className={`w-4 h-4 ${kpi.color}`} /></div>
          </div>
          <p className={`text-3xl font-black tracking-tight mt-2 ${kpi.color}`}>{kpi.value}</p>
        </Card>
      ))}
    </StaggerList>
  );
}

interface MemoryConfigForm {
  embedding_provider: string;
  embedding_model: string;
  embedding_api_key_env: string;
  decay_rate: string;
  pm_enabled: boolean;
  pm_auto_memorize: boolean;
  pm_auto_retrieve: boolean;
  pm_extraction_model: string;
  pm_max_retrieve: string;
}

function MemoryConfigDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const configQuery = useMemoryConfig();
  const updateConfig = useUpdateMemoryConfig();

  const [form, setForm] = useState<MemoryConfigForm | null>(null);

  useEffect(() => {
    if (!configQuery.data || form) return;
    setForm({
      embedding_provider: configQuery.data.embedding_provider || "",
      embedding_model: configQuery.data.embedding_model || "",
      embedding_api_key_env: configQuery.data.embedding_api_key_env || "",
      decay_rate: String(configQuery.data.decay_rate ?? 0.05),
      pm_enabled: configQuery.data.proactive_memory?.enabled ?? true,
      pm_auto_memorize: configQuery.data.proactive_memory?.auto_memorize ?? true,
      pm_auto_retrieve: configQuery.data.proactive_memory?.auto_retrieve ?? true,
      pm_extraction_model: configQuery.data.proactive_memory?.extraction_model || "",
      pm_max_retrieve: String(configQuery.data.proactive_memory?.max_retrieve ?? 10),
    });
  }, [configQuery.data, form]);

  const handleSave = async () => {
    if (!form) return;
    try {
      const decayRate = Number(form.decay_rate);
      const maxRetrieve = Number.parseInt(form.pm_max_retrieve, 10);

      await updateConfig.mutateAsync({
        embedding_provider: form.embedding_provider || undefined,
        embedding_model: form.embedding_model || undefined,
        embedding_api_key_env: form.embedding_api_key_env || undefined,
        decay_rate: Number.isFinite(decayRate) ? decayRate : 0.05,
        proactive_memory: {
          enabled: form.pm_enabled,
          auto_memorize: form.pm_auto_memorize,
          auto_retrieve: form.pm_auto_retrieve,
          extraction_model: form.pm_extraction_model || undefined,
          max_retrieve: Number.isFinite(maxRetrieve) ? maxRetrieve : 10,
        },
      });
      addToast(t("common.success"), "success");
      onClose();
    } catch (error) {
      addToast(error instanceof Error ? error.message : "Failed to save", "error");
    }
  };

  const inputCls = "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-primary";
  const labelCls = "text-[10px] font-bold uppercase tracking-widest text-text-dim mb-1 block";

  return (
    <DrawerPanel isOpen={true} onClose={onClose} title={t("memory.config_title", { defaultValue: "Memory Configuration" })} size="lg">
      <p className="text-xs text-text-dim -mt-2 mb-4">{t("memory.config_desc", { defaultValue: "Changes are written to config.toml. Restart required for full effect." })}</p>

      {configQuery.isLoading || !form ? (
        <div className="p-6 text-center"><Loader2 className="w-5 h-5 animate-spin mx-auto" /></div>
      ) : (
        <div className="space-y-4 max-h-[60vh] overflow-y-auto">
          {/* Embedding */}
          <div>
            <h4 className="text-xs font-bold mb-3">{t("memory.embedding_section", { defaultValue: "Embedding" })}</h4>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div>
                <span className={labelCls}>{t("memory.provider", { defaultValue: "Provider" })}</span>
                <select value={form.embedding_provider ?? ""} onChange={e => setForm({ ...form, embedding_provider: e.target.value })} className={inputCls}>
                  <option value="">{t("memory.auto_detect", { defaultValue: "Auto-detect" })}</option>
                  <option value="openai">{t("memory.provider_openai", { defaultValue: "OpenAI" })}</option>
                  <option value="ollama">{t("memory.provider_ollama", { defaultValue: "Ollama" })}</option>
                  <option value="vllm">{t("memory.provider_vllm", { defaultValue: "vLLM" })}</option>
                  <option value="lmstudio">{t("memory.provider_lmstudio", { defaultValue: "LM Studio" })}</option>
                  <option value="gemini">{t("memory.provider_gemini", { defaultValue: "Gemini" })}</option>
                  <option value="minimax">{t("memory.provider_minimax", { defaultValue: "MiniMax" })}</option>
                </select>
              </div>
              <div>
                <span className={labelCls}>{t("memory.model", { defaultValue: "Model" })}</span>
                <input value={form.embedding_model ?? ""} onChange={e => setForm({ ...form, embedding_model: e.target.value })}
                  placeholder="text-embedding-3-small" className={inputCls} />
              </div>
            </div>
            <div className="mt-2">
              <span className={labelCls}>{t("memory.api_key_env", { defaultValue: "API Key Env" })}</span>
              <input value={form.embedding_api_key_env ?? ""} onChange={e => setForm({ ...form, embedding_api_key_env: e.target.value })}
                placeholder="OPENAI_API_KEY" className={inputCls} />
              <p className="text-xs text-text-dim mt-1">
                {t("memory.api_key_env_hint", {
                  defaultValue: "Local providers (Ollama / vLLM / LM Studio) typically don't need a key — leave blank.",
                })}
              </p>
            </div>
          </div>

          {/* Proactive Memory */}
          <div>
            <h4 className="text-xs font-bold mb-3">{t("memory.proactive_memory", { defaultValue: "Proactive Memory" })}</h4>
            <div className="space-y-2">
              {[
                { key: "pm_enabled", label: t("memory.proactive_enabled", { defaultValue: "Enabled" }) },
                { key: "pm_auto_memorize", label: t("memory.auto_memorize", { defaultValue: "Auto Memorize" }) },
                { key: "pm_auto_retrieve", label: t("memory.auto_retrieve", { defaultValue: "Auto Retrieve" }) },
              ].map(opt => (
                <label key={opt.key} className="flex items-center justify-between rounded-lg bg-main/50 px-3 py-2">
                  <span className="text-xs font-medium">{opt.label}</span>
                  <button role="switch" aria-checked={!!form[opt.key as keyof MemoryConfigForm]} aria-label={opt.label} onClick={() => setForm({ ...form, [opt.key]: !form[opt.key as keyof MemoryConfigForm] })}
                    className={`w-10 h-5 rounded-full transition-colors ${form[opt.key as keyof MemoryConfigForm] ? "bg-brand" : "bg-border-subtle"}`}>
                    <div className={`w-4 h-4 rounded-full bg-white shadow transition-transform ${form[opt.key as keyof MemoryConfigForm] ? "translate-x-5" : "translate-x-0.5"}`} />
                  </button>
                </label>
              ))}
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-3">
              <div>
                <span className={labelCls}>{t("memory.extraction_model_label", { defaultValue: "Extraction Model" })}</span>
                <input value={form.pm_extraction_model ?? ""} onChange={e => setForm({ ...form, pm_extraction_model: e.target.value })}
                  placeholder="MiniMax-M2.7-highspeed" className={inputCls} />
              </div>
              <div>
                <span className={labelCls}>{t("memory.max_retrieve", { defaultValue: "Max Retrieve" })}</span>
                <input type="number" min={1} max={50} value={form.pm_max_retrieve ?? 10}
                  onChange={e => setForm({ ...form, pm_max_retrieve: e.target.value })} className={inputCls} />
              </div>
            </div>
          </div>

          {/* Decay */}
          <div>
            <span className={labelCls}>{t("memory.decay_rate", { defaultValue: "Decay Rate" })}</span>
            <input type="number" step={0.01} min={0} max={1} value={form.decay_rate ?? 0.05}
              onChange={e => setForm({ ...form, decay_rate: e.target.value })} className={inputCls} />
          </div>
        </div>
      )}

      <div className="flex gap-2 mt-6">
        <Button variant="primary" className="flex-1" onClick={handleSave} disabled={updateConfig.isPending}>
          {updateConfig.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : t("common.save")}
        </Button>
        <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
      </div>
    </DrawerPanel>
  );
}

// Truncate long KV values for table rendering — full value still available
// in the title attribute on hover.
function formatKvValue(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

const KV_VALUE_TRUNCATE = 200;
// Cap the hover-preview too — large KV values (multi-KB JSON blobs) would
// otherwise live in the DOM as a giant `title` attribute on every row,
// inflating page memory for what's only meant to be a quick peek.
const KV_TITLE_TRUNCATE = 2000;

// Receives the per-agent KV query result from AgentKvSection (a single
// `useQueries` observer batches all agents) so this row component stays
// presentational — no per-row hook subscription, no N+1 churn.
function AgentKvRows({ kvQuery }: { kvQuery: UseQueryResult<AgentKvPair[]> }) {
  const { t } = useTranslation();

  if (kvQuery.isLoading) {
    return (
      <tr>
        <td colSpan={4} className="px-3 py-2 text-xs text-text-dim">
          <Loader2 className="w-3.5 h-3.5 animate-spin inline" />
        </td>
      </tr>
    );
  }
  if (kvQuery.isError) {
    return (
      <tr>
        <td colSpan={4} className="px-3 py-2 text-xs text-error">
          {kvQuery.error instanceof Error ? kvQuery.error.message : t("common.error")}
        </td>
      </tr>
    );
  }

  const pairs = kvQuery.data ?? [];
  if (pairs.length === 0) {
    return (
      <tr>
        <td colSpan={4} className="px-3 py-2 text-xs text-text-dim/60 italic">
          {t("memory.kv_empty", { defaultValue: "No KV entries" })}
        </td>
      </tr>
    );
  }

  return (
    <>
      {pairs.map((pair: AgentKvPair) => {
        const formatted = formatKvValue(pair.value);
        const truncated =
          formatted.length > KV_VALUE_TRUNCATE
            ? formatted.slice(0, KV_VALUE_TRUNCATE) + "…"
            : formatted;
        const titlePreview =
          formatted.length > KV_TITLE_TRUNCATE
            ? formatted.slice(0, KV_TITLE_TRUNCATE) + "…"
            : formatted;
        return (
          <tr key={pair.key} className="border-t border-border-subtle/40">
            <td className="px-3 py-2 text-xs font-mono break-all">{pair.key}</td>
            <td className="px-3 py-2 text-xs font-mono text-text-dim break-all" title={titlePreview}>
              {truncated}
            </td>
            <td className="px-3 py-2 text-xs text-text-dim">{pair.source ?? "-"}</td>
            <td className="px-3 py-2 text-xs text-text-dim">
              {pair.created_at ? formatDateTime(pair.created_at) : "-"}
            </td>
          </tr>
        );
      })}
    </>
  );
}

function AgentKvSection({ agents }: { agents: AgentItem[] }) {
  const { t } = useTranslation();

  // Batch every per-agent KV lookup into a single useQueries observer instead
  // of mounting one `useAgentKvMemory` hook per row. Same number of network
  // requests (the API has no batch endpoint), but only one subscription point
  // — no N+1 React-Query churn, fewer re-renders, query results flow down as
  // props.
  const kvQueries = useQueries({
    queries: agents.map((agent) => agentKvMemoryQueryOptions(agent.id)),
  });

  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-sm font-bold">
        {t("memory.kv_section_title", { defaultValue: "Per-agent KV memory" })}
      </h3>
      {agents.length === 0 ? (
        <EmptyState
          title={t("memory.kv_no_agents", { defaultValue: "No agents available" })}
          icon={<Database className="h-6 w-6" />}
        />
      ) : (
        <div className="grid gap-4">
          {agents.map((agent, idx) => (
            <Card key={agent.id} padding="md">
              <div className="flex items-center gap-2 mb-3 flex-wrap">
                <h4 className="text-xs font-bold">{agent.name}</h4>
                <span className="text-[10px] font-mono text-text-dim">{agent.id.slice(0, 8)}</span>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-left">
                  <thead>
                    <tr className="text-[10px] font-bold uppercase tracking-widest text-text-dim/60">
                      <th className="px-3 py-2">{t("memory.kv_key", { defaultValue: "Key" })}</th>
                      <th className="px-3 py-2">{t("memory.kv_value", { defaultValue: "Value" })}</th>
                      <th className="px-3 py-2">{t("memory.kv_source", { defaultValue: "Source" })}</th>
                      <th className="px-3 py-2">{t("memory.created", { defaultValue: "Created" })}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <AgentKvRows kvQuery={kvQueries[idx]} />
                  </tbody>
                </table>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}

export function MemoryPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [search, setSearch] = useState("");
  const [levelFilter, setLevelFilter] = useState<string>("all");
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [showConfigDialog, setShowConfigDialog] = useState(false);
  useCreateShortcut(() => setShowAddDialog(true));
  const [editingMemory, setEditingMemory] = useState<{ id: string; content?: string } | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState<{ id: string } | null>(null);

  const memoryConfigQuery = useMemoryConfig();
  // Server-side liveness probe — distinct from "provider is configured".
  // Defaults to false while loading so a misconfigured backend can't flash a
  // green badge before the real health signal arrives.
  const memoryHealthQuery = useMemoryHealth();
  const embeddingAvailable = memoryHealthQuery.data ?? false;
  const memoryConfig = memoryConfigQuery.data
    ? {
        embedding_available: embeddingAvailable,
        embedding_provider: memoryConfigQuery.data.embedding_provider ?? "",
        embedding_model: memoryConfigQuery.data.embedding_model ?? "",
        extraction_model: memoryConfigQuery.data.proactive_memory?.extraction_model ?? "",
        proactive_memory_enabled: memoryConfigQuery.data.proactive_memory?.enabled ?? false,
      }
    : null;

  const deferredSearch = useDeferredValue(search);
  const memoryQuery = useMemorySearchOrList(deferredSearch);

  const statsQuery = useMemoryStats();
  const storageStatusQuery = useStorageStatus();
  const deleteMutation = useDeleteMemory();
  const cleanupMutation = useCleanupMemories();


  const memories = memoryQuery.data?.memories ?? [];
  const totalCount = memoryQuery.data?.total ?? 0;

  // Source of truth for "is proactive memory available right now":
  //   1. The /api/memory response carries `proactive_enabled` (preferred —
  //      reflects runtime store presence, not just config intent).
  //   2. Fall back to /api/memory/config while the list query is in flight
  //      so the UI doesn't flicker the proactive sections during load.
  // While both are still loading we default to `true` to avoid flashing
  // the "disabled" notice on first paint.
  const proactiveEnabled =
    memoryQuery.data?.proactive_enabled ??
    memoryConfig?.proactive_memory_enabled ??
    true;

  const filteredMemories = useMemo(() => {
    if (levelFilter === "all") return memories;
    return memories.filter(m => m.level === levelFilter);
  }, [memories, levelFilter]);

  const levels = useMemo(
    () => Array.from(new Set(memories.map(m => m.level).filter(Boolean))),
    [memories],
  );

  // Always-on per-agent KV view. Loaded regardless of proactive state so
  // the page remains useful even when proactive memory is disabled.
  const agentsQuery = useAgents();
  const agents = agentsQuery.data ?? [];

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("memory.cognitive_layer")}
        title={t("memory.title")}
        subtitle={t("memory.subtitle")}
        isFetching={memoryQuery.isFetching}
        onRefresh={() => void memoryQuery.refetch()}
        icon={<Database className="h-4 w-4" />}
        helpText={t("memory.help")}
        actions={
          <div className="flex items-center gap-1 sm:gap-2 flex-wrap">
            <Button variant="secondary" size="sm" onClick={() => setShowConfigDialog(true)}>
              <Settings className="w-4 h-4" />
            </Button>
            {proactiveEnabled && (
              <>
                <Button variant="secondary" size="sm" onClick={() => cleanupMutation.mutate(undefined, {
                  onSuccess: () => addToast(t("memory.cleanup_success", { defaultValue: "Cleanup complete" }), "success"),
                  onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
                })} disabled={cleanupMutation.isPending}>
                  {cleanupMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <Trash2 className="w-4 h-4" />}
                  <span className="hidden sm:inline">{t("memory.cleanup")}</span>
                </Button>
                <Button variant="primary" size="sm" onClick={() => setShowAddDialog(true)}>
                  <Plus className="w-4 h-4" />
                  <span className="hidden sm:inline ml-1">{t("memory.add")}</span>
                </Button>
              </>
            )}
          </div>
        }
      />

      {/* Proactive-disabled notice */}
      {!proactiveEnabled && (
        <Card padding="md">
          <p className="text-xs text-text-dim">
            {t("memory.proactive_disabled_notice", {
              defaultValue:
                "Proactive memory is disabled in config — showing per-agent KV memories instead.",
            })}
          </p>
        </Card>
      )}

      {/* Stats — proactive only */}
      {proactiveEnabled && (
        statsQuery.isError ? (
          <EmptyState
            title={t("common.error")}
            description={t("common.error_loading_stats", { defaultValue: "Failed to load memory stats" })}
            icon={<Database className="h-6 w-6" />}
          />
        ) : (
          <MemoryStats stats={statsQuery.data ?? null} />
        )
      )}

      {/* Storage Backend Status */}
      {storageStatusQuery.data && (
        <Card padding="md">
          <div className="flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
            <div className="flex items-center gap-1.5">
              <Database className="h-3 w-3 text-text-dim" />
              <span className="text-text-dim">{t("memory.storage_backend", { defaultValue: "Storage" })}:</span>
              <Badge variant="brand">{storageStatusQuery.data.backend_kind}</Badge>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.storage_ns", { defaultValue: "NS/DB" })}:</span>
              <span className="font-mono text-text-main">{storageStatusQuery.data.namespace}/{storageStatusQuery.data.database}</span>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.storage_connected", { defaultValue: "Connected" })}:</span>
              <Badge variant={storageStatusQuery.data.connected ? "success" : "error"}>
                {storageStatusQuery.data.connected ? t("common.yes", "Yes") : t("common.no", "No")}
              </Badge>
            </div>
            {storageStatusQuery.data.uar_linked && (
              <div className="flex items-center gap-1.5">
                <span className="text-text-dim">UAR:</span>
                <Badge variant="success">{t("memory.storage_uar_linked", { defaultValue: "linked" })}</Badge>
              </div>
            )}
          </div>
        </Card>
      )}

      {/* Memory Config */}
      {memoryConfigQuery.isError ? (
        <EmptyState
          title={t("common.error")}
          description={t("common.error_loading_config", { defaultValue: "Failed to load memory config" })}
          icon={<Settings className="h-6 w-6" />}
        />
      ) : memoryConfig && (
        <Card padding="md">
          <div className="flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.embedding_provider", { defaultValue: "Embedding" })}:</span>
              <Badge variant={memoryConfig.embedding_available ? "success" : "warning"}>
                {memoryConfig.embedding_provider || t("memory.auto", { defaultValue: "auto" })} / {memoryConfig.embedding_model || "-"}
              </Badge>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.extraction_model", { defaultValue: "Extraction" })}:</span>
              <Badge variant="brand">{memoryConfig.extraction_model || "-"}</Badge>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.proactive", { defaultValue: "Proactive" })}:</span>
              <Badge variant={memoryConfig.proactive_memory_enabled ? "success" : "default"}>
                {memoryConfig.proactive_memory_enabled ? t("common.on", { defaultValue: "ON" }) : t("common.off", { defaultValue: "OFF" })}
              </Badge>
            </div>
          </div>
        </Card>
      )}

      {/* Proactive memory: search + filters + list */}
      {proactiveEnabled && (
        <>
          {/* Filters */}
          <div className="flex flex-col sm:flex-row gap-3">
            <div className="flex-1">
              <Input
                value={search}
                onChange={(e) => { setSearch(e.target.value); }}
                placeholder={t("common.search")}
                leftIcon={<Search className="w-4 h-4" />}
                rightIcon={search && (
                  <button onClick={() => setSearch("")} className="hover:text-text-main" aria-label={t("common.clear_search", { defaultValue: "Clear search" })}>
                    <X className="w-3 h-3" />
                  </button>
                )}
              />
            </div>
            <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
              <button
                onClick={() => setLevelFilter("all")}
                className={`px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${levelFilter === "all" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
              >
                {t("memory.filter_all")}
              </button>
              {levels.map(level => (
                <button
                  key={level}
                  onClick={() => setLevelFilter(level || "all")}
                  className={`px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${levelFilter === level ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
                >
                  {level}
                </button>
              ))}
            </div>
          </div>

          {/* Count */}
          <div className="text-xs text-text-dim">
            {t("memory.showing", { count: filteredMemories.length, total: totalCount })}
          </div>

          {/* List */}
          {memoryQuery.isLoading ? (
            <div className="grid gap-4">
              {[1, 2, 3, 4, 5].map(i => <CardSkeleton key={i} />)}
            </div>
          ) : memoryQuery.isError ? (
            <EmptyState
              title={t("common.error")}
              description={t("common.error_loading_data", { defaultValue: "Failed to load memories" })}
              icon={<Database className="h-6 w-6" />}
            />
          ) : filteredMemories.length === 0 ? (
            <EmptyState
              title={search || levelFilter !== "all" ? t("common.no_data") : t("memory.no_memories")}
              icon={<Database className="h-6 w-6" />}
            />
          ) : (
            <div className="grid gap-4">
              {filteredMemories.map((m) => (
                <Card key={m.id} hover padding="md">
                  <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-1 sm:gap-2 mb-2">
                    <div className="flex items-center gap-2 min-w-0 flex-wrap">
                      <h2 className="text-xs sm:text-sm font-black truncate font-mono max-w-45 sm:max-w-none">{m.id}</h2>
                      <Badge variant={m.level === "user" ? "info" : m.level === "session" ? "warning" : m.level === "agent" ? "brand" : "default"}>
                        {m.level || t("memory.session", { defaultValue: "session" })}
                      </Badge>
                      {m.source && (
                        <Badge variant="default">{m.source}</Badge>
                      )}
                      {m.confidence != null && (
                        <Badge variant={m.confidence > 0.7 ? "success" : m.confidence > 0.3 ? "warning" : "default"}>
                          {Math.round(m.confidence * 100)}%
                        </Badge>
                      )}
                    </div>
                    <div className="flex items-center gap-1 shrink-0 self-end sm:self-auto">
                      <Button variant="ghost" size="sm" onClick={() => setEditingMemory(m)}>
                        <Edit2 className="h-3.5 w-3.5" />
                      </Button>
                      <Button variant="ghost" size="sm" className="text-error! hover:bg-error/10!" onClick={() => setDeleteConfirm({ id: m.id })}>
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </div>
                  <MarkdownContent className="text-xs text-text-dim leading-relaxed h-16 overflow-y-auto">
                    {m.content || t("common.no_data")}
                  </MarkdownContent>
                  <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[10px] text-text-dim/50">
                    {m.created_at && (
                      <span>{t("memory.created")}: {formatDateTime(m.created_at)}</span>
                    )}
                    {m.accessed_at && (
                      <span>{t("memory.last_access", { defaultValue: "Last access" })}: {formatDateTime(m.accessed_at)}</span>
                    )}
                    {m.access_count != null && m.access_count > 0 && (
                      <span>{t("memory.access_count", { defaultValue: "Accessed" })}: {m.access_count}x</span>
                    )}
                    {m.agent_id && (
                      <span>{t("memory.agent_label", { defaultValue: "Agent:" })} <span className="font-mono">{m.agent_id.slice(0, 8)}</span></span>
                    )}
                    {m.category && (
                      <span>{t("memory.category", { defaultValue: "Category" })}: {m.category}</span>
                    )}
                  </div>
                </Card>
              ))}
            </div>
          )}
        </>
      )}

      {/* Per-agent KV memory — always shown */}
      <AgentKvSection agents={agents} />


      {/* Dialogs */}
      {showAddDialog && <AddMemoryDialog onClose={() => setShowAddDialog(false)} />}
      {editingMemory && <EditMemoryDialog memory={editingMemory} onClose={() => setEditingMemory(null)} />}
      {showConfigDialog && <MemoryConfigDialog onClose={() => setShowConfigDialog(false)} />}

      <ConfirmDialog
        isOpen={deleteConfirm !== null}
        title={t("memory.delete_confirm_title", { defaultValue: "Delete Memory" })}
        message={t("memory.delete_confirm_message", { defaultValue: "This memory will be permanently deleted." })}
        tone="destructive"
        confirmLabel={t("common.delete", { defaultValue: "Delete" })}
        onConfirm={() => {
          if (deleteConfirm) {
            deleteMutation.mutate(deleteConfirm.id, {
              onSuccess: () => addToast(t("memory.delete_success", { defaultValue: "Memory deleted" }), "success"),
              onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
            });
          }
          setDeleteConfirm(null);
        }}
        onClose={() => setDeleteConfirm(null)}
      />
    </div>
  );
}
