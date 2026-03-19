import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createGoal, listAgents, listGoals, type GoalItem } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { useUIStore } from "../lib/store";
import { Shield } from "lucide-react";

const REFRESH_MS = 30000;

export function GoalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [expandedById, setExpandedById] = useState<Record<string, boolean>>({});
  const [createDraft, setCreateDraft] = useState({ title: "", description: "", status: "pending", progress: 0, parent_id: "", agent_id: "" });

  const goalsQuery = useQuery({ queryKey: ["goals", "list"], queryFn: listGoals, refetchInterval: REFRESH_MS });
  const agentsQuery = useQuery({ queryKey: ["agents", "list", "goals"], queryFn: listAgents });

  const createMutation = useMutation({ mutationFn: createGoal });
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

  const statusColors: Record<string, string> = {
    pending: "border-text-dim/30 bg-text-dim/10 text-text-dim",
    in_progress: "border-warning/30 bg-warning/10 text-warning",
    completed: "border-success/30 bg-success/10 text-success",
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
      />

      {goalsQuery.isLoading ? (
        <ListSkeleton rows={4} />
      ) : goals.length === 0 ? (
        <EmptyState
          title={t("common.no_data")}
          icon={<Shield className="h-6 w-6" />}
        />
      ) : (
        <>
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
            {[{ label: t("goals.total"), value: stats.total, color: "text-brand" }, { label: t("goals.pending"), value: stats.pending, color: "text-text-dim" }, { label: t("goals.in_progress"), value: stats.inProgress, color: "text-warning" }, { label: t("goals.completed"), value: stats.completed, color: "text-success" }].map((s, i) => (
              <article key={i} className="rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm hover:border-brand/30 transition-all">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{s.label}</span>
                <div className="mt-1 flex items-baseline gap-2"><strong className={`text-3xl font-black tracking-tight ${s.color}`}>{s.value}</strong></div>
              </article>
            ))}
          </div>

          <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
            <aside className="h-fit rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
              <h2 className="text-lg font-black tracking-tight">{t("goals.create_goal")}</h2>
              <form className="mt-6 flex flex-col gap-4" onSubmit={handleCreate}>
                <input value={createDraft.title} onChange={e => setCreateDraft({...createDraft, title: e.target.value})} placeholder={t("goals.goal_title_placeholder")} className={inputClass} />
                <textarea value={createDraft.description} onChange={e => setCreateDraft({...createDraft, description: e.target.value})} placeholder={t("goals.goal_desc_placeholder")} className={`${inputClass} resize-none`} rows={3} />
                <button type="submit" disabled={createMutation.isPending || !createDraft.title.trim()} className="mt-2 rounded-xl bg-brand py-3 text-sm font-bold text-white shadow-lg hover:opacity-90 disabled:opacity-50 transition-all">{createMutation.isPending ? t("common.loading") : t("goals.create_goal")}</button>
              </form>
            </aside>

            <article className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("goals.goal_tree")}</h2>
              <div className="space-y-2 mt-6">
                {rows.map(r => (
                  <div key={r.goal.id} className="p-4 rounded-xl bg-main/40 border border-border-subtle hover:border-brand/30 transition-all" style={{ marginLeft: `${r.depth * 20}px` }}>
                    <div className="flex items-center gap-3">
                      {r.hasChildren && <button onClick={() => setExpandedById({...expandedById, [r.goal.id]: !expandedById[r.goal.id]})} className="text-text-dim font-bold hover:text-brand transition-colors">{expandedById[r.goal.id] ? "−" : "+"}</button>}
                      <span className="text-sm font-black">{r.goal.title}</span>
                      <span className={`text-[10px] font-bold px-2 py-0.5 rounded-lg border uppercase ${statusColors[r.goal.status] || statusColors.pending}`}>{r.goal.status}</span>
                    </div>
                  </div>
                ))}
              </div>
            </article>
          </div>
        </>
      )}
    </div>
  );
}
