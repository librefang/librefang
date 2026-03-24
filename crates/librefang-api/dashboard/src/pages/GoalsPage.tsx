import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createGoal, listGoals, listGoalTemplates, updateGoal, deleteGoal, type GoalItem, type GoalTemplate } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { Shield, Trash2, Edit2, Plus, Target, Rocket, Bot, Database, Users, AlertTriangle, Loader2, CheckCircle2, Clock, Play, ChevronDown, ChevronRight } from "lucide-react";

const REFRESH_MS = 30000;

const TEMPLATE_ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  rocket: Rocket,
  bot: Bot,
  shield: Shield,
  database: Database,
  users: Users,
  alert: AlertTriangle,
};

export function GoalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [expandedById, setExpandedById] = useState<Record<string, boolean>>({});
  const [createDraft, setCreateDraft] = useState({ title: "", description: "", status: "pending" as string, progress: 0, parent_id: "", agent_id: "" });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState({ title: "", description: "", status: "pending" as string, progress: 0 });
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const goalsQuery = useQuery({ queryKey: ["goals", "list"], queryFn: listGoals, refetchInterval: REFRESH_MS });
  const templatesQuery = useQuery({ queryKey: ["goals", "templates"], queryFn: listGoalTemplates });
  const [applyingTemplate, setApplyingTemplate] = useState<string | null>(null);

  const createMutation = useMutation({ mutationFn: createGoal });
  const updateMutation = useMutation({ mutationFn: ({ id, data }: { id: string; data: any }) => updateGoal(id, data) });
  const deleteMutation = useMutation({ mutationFn: deleteGoal });
  const goals = goalsQuery.data ?? [];
  const templates = templatesQuery.data ?? [];

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!createDraft.title.trim()) return;
    try {
      await createMutation.mutateAsync(createDraft);
      addToast(t("common.success"), "success");
      setCreateDraft({ title: "", description: "", status: "pending", progress: 0, parent_id: "", agent_id: "" });
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const handleApplyTemplate = async (tpl: GoalTemplate) => {
    setApplyingTemplate(tpl.id);
    try {
      for (const g of tpl.goals) {
        await createMutation.mutateAsync(g);
      }
      addToast(t("common.success"), "success");
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    } finally {
      setApplyingTemplate(null);
    }
  };

  const handleStartEdit = (goal: GoalItem) => {
    setEditingId(goal.id);
    setEditDraft({
      title: goal.title || "",
      description: goal.description || "",
      status: goal.status || "pending",
      progress: goal.progress || 0
    });
  };

  const handleSaveEdit = async () => {
    if (!editingId || !editDraft.title.trim()) return;
    try {
      await updateMutation.mutateAsync({ id: editingId, data: editDraft });
      addToast(t("common.success"), "success");
      setEditingId(null);
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteMutation.mutateAsync(id);
      addToast(t("common.success"), "success");
      setConfirmDeleteId(null);
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const nextStatus = (current: string) => {
    if (current === "pending") return "in_progress";
    if (current === "in_progress") return "completed";
    return "pending";
  };

  const handleStatusChange = async (id: string, current: string) => {
    const status = nextStatus(current);
    try {
      await updateMutation.mutateAsync({ id, data: { status, progress: status === "completed" ? 100 : status === "in_progress" ? 50 : 0 } });
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const handleClearAll = async () => {
    try {
      for (const g of goals) {
        await deleteMutation.mutateAsync(g.id);
      }
      addToast(t("common.success"), "success");
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const rows = useMemo(() => {
    const roots: GoalItem[] = [];
    const childrenByParent = new Map<string, GoalItem[]>();
    for (const goal of goals) {
      if (goal.parent_id) {
        const list = childrenByParent.get(goal.parent_id) ?? [];
        list.push(goal);
        childrenByParent.set(goal.parent_id, list);
      } else roots.push(goal);
    }
    const result: { goal: GoalItem; depth: number; hasChildren: boolean }[] = [];
    function walk(goal: GoalItem, depth: number) {
      const children = childrenByParent.get(goal.id) ?? [];
      result.push({ goal, depth, hasChildren: children.length > 0 });
      if (expandedById[goal.id]) for (const child of children) walk(child, depth + 1);
    }
    for (const root of roots) walk(root, 0);
    return result;
  }, [expandedById, goals]);

  const stats = useMemo(() => ({
    total: goals.length,
    completed: goals.filter(g => g.status === "completed").length,
    inProgress: goals.filter(g => g.status === "in_progress").length,
    pending: goals.filter(g => g.status === "pending").length,
    pct: goals.length > 0 ? Math.round((goals.filter(g => g.status === "completed").length / goals.length) * 100) : 0,
  }), [goals]);
  const [showClearConfirm, setShowClearConfirm] = useState(false);

  const inputClass = "rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand outline-none transition-colors";

  const statusLabel = (status: string) => {
    if (status === "in_progress") return t("goals.in_progress");
    if (status === "completed") return t("goals.completed");
    return t("goals.pending");
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.automation")}
        title={t("goals.title")}
        subtitle={t("goals.subtitle")}
        isFetching={goalsQuery.isFetching}
        onRefresh={() => void goalsQuery.refetch()}
        icon={<Shield className="h-4 w-4" />}
        helpText={t("goals.help")}
      />

      {goalsQuery.isLoading ? (
        <ListSkeleton rows={4} />
      ) : goals.length === 0 ? (
        <div className="flex flex-col gap-6">
          <div className="text-center py-8">
            <div className="w-14 h-14 rounded-2xl bg-brand/10 flex items-center justify-center mx-auto mb-4">
              <Target className="h-7 w-7 text-brand" />
            </div>
            <h3 className="text-lg font-black tracking-tight mb-1">{t("goals.pick_template")}</h3>
            <p className="text-sm text-text-dim">{t("goals.pick_template_desc")}</p>
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 stagger-children">
            {templates.map((tpl) => {
              const Icon = TEMPLATE_ICONS[tpl.icon] ?? Target;
              const isApplying = applyingTemplate === tpl.id;
              return (
                <Card key={tpl.id} hover padding="lg" className="flex flex-col">
                  <div className="flex items-start gap-3 mb-3">
                    <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
                      <Icon className="w-5 h-5 text-brand" />
                    </div>
                    <div className="min-w-0">
                      <h4 className="text-sm font-black tracking-tight">{tpl.name}</h4>
                      <p className="text-xs text-text-dim mt-0.5">{tpl.description}</p>
                    </div>
                  </div>
                  <div className="flex-1 space-y-1.5 mb-4">
                    {tpl.goals.map((g, i) => (
                      <div key={i} className="flex items-center gap-2 text-xs text-text-dim">
                        <span className="w-5 h-5 rounded-md bg-main flex items-center justify-center text-[10px] font-bold shrink-0">{i + 1}</span>
                        <span className="truncate">{g.title}</span>
                      </div>
                    ))}
                  </div>
                  <Button
                    variant="secondary"
                    size="sm"
                    className="w-full"
                    disabled={isApplying || applyingTemplate !== null}
                    onClick={() => handleApplyTemplate(tpl)}
                  >
                    {isApplying ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
                    {isApplying ? t("common.loading") : t("goals.use_template")}
                  </Button>
                </Card>
              );
            })}
          </div>
        </div>
      ) : (
        <>
          {/* KPI row */}
          <div className="grid grid-cols-2 gap-2 sm:gap-4 xl:grid-cols-4 stagger-children">
            {[
              { label: t("goals.total"), value: stats.total, color: "text-brand", bg: "bg-brand/10", icon: Target },
              { label: t("goals.pending"), value: stats.pending, color: "text-text-dim", bg: "bg-main", icon: Clock },
              { label: t("goals.in_progress"), value: stats.inProgress, color: "text-warning", bg: "bg-warning/10", icon: Play },
              { label: t("goals.completed"), value: stats.completed, color: "text-success", bg: "bg-success/10", icon: CheckCircle2 },
            ].map((s, i) => (
              <Card key={i} hover padding="md">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{s.label}</span>
                  <div className={`w-8 h-8 rounded-lg ${s.bg} flex items-center justify-center`}>
                    <s.icon className={`w-4 h-4 ${s.color}`} />
                  </div>
                </div>
                <div className="mt-2"><strong className={`text-3xl font-black tracking-tight ${s.color}`}>{s.value}</strong></div>
              </Card>
            ))}
          </div>

          {/* Overall progress */}
          <Card padding="md">
            <div className="flex items-center justify-between mb-2">
              <span className="text-xs font-bold text-text-dim">{t("goals.overall_progress")}</span>
              <span className="text-sm font-black text-brand">{stats.pct}%</span>
            </div>
            <div className="h-2.5 rounded-full bg-main overflow-hidden">
              <div
                className="h-full rounded-full bg-gradient-to-r from-brand to-success transition-all duration-700"
                style={{ width: `${stats.pct}%` }}
              />
            </div>
            <div className="flex items-center justify-between mt-2 text-[10px] text-text-dim">
              <span>{stats.completed} / {stats.total} {t("goals.completed").toLowerCase()}</span>
              <div className="flex items-center gap-2">
                {showClearConfirm ? (
                  <>
                    <span className="text-error">{t("goals.clear_all_confirm")}</span>
                    <button onClick={() => { handleClearAll(); setShowClearConfirm(false); }} className="text-error font-bold hover:underline">{t("common.confirm")}</button>
                    <button onClick={() => setShowClearConfirm(false)} className="hover:underline">{t("common.cancel")}</button>
                  </>
                ) : (
                  <button onClick={() => setShowClearConfirm(true)} className="text-text-dim hover:text-error transition-colors">{t("goals.clear_all")}</button>
                )}
              </div>
            </div>
          </Card>

          {/* Create + Goal tree */}
          <div className="grid gap-6 lg:grid-cols-[320px_1fr] xl:grid-cols-[360px_1fr]">
            <Card padding="lg" hover>
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Plus className="w-4 h-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("goals.create_goal")}</h2>
              </div>
              <form className="flex flex-col gap-4" onSubmit={handleCreate}>
                <input value={createDraft.title} onChange={e => setCreateDraft({...createDraft, title: e.target.value})} placeholder={t("goals.goal_title_placeholder")} className={inputClass} />
                <textarea value={createDraft.description} onChange={e => setCreateDraft({...createDraft, description: e.target.value})} placeholder={t("goals.goal_desc_placeholder")} className={`${inputClass} resize-none`} rows={3} />
                <Button type="submit" variant="primary" disabled={createMutation.isPending || !createDraft.title.trim()} className="mt-2">
                  {createMutation.isPending ? t("common.loading") : t("goals.create_goal")}
                </Button>
              </form>
            </Card>

            <Card padding="lg">
              <div className="flex justify-between items-center mb-4">
                <h2 className="text-lg font-black tracking-tight">{t("goals.goal_tree")}</h2>
              </div>
              <div className="space-y-2">
                {rows.map(r => {
                  const status = r.goal.status || "pending";
                  const progress = r.goal.progress ?? 0;
                  const statusIcon = status === "completed"
                    ? <CheckCircle2 className="h-4 w-4 text-success" />
                    : status === "in_progress"
                      ? <Play className="h-4 w-4 text-warning" />
                      : <Clock className="h-4 w-4 text-text-dim/40" />;
                  return (
                    <div key={r.goal.id} className="rounded-xl bg-main/40 border border-border-subtle hover:border-brand/30 transition-colors" style={{ marginLeft: `${r.depth * 16}px` }}>
                      {editingId === r.goal.id ? (
                        <div className="p-3 sm:p-4 flex flex-col gap-2">
                          <input value={editDraft.title} onChange={e => setEditDraft({...editDraft, title: e.target.value})} className={inputClass} placeholder={t("goals.title_label")} />
                          <textarea value={editDraft.description} onChange={e => setEditDraft({...editDraft, description: e.target.value})} className={`${inputClass} resize-none`} rows={2} placeholder={t("goals.desc_label")} />
                          <div className="flex flex-wrap gap-2">
                            <select value={editDraft.status} onChange={e => setEditDraft({...editDraft, status: e.target.value})} className={`${inputClass} flex-1 min-w-[120px]`}>
                              <option value="pending">{t("goals.pending")}</option>
                              <option value="in_progress">{t("goals.in_progress")}</option>
                              <option value="completed">{t("goals.completed")}</option>
                            </select>
                            <input type="number" value={editDraft.progress} onChange={e => setEditDraft({...editDraft, progress: Number(e.target.value)})} className={inputClass} min={0} max={100} style={{ width: "80px" }} />
                            <Button variant="primary" size="sm" onClick={handleSaveEdit}>{t("common.save")}</Button>
                            <Button variant="ghost" size="sm" onClick={() => setEditingId(null)}>{t("common.cancel")}</Button>
                          </div>
                        </div>
                      ) : confirmDeleteId === r.goal.id ? (
                        <div className="p-3 sm:p-4 flex items-center justify-between gap-3">
                          <span className="text-sm text-text-dim">{t("goals.delete_confirm")}</span>
                          <div className="flex items-center gap-2">
                            <Button variant="primary" size="sm" onClick={() => handleDelete(r.goal.id)} className="bg-error! hover:bg-error/80!">{t("common.confirm")}</Button>
                            <Button variant="ghost" size="sm" onClick={() => setConfirmDeleteId(null)}>{t("common.cancel")}</Button>
                          </div>
                        </div>
                      ) : (
                        <div className="p-3 sm:p-4">
                          <div className="flex items-center justify-between gap-2 sm:gap-3">
                            <div className="flex items-center gap-2 flex-1 min-w-0">
                              {r.hasChildren && (
                                <button onClick={() => setExpandedById({...expandedById, [r.goal.id]: !expandedById[r.goal.id]})} className="text-text-dim hover:text-brand transition-colors shrink-0">
                                  {expandedById[r.goal.id] ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                                </button>
                              )}
                              <button
                                onClick={() => handleStatusChange(r.goal.id, status)}
                                className="shrink-0 hover:scale-110 transition-transform"
                                title={t("goals.toggle_reset")}
                              >
                                {statusIcon}
                              </button>
                              <span className={`text-sm font-bold truncate ${status === "completed" ? "line-through text-text-dim" : ""}`}>
                                {r.goal.title}
                              </span>
                              <Badge variant={status === "completed" ? "success" : status === "in_progress" ? "warning" : "default"} className="shrink-0">
                                {statusLabel(status)}
                              </Badge>
                            </div>
                            <div className="flex items-center gap-1 shrink-0">
                              <button onClick={() => handleStartEdit(r.goal)} className="p-1.5 rounded-lg hover:bg-brand/10 text-text-dim hover:text-brand transition-colors" title={t("common.edit")}>
                                <Edit2 className="h-3.5 w-3.5" />
                              </button>
                              <button onClick={() => setConfirmDeleteId(r.goal.id)} className="p-1.5 rounded-lg hover:bg-error/10 text-text-dim hover:text-error transition-colors" title={t("common.delete")}>
                                <Trash2 className="h-3.5 w-3.5" />
                              </button>
                            </div>
                          </div>
                          {/* Description */}
                          {r.goal.description && (
                            <p className="text-xs text-text-dim mt-1.5 ml-[calc(1rem+4px)] line-clamp-2">{r.goal.description}</p>
                          )}
                          {/* Progress bar */}
                          {progress > 0 && status !== "completed" && (
                            <div className="mt-2 ml-[calc(1rem+4px)]">
                              <div className="flex items-center gap-2">
                                <div className="flex-1 h-1.5 rounded-full bg-main overflow-hidden">
                                  <div
                                    className={`h-full rounded-full transition-all duration-500 ${status === "in_progress" ? "bg-warning" : "bg-brand"}`}
                                    style={{ width: `${progress}%` }}
                                  />
                                </div>
                                <span className="text-[10px] font-mono text-text-dim">{progress}%</span>
                              </div>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </Card>
          </div>
        </>
      )}
    </div>
  );
}
