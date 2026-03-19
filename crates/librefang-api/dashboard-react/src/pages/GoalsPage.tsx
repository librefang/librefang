import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createGoal, listAgents, listGoals, updateGoal, deleteGoal, type GoalItem } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { Shield, Trash2, Edit2, Plus, Target, Zap, BookOpen } from "lucide-react";

const REFRESH_MS = 30000;

const EXAMPLE_GOALS = [
  { title: "部署生产环境", description: "配置并部署应用到生产服务器", status: "pending" as const },
  { title: "优化数据库查询", description: "分析并优化慢查询，提升性能", status: "pending" as const },
  { title: "编写API文档", description: "为所有API端点生成文档", status: "pending" as const },
];

export function GoalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [expandedById, setExpandedById] = useState<Record<string, boolean>>({});
  const [createDraft, setCreateDraft] = useState({ title: "", description: "", status: "pending" as string, progress: 0, parent_id: "", agent_id: "" });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState({ title: "", description: "", status: "pending" as string, progress: 0 });

  const goalsQuery = useQuery({ queryKey: ["goals", "list"], queryFn: listGoals, refetchInterval: REFRESH_MS });
  const agentsQuery = useQuery({ queryKey: ["agents", "list", "goals"], queryFn: listAgents });

  const createMutation = useMutation({ mutationFn: createGoal });
  const updateMutation = useMutation({ mutationFn: ({ id, data }: { id: string; data: any }) => updateGoal(id, data) });
  const deleteMutation = useMutation({ mutationFn: deleteGoal });
  const goals = goalsQuery.data ?? [];

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

  const handleAddExamples = async () => {
    try {
      for (const g of EXAMPLE_GOALS) {
        await createMutation.mutateAsync(g);
      }
      addToast(t("common.success"), "success");
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
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
    if (!window.confirm(t("common.confirm"))) return;
    try {
      await deleteMutation.mutateAsync(id);
      addToast(t("common.success"), "success");
      await queryClient.invalidateQueries({ queryKey: ["goals"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const handleStatusChange = async (id: string, status: string) => {
    try {
      await updateMutation.mutateAsync({ id, data: { status, progress: status === "completed" ? 100 : status === "in_progress" ? 50 : 0 } });
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

  const stats = {
    total: goals.length,
    completed: goals.filter(g => g.status === "completed").length,
    inProgress: goals.filter(g => g.status === "in_progress").length,
    pending: goals.filter(g => g.status === "pending").length,
  };

  const inputClass = "rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand outline-none transition-all";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.automation")}
        title={t("goals.title")}
        subtitle={t("goals.subtitle")}
        isFetching={goalsQuery.isFetching}
        onRefresh={() => void goalsQuery.refetch()}
        icon={<Shield className="h-4 w-4" />}
      />

      {goalsQuery.isLoading ? (
        <ListSkeleton rows={4} />
      ) : goals.length === 0 ? (
        <div className="flex flex-col items-center gap-6 py-12">
          <EmptyState
            title={t("common.no_data")}
            icon={<Shield className="h-6 w-6" />}
          />
          <Button variant="secondary" onClick={handleAddExamples}>
            <Plus className="h-4 w-4" />
            添加示例目标
          </Button>
        </div>
      ) : (
        <>
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
            {[{ label: t("goals.total"), value: stats.total, color: "text-brand" }, { label: t("goals.pending"), value: stats.pending, color: "text-text-dim" }, { label: t("goals.in_progress"), value: stats.inProgress, color: "text-warning" }, { label: t("goals.completed"), value: stats.completed, color: "text-success" }].map((s, i) => (
              <Card key={i} hover padding="md">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{s.label}</span>
                <div className="mt-1 flex items-baseline gap-2"><strong className={`text-3xl font-black tracking-tight ${s.color}`}>{s.value}</strong></div>
              </Card>
            ))}
          </div>

          <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
            <Card padding="lg">
              <h2 className="text-lg font-black tracking-tight">{t("goals.create_goal")}</h2>
              <form className="mt-6 flex flex-col gap-4" onSubmit={handleCreate}>
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
                <Button variant="ghost" size="sm" onClick={handleAddExamples}>
                  <Zap className="h-3.5 w-3.5" />
                  添加示例
                </Button>
              </div>
              <div className="space-y-2">
                {rows.map(r => (
                  <div key={r.goal.id} className="p-4 rounded-xl bg-main/40 border border-border-subtle hover:border-brand/30 transition-all" style={{ marginLeft: `${r.depth * 20}px` }}>
                    {editingId === r.goal.id ? (
                      <div className="flex flex-col gap-2">
                        <input value={editDraft.title} onChange={e => setEditDraft({...editDraft, title: e.target.value})} className={inputClass} placeholder="标题" />
                        <textarea value={editDraft.description} onChange={e => setEditDraft({...editDraft, description: e.target.value})} className={`${inputClass} resize-none`} rows={2} placeholder="描述" />
                        <div className="flex gap-2">
                          <select value={editDraft.status} onChange={e => setEditDraft({...editDraft, status: e.target.value})} className={inputClass}>
                            <option value="pending">待处理</option>
                            <option value="in_progress">进行中</option>
                            <option value="completed">已完成</option>
                          </select>
                          <input type="number" value={editDraft.progress} onChange={e => setEditDraft({...editDraft, progress: Number(e.target.value)})} className={inputClass} min={0} max={100} style={{ width: "80px" }} />
                          <Button variant="primary" size="sm" onClick={handleSaveEdit}>保存</Button>
                          <Button variant="ghost" size="sm" onClick={() => setEditingId(null)}>取消</Button>
                        </div>
                      </div>
                    ) : (
                      <div className="flex items-center justify-between gap-3">
                        <div className="flex items-center gap-3 flex-1 min-w-0">
                          {r.hasChildren && <button onClick={() => setExpandedById({...expandedById, [r.goal.id]: !expandedById[r.goal.id]})} className="text-text-dim font-bold hover:text-brand transition-colors w-5">{expandedById[r.goal.id] ? "−" : "+"}</button>}
                          <span className="text-sm font-black truncate">{r.goal.title}</span>
                          <Badge variant={r.goal.status === "completed" ? "success" : r.goal.status === "in_progress" ? "warning" : "default"}>
                            {r.goal.status === "in_progress" ? "进行中" : r.goal.status === "completed" ? "已完成" : "待处理"}
                          </Badge>
                          {r.goal.progress !== undefined && r.goal.progress > 0 && (
                            <span className="text-xs text-text-dim">{r.goal.progress}%</span>
                          )}
                        </div>
                        <div className="flex items-center gap-1">
                          <button onClick={() => handleStatusChange(r.goal.id, r.goal.status === "completed" ? "pending" : "completed")} className="p-1.5 rounded-lg hover:bg-brand/10 text-text-dim hover:text-brand transition-all" title="完成/重置">
                            <Target className="h-3.5 w-3.5" />
                          </button>
                          <button onClick={() => handleStartEdit(r.goal)} className="p-1.5 rounded-lg hover:bg-brand/10 text-text-dim hover:text-brand transition-all" title="编辑">
                            <Edit2 className="h-3.5 w-3.5" />
                          </button>
                          <button onClick={() => handleDelete(r.goal.id)} className="p-1.5 rounded-lg hover:bg-error/10 text-text-dim hover:text-error transition-all" title="删除">
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </Card>
          </div>
        </>
      )}
    </div>
  );
}
