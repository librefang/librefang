import { formatDateTime } from "../lib/datetime";
import { useState, useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { type MemoryStatsResponse } from "../api";
import { useMemoryStats, useMemoryConfig, useMemoryHealth, useMemorySearchOrList } from "../lib/queries/memory";
import { useAddMemory, useUpdateMemory, useDeleteMemory, useCleanupMemories, useUpdateMemoryConfig } from "../lib/mutations/memory";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { Modal } from "../components/ui/Modal";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { Database, Search, Trash2, Plus, X, Sparkles, Zap, Clock, Edit2, Loader2, Settings } from "lucide-react";

// Add Memory Dialog
function AddMemoryDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [content, setContent] = useState("");
  const [agentId, setAgentId] = useState("");

  const addMutation = useAddMemory();

  const handleAdd = () => {
    addMutation.mutate(
      { content, level, agentId: agentId || undefined },
      { onSuccess: () => onClose() }
    );
  };

  return (
    <Modal isOpen={true} onClose={onClose} title={t("memory.add_memory")} size="md">
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
    </Modal>
  );
}

// Edit Memory Dialog
function EditMemoryDialog({ memory, onClose }: { memory: { id: string; content?: string }; onClose: () => void }) {
  const { t } = useTranslation();
  const [content, setContent] = useState(memory.content || "");

  const editMutation = useUpdateMemory();

  const handleSave = () => {
    editMutation.mutate(
      { id: memory.id, content },
      { onSuccess: () => onClose() }
    );
  };

  return (
    <Modal isOpen={true} onClose={onClose} title={t("memory.edit_memory")} size="md">
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
    </Modal>
  );
}

// Memory Stats Card
function MemoryStats({ stats }: { stats: MemoryStatsResponse | null }) {
  const { t } = useTranslation();

  if (!stats) return null;

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-4 stagger-children">
      {[
        { icon: Database, label: t("memory.total_memories"), value: stats.total ?? 0, color: "text-brand", bg: "bg-brand/10" },
        { icon: Sparkles, label: t("memory.episodic"), value: (stats as any).episodic_count ?? 0, color: "text-success", bg: "bg-success/10" },
        { icon: Zap, label: t("memory.semantic"), value: (stats as any).semantic_count ?? 0, color: "text-warning", bg: "bg-warning/10" },
        { icon: Clock, label: t("memory.working"), value: (stats as any).working_count ?? 0, color: "text-accent", bg: "bg-accent/10" },
      ].map((kpi, i) => (
        <Card key={i} hover padding="md">
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
            <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}><kpi.icon className={`w-4 h-4 ${kpi.color}`} /></div>
          </div>
          <p className={`text-3xl font-black tracking-tight mt-2 ${kpi.color}`}>{kpi.value}</p>
        </Card>
      ))}
    </div>
  );
}

function MemoryConfigDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const configQuery = useMemoryConfig();
  const updateConfig = useUpdateMemoryConfig();

  const [form, setForm] = useState<Record<string, any> | null>(null);

  // Init form from API data
  useEffect(() => {
    const data = configQuery.data;
    if (data && !form) {
      setForm({
        embedding_provider: data.embedding_provider || "",
        embedding_model: data.embedding_model || "",
        embedding_api_key_env: data.embedding_api_key_env || "",
        decay_rate: data.decay_rate ?? 0.05,
        pm_enabled: data.proactive_memory?.enabled ?? true,
        pm_auto_memorize: data.proactive_memory?.auto_memorize ?? true,
        pm_auto_retrieve: data.proactive_memory?.auto_retrieve ?? true,
        pm_extraction_model: data.proactive_memory?.extraction_model || "",
        pm_max_retrieve: data.proactive_memory?.max_retrieve ?? 10,
      });
    }
  }, [configQuery.data, form]);

  const handleSave = async () => {
    if (!form) return;
    try {
      await updateConfig.mutateAsync({
        embedding_provider: form.embedding_provider || undefined,
        embedding_model: form.embedding_model || undefined,
        embedding_api_key_env: form.embedding_api_key_env || undefined,
        decay_rate: parseFloat(form.decay_rate) || 0.05,
        proactive_memory: {
          enabled: form.pm_enabled,
          auto_memorize: form.pm_auto_memorize,
          auto_retrieve: form.pm_auto_retrieve,
          extraction_model: form.pm_extraction_model || undefined,
          max_retrieve: parseInt(form.pm_max_retrieve) || 10,
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
    <div className="fixed inset-0 z-100 flex items-end sm:items-center justify-center bg-black/50 backdrop-blur-sm p-0 sm:p-4" onClick={onClose}>
      <div className="w-full max-w-lg rounded-t-2xl sm:rounded-2xl bg-surface border border-border-subtle shadow-2xl animate-fade-in-scale overflow-hidden" onClick={e => e.stopPropagation()}>
        <div className="p-6 pb-4">
          <h3 className="text-lg font-black">{t("memory.config_title", { defaultValue: "Memory Configuration" })}</h3>
          <p className="text-xs text-text-dim mt-0.5">{t("memory.config_desc", { defaultValue: "Changes are written to config.toml. Restart required for full effect." })}</p>
        </div>

        {configQuery.isLoading || !form ? (
          <div className="p-6 text-center"><Loader2 className="w-5 h-5 animate-spin mx-auto" /></div>
        ) : (
          <div className="px-6 pb-4 space-y-4 max-h-[60vh] overflow-y-auto">
            {/* Embedding */}
            <div>
              <h4 className="text-xs font-bold mb-3">Embedding</h4>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div>
                  <span className={labelCls}>Provider</span>
                  <select value={form.embedding_provider ?? ""} onChange={e => setForm({ ...form, embedding_provider: e.target.value })} className={inputCls}>
                    <option value="">Auto-detect</option>
                    <option value="openai">OpenAI</option>
                    <option value="ollama">Ollama</option>
                    <option value="gemini">Gemini</option>
                    <option value="minimax">MiniMax</option>
                  </select>
                </div>
                <div>
                  <span className={labelCls}>Model</span>
                  <input value={form.embedding_model ?? ""} onChange={e => setForm({ ...form, embedding_model: e.target.value })}
                    placeholder="text-embedding-3-small" className={inputCls} />
                </div>
              </div>
              <div className="mt-2">
                <span className={labelCls}>API Key Env</span>
                <input value={form.embedding_api_key_env ?? ""} onChange={e => setForm({ ...form, embedding_api_key_env: e.target.value })}
                  placeholder="OPENAI_API_KEY" className={inputCls} />
              </div>
            </div>

            {/* Proactive Memory */}
            <div>
              <h4 className="text-xs font-bold mb-3">Proactive Memory</h4>
              <div className="space-y-2">
                {[
                  { key: "pm_enabled", label: t("memory.proactive_enabled", { defaultValue: "Enabled" }) },
                  { key: "pm_auto_memorize", label: t("memory.auto_memorize", { defaultValue: "Auto Memorize" }) },
                  { key: "pm_auto_retrieve", label: t("memory.auto_retrieve", { defaultValue: "Auto Retrieve" }) },
                ].map(opt => (
                  <label key={opt.key} className="flex items-center justify-between rounded-lg bg-main/50 px-3 py-2">
                    <span className="text-xs font-medium">{opt.label}</span>
                    <button onClick={() => setForm({ ...form, [opt.key]: !form[opt.key] })}
                      className={`w-10 h-5 rounded-full transition-colors ${form[opt.key] ? "bg-brand" : "bg-border-subtle"}`}>
                      <div className={`w-4 h-4 rounded-full bg-white shadow transition-transform ${form[opt.key] ? "translate-x-5" : "translate-x-0.5"}`} />
                    </button>
                  </label>
                ))}
              </div>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-3">
                <div>
                  <span className={labelCls}>Extraction Model</span>
                  <input value={form.pm_extraction_model ?? ""} onChange={e => setForm({ ...form, pm_extraction_model: e.target.value })}
                    placeholder="MiniMax-M2.7-highspeed" className={inputCls} />
                </div>
                <div>
                  <span className={labelCls}>Max Retrieve</span>
                  <input type="number" min={1} max={50} value={form.pm_max_retrieve ?? 10}
                    onChange={e => setForm({ ...form, pm_max_retrieve: e.target.value })} className={inputCls} />
                </div>
              </div>
            </div>

            {/* Decay */}
            <div>
              <span className={labelCls}>Decay Rate</span>
              <input type="number" step={0.01} min={0} max={1} value={form.decay_rate ?? 0.05}
                onChange={e => setForm({ ...form, decay_rate: e.target.value })} className={inputCls} />
            </div>
          </div>
        )}

        <div className="flex gap-2 p-6 pt-3">
          <Button variant="primary" className="flex-1" onClick={handleSave} disabled={updateConfig.isPending}>
            {updateConfig.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : t("common.save")}
          </Button>
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
        </div>
      </div>
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

  const memoryQuery = useMemorySearchOrList(search);

  const statsQuery = useMemoryStats();
  const deleteMutation = useDeleteMemory();
  const cleanupMutation = useCleanupMemories();


  const memories = memoryQuery.data?.memories ?? [];
  const totalCount = memoryQuery.data?.total ?? 0;

  const filteredMemories = useMemo(
    () => memories.filter(m => {
      const matchesLevel = levelFilter === "all" || m.level === levelFilter;
      return matchesLevel;
    }),
    [memories, levelFilter],
  );

  const levels = useMemo(
    () => Array.from(new Set(memories.map(m => m.level).filter(Boolean))),
    [memories],
  );

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
          </div>
        }
      />

      {/* Stats */}
      <MemoryStats stats={statsQuery.data ?? null} />

      {/* Memory Config */}
      {memoryConfig && (
        <Card padding="md">
          <div className="flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.embedding_provider", { defaultValue: "Embedding" })}:</span>
              <Badge variant={memoryConfig.embedding_available ? "success" : "warning"}>
                {memoryConfig.embedding_provider || "auto"} / {memoryConfig.embedding_model || "-"}
              </Badge>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.extraction_model", { defaultValue: "Extraction" })}:</span>
              <Badge variant="brand">{memoryConfig.extraction_model || "-"}</Badge>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("memory.proactive", { defaultValue: "Proactive" })}:</span>
              <Badge variant={memoryConfig.proactive_memory_enabled ? "success" : "default"}>
                {memoryConfig.proactive_memory_enabled ? "ON" : "OFF"}
              </Badge>
            </div>
          </div>
        </Card>
      )}

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
                  <Badge variant={m.level === "user" ? "success" : m.level === "agent" ? "warning" : "info"}>
                    {m.level || "session"}
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
                  <Button variant="ghost" size="sm" className="text-error! hover:bg-error/10!" onClick={() => deleteMutation.mutate(m.id, {
                    onSuccess: () => addToast(t("memory.delete_success", { defaultValue: "Memory deleted" }), "success"),
                    onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
                  })}>
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
                  <span>Agent: <span className="font-mono">{m.agent_id.slice(0, 8)}</span></span>
                )}
                {m.category && (
                  <span>{t("memory.category", { defaultValue: "Category" })}: {m.category}</span>
                )}
              </div>
            </Card>
          ))}
        </div>
      )}


      {/* Dialogs */}
      {showAddDialog && <AddMemoryDialog onClose={() => setShowAddDialog(false)} />}
      {editingMemory && <EditMemoryDialog memory={editingMemory} onClose={() => setEditingMemory(null)} />}
      {showConfigDialog && <MemoryConfigDialog onClose={() => setShowConfigDialog(false)} />}
    </div>
  );
}
