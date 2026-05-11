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
import { useAutoDreamStatus } from "../lib/queries/autoDream";
import { useAddMemory, useUpdateMemory, useDeleteMemory, useCleanupMemories, useUpdateMemoryConfig } from "../lib/mutations/memory";
import { useTriggerAutoDream, useAbortAutoDream, useSetAutoDreamEnabled } from "../lib/mutations/autoDream";
import type { AgentItem, AgentKvPair, AutoDreamAgentStatus } from "../api";
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
import { Database, Search, Trash2, Plus, X, Sparkles, Zap, Clock, Edit2, Loader2, Settings, Moon, Play, Square, CheckCircle, XCircle } from "lucide-react";
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
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none resize-none"
            />
          </div>

          <div>
            <label className="text-xs font-bold text-text-dim mb-1 block">{t("memory.level", { defaultValue: "Level" })}</label>
            <select
              value={level}
              onChange={(e) => setLevel(e.target.value)}
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
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
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
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
            className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none resize-none"
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
    { icon: Database, label: t("memory.total_memories"), value: stats?.total ?? 0, color: "text-brand", bg: "bg-brand/10" },
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

  const inputCls = "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";
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

      {/* Auto-Dream (background memory consolidation) */}
      <AutoDreamSection />

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

/* ------------------------------------------------------------------ */
/*  Auto-Dream Section                                                 */
/* ------------------------------------------------------------------ */

// Format an epoch-ms into a short human-readable "N hours ago" / "in N
// hours" label. Returns "never" when ts is 0 or undefined — the status
// endpoint omits `next_eligible_at_ms` for never-dreamed agents, and
// `last_consolidated_at_ms` is 0 in the same case.
const _rtfCache = new Map<string, Intl.RelativeTimeFormat>();

function getRelativeTimeFormat(locale: string): Intl.RelativeTimeFormat {
  let rtf = _rtfCache.get(locale);
  if (!rtf) {
    rtf = new Intl.RelativeTimeFormat(locale, {
      numeric: "auto",
      style: "narrow",
    });
    _rtfCache.set(locale, rtf);
  }
  return rtf;
}

function formatRelativeMs(
  ts: number | undefined,
  now: number,
  locale: string,
  t: (key: string) => string,
): string {
  if (ts === undefined || ts === 0) return t("common.never");
  const diff = ts - now;
  const absMinutes = Math.abs(diff) / 60_000;
  const rtf = getRelativeTimeFormat(locale);
  if (absMinutes < 60) {
    return rtf.format(Math.round(diff / 60_000), "minute");
  }
  const absHours = absMinutes / 60;
  if (absHours < 24) {
    return rtf.format(parseFloat((diff / 3_600_000).toFixed(1)), "hour");
  }
  return rtf.format(parseFloat((diff / 86_400_000).toFixed(1)), "day");
}

// Human-readable duration for effective_min_hours. Switches between hours,
// days, and weeks so "every 168h" renders as "every 1w" etc.
function formatHours(hours: number, t: (key: string) => string): string {
  if (hours < 1) return `${(hours * 60).toFixed(0)}${t("settings.auto_dream_dur_minute")}`;
  if (hours < 24) return `${hours % 1 === 0 ? hours.toFixed(0) : hours.toFixed(1)}${t("settings.auto_dream_dur_hour")}`;
  const days = hours / 24;
  if (days < 7) return `${days % 1 === 0 ? days.toFixed(0) : days.toFixed(1)}${t("settings.auto_dream_dur_day")}`;
  const weeks = days / 7;
  return `${weeks % 1 === 0 ? weeks.toFixed(0) : weeks.toFixed(1)}${t("settings.auto_dream_dur_week")}`;
}

