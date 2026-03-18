import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createGoal, deleteGoal, listAgents, listGoals, updateGoal, type GoalItem } from "../api";

const REFRESH_MS = 30000;

export function GoalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [expandedById, setExpandedById] = useState<Record<string, boolean>>({});
  const [createDraft, setCreateDraft] = useState({ title: "", description: "", status: "pending", progress: 0, parent_id: "", agent_id: "" });

  const goalsQuery = useQuery({ queryKey: ["goals", "list"], queryFn: listGoals, refetchInterval: REFRESH_MS });
  const agentsQuery = useQuery({ queryKey: ["agents", "list", "goals"], queryFn: listAgents });

  const createMutation = useMutation({ mutationFn: createGoal });
  const goals = goalsQuery.data ?? [];
  const agents = agentsQuery.data ?? [];

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
    const result: any[] = [];
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
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" /></svg>
            {t("nav.automation")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("goals.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("goals.subtitle")}</p>
        </div>
        <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void goalsQuery.refetch()}>
          <svg className={`h-3.5 w-3.5 ${goalsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" /></svg>
          {t("common.refresh")}
        </button>
      </header>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[{ label: t("goals.total"), value: stats.total, color: "brand" }, { label: t("goals.pending"), value: stats.pending, color: "text-dim" }, { label: t("goals.in_progress"), value: stats.inProgress, color: "warning" }, { label: t("goals.completed"), value: stats.completed, color: "success" }].map((s, i) => (
          <article key={i} className="rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm">
            <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{s.label}</span>
            <div className="mt-1 flex items-baseline gap-2"><strong className={`text-3xl font-black tracking-tight text-${s.color}`}>{s.value}</strong></div>
          </article>
        ))}
      </div>

      <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
        <aside className="h-fit rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight">{t("goals.create_goal")}</h2>
          <form className="mt-6 flex flex-col gap-4" onSubmit={(e) => e.preventDefault()}>
            <input value={createDraft.title} onChange={e => setCreateDraft({...createDraft, title: e.target.value})} placeholder={t("goals.goal_title_placeholder")} className={inputClass} />
            <textarea value={createDraft.description} onChange={e => setCreateDraft({...createDraft, description: e.target.value})} placeholder={t("goals.goal_desc_placeholder")} className={`${inputClass} resize-none`} rows={3} />
            <button type="submit" className="mt-2 rounded-xl bg-brand py-3 text-sm font-bold text-white shadow-lg">{t("goals.create_goal")}</button>
          </form>
        </aside>

        <article className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight mb-1">{t("goals.goal_tree")}</h2>
          <div className="space-y-2 mt-6">
            {rows.map(r => (
              <div key={r.goal.id} className="p-4 rounded-xl bg-main/40 border border-border-subtle hover:border-brand/30 transition-all" style={{ marginLeft: `${r.depth * 20}px` }}>
                <div className="flex items-center gap-3">
                  {r.hasChildren && <button onClick={() => setExpandedById({...expandedById, [r.goal.id]: !expandedById[r.goal.id]})} className="text-text-dim font-bold">{expandedById[r.goal.id] ? "−" : "+"}</button>}
                  <span className="text-sm font-black">{r.goal.title}</span>
                  <span className="text-[10px] font-bold px-2 py-0.5 rounded-lg border border-brand/20 bg-brand/10 text-brand uppercase">{r.goal.status}</span>
                </div>
              </div>
            ))}
          </div>
        </article>
      </div>
    </div>
  );
}
