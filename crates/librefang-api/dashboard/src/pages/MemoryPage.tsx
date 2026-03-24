import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { listMemories, deleteMemory, getMemoryStats, addMemoryFromText, updateMemory, cleanupMemories, decayMemories, type MemoryStatsResponse } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { useUIStore } from "../lib/store";
import { Database, Search, Trash2, Plus, X, Sparkles, Zap, Clock, RefreshCw, Edit2, Loader2, BarChart3 } from "lucide-react";

const REFRESH_MS = 30000;

// Add Memory Dialog
function AddMemoryDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [content, setContent] = useState("");
  const [agentId, setAgentId] = useState("");
  const [level, setLevel] = useState("episodic");

  const addMutation = useMutation({
    mutationFn: () => addMemoryFromText(content, agentId || undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["memory"] });
      onClose();
    }
  });

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4 bg-black/50 backdrop-blur-xl backdrop-saturate-150" onClick={onClose}>
      <div className="bg-surface rounded-2xl border border-border-subtle w-full sm:max-w-md p-4 sm:p-6 rounded-t-2xl sm:rounded-2xl shadow-2xl animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-black">{t("memory.add_memory")}</h3>
          <button onClick={onClose} className="p-1 hover:bg-main/30 rounded-lg">
            <X className="w-5 h-5 text-text-dim" />
          </button>
        </div>

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

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs font-bold text-text-dim mb-1 block">{t("memory.level")}</label>
              <select
                value={level}
                onChange={(e) => setLevel(e.target.value)}
                className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
              >
                <option value="episodic">{t("memory.episodic")}</option>
                <option value="semantic">{t("memory.semantic")}</option>
                <option value="working">{t("memory.working")}</option>
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
        </div>

        <div className="flex gap-3 mt-6">
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
          <Button variant="primary" className="flex-1" onClick={() => addMutation.mutate()} disabled={!content.trim() || addMutation.isPending}>
            {addMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
            {t("common.save")}
          </Button>
        </div>
      </div>
    </div>
  );
}

// Edit Memory Dialog
function EditMemoryDialog({ memory, onClose }: { memory: { id: string; content?: string }; onClose: () => void }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [content, setContent] = useState(memory.content || "");

  const editMutation = useMutation({
    mutationFn: () => updateMemory(memory.id, content),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["memory"] });
      onClose();
    }
  });

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4 bg-black/50 backdrop-blur-xl backdrop-saturate-150" onClick={onClose}>
      <div className="bg-surface rounded-2xl border border-border-subtle w-full sm:max-w-md p-4 sm:p-6 rounded-t-2xl sm:rounded-2xl shadow-2xl animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-black">{t("memory.edit_memory")}</h3>
          <button onClick={onClose} className="p-1 hover:bg-main/30 rounded-lg">
            <X className="w-5 h-5 text-text-dim" />
          </button>
        </div>

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
          <Button variant="primary" className="flex-1" onClick={() => editMutation.mutate()} disabled={!content.trim() || editMutation.isPending}>
            {editMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : t("common.save")}
          </Button>
        </div>
      </div>
    </div>
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