function AutoDreamAgentRow({
  agent,
  disabled,
  onTrigger,
  onAbort,
  onToggle,
  triggerPending,
  abortPending,
  togglePending,
}: {
  agent: AutoDreamAgentStatus;
  disabled: boolean;
  onTrigger: (id: string) => void;
  onAbort: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
  triggerPending: boolean;
  abortPending: boolean;
  togglePending: boolean;
}) {
  const { t, i18n } = useTranslation();
  const now = Date.now();
  const progress = agent.progress;
  const running = progress?.status === "running";
  const lastTurn = progress?.turns[progress.turns.length - 1];
  const optedIn = agent.auto_dream_enabled;

  return (
    <div className="rounded-lg border border-border-subtle/50 bg-main">
      <div className="flex items-center justify-between px-3 py-2">
        <div className="flex items-start gap-2 min-w-0 flex-1">
          <Moon
            className={`w-4 h-4 shrink-0 mt-0.5 ${
              optedIn
                ? running
                  ? "text-purple-400 animate-pulse"
                  : "text-purple-400"
                : "text-text-dim"
            }`}
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <p className="text-sm font-medium truncate">{agent.agent_name}</p>
              {progress && (
                <Badge
                  variant={
                    progress.status === "running"
                      ? "info"
                      : progress.status === "completed"
                      ? "success"
                      : progress.status === "aborted"
                      ? "warning"
                      : "error"
                  }
                >
                  {t(`settings.auto_dream_status_${progress.status}`, progress.status)}
                </Badge>
              )}
            </div>
            {optedIn ? (
              <p className="text-[11px] text-text-dim">
                {t("settings.auto_dream_last", "Last")}:{" "}
                {formatRelativeMs(agent.last_consolidated_at_ms, now, i18n.language, t)}
                {" · "}
                {t("settings.auto_dream_next", "Next")}:{" "}
                {formatRelativeMs(agent.next_eligible_at_ms, now, i18n.language, t)}
                {" · "}
                {agent.effective_min_sessions > 0 ? (
                  <span
                    title={t(
                      "settings.auto_dream_sessions_progress_title",
                      "Sessions touched since last dream / required threshold",
                    )}
                  >
                    {agent.sessions_since_last}/{agent.effective_min_sessions}{" "}
                    {t("settings.auto_dream_sessions_since", "sessions since")}
                  </span>
                ) : (
                  <>
                    {agent.sessions_since_last}{" "}
                    {t("settings.auto_dream_sessions_since", "sessions since")}
                  </>
                )}
                {" · "}
                <span
                  title={t(
                    "settings.auto_dream_effective_title",
                    "Resolved threshold — manifest override or global default",
                  )}
                >
                  {t("settings.auto_dream_every", "every")}{" "}
                  {formatHours(agent.effective_min_hours, t)}
                </span>
              </p>
            ) : running ? (
              // Agent was toggled off while a manual dream was already in
              // flight. Keep the operator informed — the run continues to
              // completion or abort, and the abort button above stays live.
              <p className="text-[11px] text-text-dim italic">
                {t(
                  "settings.auto_dream_opt_out_running",
                  "Disabled mid-dream — the current run will finish or can be aborted.",
                )}
              </p>
            ) : (
              <p className="text-[11px] text-text-dim italic">
                {t(
                  "settings.auto_dream_opt_in_hint",
                  "Not enrolled — toggle on to include in the scheduler.",
                )}
              </p>
            )}
          </div>
        </div>
        <div className="flex gap-2 shrink-0 items-center">
          <label
            className="flex items-center gap-1.5 cursor-pointer select-none"
            title={t("settings.auto_dream_toggle_title", "Opt this agent in or out")}
          >
            <input
              type="checkbox"
              checked={optedIn}
              disabled={togglePending}
              onChange={(e) => onToggle(agent.agent_id, e.target.checked)}
              className="w-3.5 h-3.5 accent-purple-500"
            />
            <span className="text-[11px] text-text-dim">
              {optedIn
                ? t("settings.auto_dream_enrolled", "Enrolled")
                : t("settings.auto_dream_not_enrolled", "Off")}
            </span>
          </label>
          {running && agent.can_abort && (
            // Surface the abort affordance even when the agent has been
            // toggled off mid-dream — otherwise the in-flight operation
            // keeps spending tokens with no UI to stop it.
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onAbort(agent.agent_id)}
              disabled={abortPending}
            >
              <Square className="w-3.5 h-3.5 mr-1.5" />
              {t("settings.auto_dream_abort", "Abort")}
            </Button>
          )}
          {optedIn && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onTrigger(agent.agent_id)}
              disabled={triggerPending || disabled || running}
              title={disabled ? t("settings.auto_dream_off", "Disabled") : undefined}
            >
              <Play className="w-3.5 h-3.5 mr-1.5" />
              {t("settings.auto_dream_trigger", "Dream now")}
            </Button>
          )}
        </div>
      </div>

      {progress && (progress.status !== "completed" || progress.memories_touched.length > 0) && (
        <div className="px-3 pb-2 pt-1 border-t border-border-subtle/30 space-y-1">
          <p className="text-[10px] text-text-dim">
            <span className="uppercase tracking-wider">
              {t("settings.auto_dream_phase", "Phase")}:
            </span>{" "}
            <span className="font-mono">{progress.phase}</span>
            {" · "}
            {progress.tool_use_count}{" "}
            {t("settings.auto_dream_tool_calls", "tool calls")}
            {progress.memories_touched.length > 0 && (
              <>
                {" · "}
                {progress.memories_touched.length}{" "}
                {t("settings.auto_dream_memories_touched", "memories touched")}
              </>
            )}
          </p>
          {lastTurn && lastTurn.text && (
            <p className="text-[11px] text-text-muted line-clamp-2 italic">
              &ldquo;{lastTurn.text}&rdquo;
            </p>
          )}
          {progress.error && (
            <p className="text-[11px] text-red-500">
              <XCircle className="w-3 h-3 inline mr-1" />
              {progress.error}
            </p>
          )}
          {/* Cache-hit visibility. Since the forkedAgent migration, dreams
              fork off the parent turn and hit Anthropic's prompt cache on
              the (system + tools + messages) prefix. Surfacing the hit
              rate here lets operators see the actual cost win — the
              whole reason the forkedAgent PR exists. Only shown for
              completed dreams (usage is populated then) and only when
              there actually was input (avoids 0/0 noise). */}
          {progress.usage && progress.usage.input_tokens > 0 && (
            <p className="text-[10px] text-text-dim">
              <span className="uppercase tracking-wider">
                {t("settings.auto_dream_cache", "Cache")}:
              </span>{" "}
              {(() => {
                const u = progress.usage!;
                const totalIn =
                  u.input_tokens +
                  u.cache_read_input_tokens +
                  u.cache_creation_input_tokens;
                const hitPct =
                  totalIn > 0
                    ? Math.round((u.cache_read_input_tokens / totalIn) * 100)
                    : 0;
                return (
                  <span
                    title={t(
                      "settings.auto_dream_cache_title",
                      "Prompt cache hit rate for this dream — higher means more of the prefix came from Anthropic's cache instead of being re-billed.",
                    )}
                  >
                    <span className="font-mono">{hitPct}%</span>
                    {" "}
                    ({u.cache_read_input_tokens.toLocaleString()}/
                    {totalIn.toLocaleString()} tok)
                  </span>
                );
              })()}
              {typeof progress.usage.cost_usd === "number" && (
                <>
                  {" · "}
                  <span
                    title={t(
                      "settings.auto_dream_cost_title",
                      "Measured provider cost for this dream turn (input + output, cached tokens billed at the reduced rate).",
                    )}
                  >
                    ${progress.usage.cost_usd.toFixed(5)}
                  </span>
                </>
              )}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function AutoDreamSection() {
  const { t } = useTranslation();
  const statusQuery = useAutoDreamStatus();
  const trigger = useTriggerAutoDream();
  const abort = useAbortAutoDream();
  const setEnabled = useSetAutoDreamEnabled();
  const [error, setError] = useState<string | null>(null);
  const [lastMsg, setLastMsg] = useState<string | null>(null);

  const status = statusQuery.data;

  const onTrigger = async (agentId: string) => {
    setError(null);
    setLastMsg(null);
    try {
      const outcome = await trigger.mutateAsync(agentId);
      setLastMsg(outcome.fired ? t("settings.auto_dream_fired", "Consolidation fired") : outcome.reason);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const onAbort = async (agentId: string) => {
    setError(null);
    setLastMsg(null);
    try {
      const outcome = await abort.mutateAsync(agentId);
      setLastMsg(outcome.aborted ? t("settings.auto_dream_aborted", "Abort signalled") : outcome.reason);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const onToggle = async (agentId: string, enabled: boolean) => {
    setError(null);
    setLastMsg(null);
    try {
      await setEnabled.mutateAsync({ agentId, enabled });
      setLastMsg(
        enabled
          ? t("settings.auto_dream_enrolled_ok", "Agent enrolled")
          : t("settings.auto_dream_unenrolled_ok", "Agent unenrolled"),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface">
      <div className="px-5 py-3 border-b border-border-subtle/50 flex items-center justify-between">
        <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
          {t("settings.auto_dream", "Auto-Dream")}
        </p>
        {status && (
          <Badge variant={status.enabled ? "success" : "default"}>
            {status.enabled
              ? t("settings.auto_dream_on", "Enabled")
              : t("settings.auto_dream_off", "Disabled")}
          </Badge>
        )}
      </div>
      <div className="px-5 py-3">
        <p className="text-xs text-text-dim mb-3">
          {t(
            "settings.auto_dream_desc",
            "Periodically asks opt-in agents to consolidate their memory. Configure in config.toml via [auto_dream] enabled + per-agent auto_dream_enabled.",
          )}
        </p>

        {statusQuery.isLoading && (
          <p className="text-xs text-text-dim">{t("common.loading", "Loading…")}</p>
        )}
        {statusQuery.isError && (
          <p className="text-xs text-red-500">
            {t("settings.auto_dream_load_err", "Failed to load auto-dream status")}
          </p>
        )}

        {status && status.agents.length === 0 && (
          <p className="text-xs text-text-dim italic">
            {t(
              "settings.auto_dream_no_agents_registered",
              "No agents registered yet. Create an agent first, then toggle it on here.",
            )}
          </p>
        )}

        {status && status.agents.length > 0 && (
          <div className="space-y-2">
            {status.agents.map((a) => (
              <AutoDreamAgentRow
                key={a.agent_id}
                agent={a}
                disabled={!status.enabled}
                onTrigger={onTrigger}
                onAbort={onAbort}
                onToggle={onToggle}
                triggerPending={trigger.isPending && trigger.variables === a.agent_id}
                abortPending={abort.isPending && abort.variables === a.agent_id}
                togglePending={setEnabled.isPending && setEnabled.variables?.agentId === a.agent_id}
              />
            ))}
          </div>
        )}

        {lastMsg && (
          <p className="text-xs text-green-500 mt-2">
            <CheckCircle className="w-3 h-3 inline mr-1" />
            {lastMsg}
          </p>
        )}
        {error && (
          <p className="text-xs text-red-500 mt-2">
            <XCircle className="w-3 h-3 inline mr-1" />
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