export function MemoryPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [search, setSearch] = useState("");
  const [levelFilter, setLevelFilter] = useState<string>("all");
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [editingMemory, setEditingMemory] = useState<{ id: string; content?: string } | null>(null);
  const [showStats, setShowStats] = useState(true);

  const memoryQuery = useQuery({ queryKey: ["memory", "list"], queryFn: () => listMemories(), refetchInterval: REFRESH_MS });
  const statsQuery = useQuery({ queryKey: ["memory", "stats"], queryFn: () => getMemoryStats(), refetchInterval: REFRESH_MS * 2 });

  const deleteMutation = useMutation({
    mutationFn: deleteMemory,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["memory"] });
      addToast(t("common.success"), "success");
    }
  });

  const cleanupMutation = useMutation({
    mutationFn: cleanupMemories,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["memory"] });
      addToast(t("common.success"), "success");
    }
  });

  const decayMutation = useMutation({
    mutationFn: decayMemories,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["memory"] });
      addToast(t("common.success"), "success");
    }
  });

  const memories = memoryQuery.data?.memories ?? [];
  const totalCount = memoryQuery.data?.total ?? 0;

  const filteredMemories = memories.filter(m => {
    const matchesSearch = !search ||
      m.id.toLowerCase().includes(search.toLowerCase()) ||
      (m.content || "").toLowerCase().includes(search.toLowerCase());
    const matchesLevel = levelFilter === "all" || m.level === levelFilter;
    return matchesSearch && matchesLevel;
  });

  const levels = Array.from(new Set(memories.map(m => m.level).filter(Boolean)));

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("memory.cognitive_layer")}
        title={t("memory.title")}
        subtitle={t("memory.subtitle")}
        isFetching={memoryQuery.isFetching}
        onRefresh={() => void memoryQuery.refetch()}
        icon={<Database className="h-4 w-4" />}
        actions={
          <div className="flex items-center gap-1 sm:gap-2 flex-wrap">
            <Button variant="secondary" size="sm" onClick={() => setShowStats(!showStats)}>
              <BarChart3 className="w-4 h-4" />
            </Button>
            <Button variant="secondary" size="sm" onClick={() => cleanupMutation.mutate()} disabled={cleanupMutation.isPending}>
              {cleanupMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <Trash2 className="w-4 h-4" />}
              <span className="hidden sm:inline">{t("memory.cleanup")}</span>
            </Button>
            <Button variant="secondary" size="sm" onClick={() => decayMutation.mutate()} disabled={decayMutation.isPending}>
              {decayMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <RefreshCw className="w-4 h-4" />}
              <span className="hidden sm:inline">{t("memory.decay")}</span>
            </Button>
            <Button variant="primary" size="sm" onClick={() => setShowAddDialog(true)}>
              <Plus className="w-4 h-4" />
              <span className="hidden sm:inline ml-1">{t("memory.add")}</span>
            </Button>
          </div>
        }
      />

      {/* Stats */}
      {showStats && <MemoryStats stats={statsQuery.data ?? null} />}

      {/* Filters */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex-1">
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("common.search")}
            leftIcon={<Search className="w-4 h-4" />}
            rightIcon={search && (
              <button onClick={() => setSearch("")} className="hover:text-text-main">
                <X className="w-3 h-3" />
              </button>
            )}
          />
        </div>
        <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
          <button
            onClick={() => setLevelFilter("all")}
            className={`px-3 py-1.5 rounded-md text-xs font-bold transition-all ${levelFilter === "all" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
          >
            {t("memory.filter_all")}
          </button>
          {levels.map(level => (
            <button
              key={level}
              onClick={() => setLevelFilter(level || "all")}
              className={`px-3 py-1.5 rounded-md text-xs font-bold transition-all ${levelFilter === level ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
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
                  <h2 className="text-xs sm:text-sm font-black truncate font-mono max-w-[180px] sm:max-w-none">{m.id}</h2>
                  <Badge variant={m.level === "episodic" ? "success" : m.level === "semantic" ? "warning" : "info"}>
                    {m.level || "Vector"}
                  </Badge>
                  {(m as any).importance !== undefined && (
                    <Badge variant={(m as any).importance > 0.7 ? "error" : (m as any).importance > 0.3 ? "warning" : "default"}>
                      {Math.round((m as any).importance * 100)}%
                    </Badge>
                  )}
                </div>
                <div className="flex items-center gap-1 shrink-0 self-end sm:self-auto">
                  <Button variant="ghost" size="sm" onClick={() => setEditingMemory(m)}>
                    <Edit2 className="h-3.5 w-3.5" />
                  </Button>
                  <Button variant="ghost" size="sm" className="text-error! hover:bg-error/10!" onClick={() => deleteMutation.mutate(m.id)}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </div>
              <p className="text-xs text-text-dim line-clamp-3 leading-relaxed whitespace-pre-wrap">{m.content || t("common.no_data")}</p>
              {m.created_at && (
                <div className="mt-2 text-[10px] text-text-dim/50">
                  {t("memory.created")}: {new Date(m.created_at).toLocaleString()}
                </div>
              )}
            </Card>
          ))}
        </div>
      )}

      {/* Dialogs */}
      {showAddDialog && <AddMemoryDialog onClose={() => setShowAddDialog(false)} />}
      {editingMemory && <EditMemoryDialog memory={editingMemory} onClose={() => setEditingMemory(null)} />}
    </div>
  );
}
