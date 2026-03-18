import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import { createGoal, deleteGoal, listAgents, listGoals, updateGoal, type GoalItem } from "../api";

const REFRESH_MS = 30000;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

interface GoalRow {
  goal: GoalItem;
  depth: number;
  hasChildren: boolean;
}

interface GoalDraft {
  title: string;
  description: string;
  status: string;
  progress: number;
  parent_id: string;
  agent_id: string;
}

function statusClass(status?: string): string {
  const value = (status ?? "").toLowerCase();
  if (value === "completed") return "border-success/20 bg-success/10 text-success";
  if (value === "in_progress") return "border-warning/20 bg-warning/10 text-warning";
  if (value === "cancelled") return "border-border-subtle bg-surface text-text-dim";
  return "border-brand/20 bg-brand/10 text-brand";
}

function statusLabel(status?: string): string {
  if (status === "in_progress") return "In Progress";
  if (status === "completed") return "Completed";
  if (status === "cancelled") return "Cancelled";
  return "Pending";
}

function clampProgress(value: number): number {
  if (Number.isNaN(value)) return 0;
  return Math.max(0, Math.min(100, Math.floor(value)));
}

function emptyDraft(): GoalDraft {
  return {
    title: "",
    description: "",
    status: "pending",
    progress: 0,
    parent_id: "",
    agent_id: ""
  };
}

export function GoalsPage() {
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [expandedById, setExpandedById] = useState<Record<string, boolean>>({});
  const [createDraft, setCreateDraft] = useState<GoalDraft>(emptyDraft());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState<GoalDraft>(emptyDraft());
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const goalsQuery = useQuery({
    queryKey: ["goals", "list"],
    queryFn: listGoals,
    refetchInterval: REFRESH_MS
  });
  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "goals-helper"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const createMutation = useMutation({
    mutationFn: createGoal
  });
  const updateMutation = useMutation({
    mutationFn: ({ goalId, payload }: { goalId: string; payload: Parameters<typeof updateGoal>[1] }) =>
      updateGoal(goalId, payload)
  });
  const deleteMutation = useMutation({
    mutationFn: deleteGoal
  });

  const goals = goalsQuery.data ?? [];
  const agents = agentsQuery.data ?? [];

  const goalById = useMemo(() => {
    const map = new Map<string, GoalItem>();
    for (const goal of goals) {
      map.set(goal.id, goal);
    }
    return map;
  }, [goals]);

  const rows = useMemo<GoalRow[]>(() => {
    const childrenByParent = new Map<string, GoalItem[]>();
    const roots: GoalItem[] = [];

    for (const goal of goals) {
      if (goal.parent_id && goalById.has(goal.parent_id)) {
        const list = childrenByParent.get(goal.parent_id) ?? [];
        list.push(goal);
        childrenByParent.set(goal.parent_id, list);
      } else {
        roots.push(goal);
      }
    }

    for (const [, list] of childrenByParent) {
      list.sort((a, b) => (a.created_at ?? "").localeCompare(b.created_at ?? ""));
    }
    roots.sort((a, b) => (a.created_at ?? "").localeCompare(b.created_at ?? ""));

    const result: GoalRow[] = [];
    const visited = new Set<string>();

    function walk(goal: GoalItem, depth: number) {
      if (visited.has(goal.id)) return;
      visited.add(goal.id);
      const children = childrenByParent.get(goal.id) ?? [];
      result.push({
        goal,
        depth,
        hasChildren: children.length > 0
      });
      if (children.length === 0) return;
      if (!expandedById[goal.id]) return;
      for (const child of children) {
        walk(child, depth + 1);
      }
    }

    for (const root of roots) {
      walk(root, 0);
    }

    for (const goal of goals) {
      if (!visited.has(goal.id)) {
        walk(goal, 0);
      }
    }
    return result;
  }, [expandedById, goalById, goals]);

  const stats = useMemo(() => {
    const total = goals.length;
    const completed = goals.filter((goal) => goal.status === "completed").length;
    const inProgress = goals.filter((goal) => goal.status === "in_progress").length;
    const pending = goals.filter((goal) => goal.status === "pending").length;
    return { total, completed, inProgress, pending };
  }, [goals]);

  const agentNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const agent of agents) {
      map.set(agent.id, agent.name);
    }
    return map;
  }, [agents]);

  const error = (() => {
    if (goalsQuery.error instanceof Error) return goalsQuery.error.message;
    if (agentsQuery.error instanceof Error) return agentsQuery.error.message;
    return "";
  })();

  async function refreshGoals() {
    await queryClient.invalidateQueries({ queryKey: ["goals"] });
    await goalsQuery.refetch();
  }

  function toggleExpand(goalId: string) {
    setExpandedById((current) => ({ ...current, [goalId]: !current[goalId] }));
  }

  function startEdit(goal: GoalItem) {
    setEditingId(goal.id);
    setEditDraft({
      title: goal.title ?? "",
      description: goal.description ?? "",
      status: goal.status ?? "pending",
      progress: clampProgress(goal.progress ?? 0),
      parent_id: goal.parent_id ?? "",
      agent_id: goal.agent_id ?? ""
    });
  }

  async function handleCreateGoal(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!createDraft.title.trim() || createMutation.isPending) return;
    try {
      await createMutation.mutateAsync({
        title: createDraft.title.trim(),
        description: createDraft.description.trim(),
        status: createDraft.status,
        progress: clampProgress(createDraft.progress),
        ...(createDraft.parent_id ? { parent_id: createDraft.parent_id } : {}),
        ...(createDraft.agent_id ? { agent_id: createDraft.agent_id } : {})
      });
      setFeedback({ type: "ok", text: "Goal created." });
      setCreateDraft(emptyDraft());
      await refreshGoals();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to create goal."
      });
    }
  }

  async function handleSaveEdit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!editingId || !editDraft.title.trim() || updateMutation.isPending) return;
    try {
      await updateMutation.mutateAsync({
        goalId: editingId,
        payload: {
          title: editDraft.title.trim(),
          description: editDraft.description.trim(),
          status: editDraft.status,
          progress: clampProgress(editDraft.progress),
          parent_id: editDraft.parent_id || null,
          agent_id: editDraft.agent_id || null
        }
      });
      setFeedback({ type: "ok", text: "Goal updated." });
      setEditingId(null);
      await refreshGoals();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to update goal."
      });
    }
  }

  async function handleDelete(goalId: string) {
    if (deleteMutation.isPending) return;
    if (!window.confirm("Delete this goal and its descendants?")) return;
    setPendingDeleteId(goalId);
    try {
      await deleteMutation.mutateAsync(goalId);
      setFeedback({ type: "ok", text: "Goal deleted." });
      if (editingId === goalId) {
        setEditingId(null);
      }
      await refreshGoals();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to delete goal."
      });
    } finally {
      setPendingDeleteId(null);
    }
  }

  const inputClass = "rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand focus:ring-2 focus:ring-brand/20 transition-all outline-none disabled:opacity-50";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
            </svg>
            Strategic Planning
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Goals</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Hierarchical mission tracking with multi-agent ownership and progress monitoring.</p>
        </div>
        <button
          className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm disabled:opacity-50"
          onClick={() => void refreshGoals()}
          disabled={goalsQuery.isFetching}
        >
          <svg className={`h-3.5 w-3.5 ${goalsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
            <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
          Refresh
        </button>
      </header>

      {feedback ? (
        <div
          className={`animate-in fade-in slide-in-from-top-2 rounded-xl border p-4 text-sm font-bold shadow-sm ${
            feedback.type === "ok"
              ? "border-success/20 bg-success/5 text-success"
              : "border-error/20 bg-error/5 text-error"
          }`}
        >
          <div className="flex items-center gap-3">
            <div className={`h-2 w-2 rounded-full ${feedback.type === 'ok' ? 'bg-success' : 'bg-error'}`} />
            {feedback.text}
          </div>
        </div>
      ) : null}
      
      {error ? (
        <div className="rounded-xl border border-error/20 bg-error/5 p-4 text-sm text-error font-bold">{error}</div>
      ) : null}

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: "Total Goals", value: stats.total, color: "brand" },
          { label: "Pending", value: stats.pending, color: "text-dim" },
          { label: "In Progress", value: stats.inProgress, color: "warning" },
          { label: "Completed", value: stats.completed, color: "success" },
        ].map((stat, i) => (
          <article key={i} className="rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
            <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{stat.label}</span>
            <div className="mt-1 flex items-baseline gap-2">
              <strong className={`text-3xl font-black tracking-tight text-${stat.color}`}>{stat.value}</strong>
              <div className={`h-1 w-1 rounded-full bg-${stat.color}`} />
            </div>
          </article>
        ))}
      </div>

      <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
        <aside className="flex flex-col gap-6">
          <section className="h-fit rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
            <h2 className="text-lg font-black tracking-tight">Create Goal</h2>
            <p className="mb-6 text-xs text-text-dim font-medium">Define a new objective for your agents.</p>
            
            <form className="flex flex-col gap-4" onSubmit={handleCreateGoal}>
              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Goal Title</label>
                <input
                  value={createDraft.title}
                  onChange={(event) => setCreateDraft((current) => ({ ...current, title: event.target.value }))}
                  placeholder="e.g. Master the Rust language"
                  className={inputClass}
                />
              </div>
              
              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Description</label>
                <textarea
                  value={createDraft.description}
                  onChange={(event) => setCreateDraft((current) => ({ ...current, description: event.target.value }))}
                  rows={3}
                  placeholder="Detailed breakdown..."
                  className={`${inputClass} resize-none`}
                />
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div className="flex flex-col gap-1.5">
                  <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Status</label>
                  <select
                    value={createDraft.status}
                    onChange={(event) => setCreateDraft((current) => ({ ...current, status: event.target.value }))}
                    className={inputClass}
                  >
                    <option value="pending">Pending</option>
                    <option value="in_progress">In Progress</option>
                    <option value="completed">Completed</option>
                    <option value="cancelled">Cancelled</option>
                  </select>
                </div>
                <div className="flex flex-col gap-1.5">
                  <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Progress %</label>
                  <input
                    type="number"
                    min={0}
                    max={100}
                    value={createDraft.progress}
                    onChange={(event) => setCreateDraft((current) => ({ ...current, progress: clampProgress(Number(event.target.value)) }))}
                    className={inputClass}
                  />
                </div>
              </div>

              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Parent Goal</label>
                <select
                  value={createDraft.parent_id}
                  onChange={(event) => setCreateDraft((current) => ({ ...current, parent_id: event.target.value }))}
                  className={inputClass}
                >
                  <option value="">No parent (Top level)</option>
                  {goals.map((goal) => (
                    <option key={goal.id} value={goal.id}>
                      {goal.title ?? goal.id}
                    </option>
                  ))}
                </select>
              </div>

              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Responsible Agent</label>
                <select
                  value={createDraft.agent_id}
                  onChange={(event) => setCreateDraft((current) => ({ ...current, agent_id: event.target.value }))}
                  className={inputClass}
                >
                  <option value="">No agent assigned</option>
                  {agents.map((agent) => (
                    <option key={agent.id} value={agent.id}>
                      {agent.name}
                    </option>
                  ))}
                </select>
              </div>

              <button
                type="submit"
                className="mt-2 rounded-xl bg-brand py-3 text-sm font-bold text-white shadow-lg shadow-brand/20 hover:opacity-90 transition-all disabled:opacity-50"
                disabled={createDraft.title.trim().length === 0 || createMutation.isPending}
              >
                Create Goal
              </button>
            </form>
          </section>

          {editingId ? (
            <section className="h-fit rounded-2xl border border-brand/30 bg-brand-muted p-6 shadow-sm animate-in slide-in-from-bottom-4">
              <h3 className="text-sm font-black uppercase tracking-widest text-brand mb-4">Edit Goal</h3>
              <form className="flex flex-col gap-3" onSubmit={handleSaveEdit}>
                <input
                  value={editDraft.title}
                  onChange={(event) => setEditDraft((current) => ({ ...current, title: event.target.value }))}
                  className={inputClass}
                />
                <textarea
                  value={editDraft.description}
                  onChange={(event) => setEditDraft((current) => ({ ...current, description: event.target.value }))}
                  rows={2}
                  className={`${inputClass} resize-none`}
                />
                <div className="grid grid-cols-2 gap-2">
                  <select
                    value={editDraft.status}
                    onChange={(event) => setEditDraft((current) => ({ ...current, status: event.target.value }))}
                    className={inputClass}
                  >
                    <option value="pending">Pending</option>
                    <option value="in_progress">In Progress</option>
                    <option value="completed">Completed</option>
                    <option value="cancelled">Cancelled</option>
                  </select>
                  <input
                    type="number"
                    value={editDraft.progress}
                    onChange={(event) => setEditDraft((current) => ({ ...current, progress: clampProgress(Number(event.target.value)) }))}
                    className={inputClass}
                  />
                </div>
                <div className="flex gap-2 mt-2">
                  <button
                    type="submit"
                    className="flex-1 rounded-xl bg-brand py-2 text-xs font-bold text-white shadow-md shadow-brand/20 hover:opacity-90"
                    disabled={updateMutation.isPending}
                  >
                    Save Changes
                  </button>
                  <button
                    type="button"
                    className="flex-1 rounded-xl border border-border-subtle bg-surface py-2 text-xs font-bold text-text-dim hover:text-slate-900 dark:hover:text-white"
                    onClick={() => setEditingId(null)}
                  >
                    Cancel
                  </button>
                </div>
              </form>
            </section>
          ) : null}
        </aside>

        <article className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5 overflow-hidden">
          <h2 className="text-lg font-black tracking-tight mb-1">Goal Tree</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">Visualized hierarchy of objectives and sub-tasks.</p>
          
          {goalsQuery.isLoading && goals.length === 0 ? (
            <div className="py-24 text-center">
              <div className="mx-auto h-10 w-10 animate-spin rounded-full border-2 border-brand border-t-transparent mb-4" />
              <p className="text-sm text-text-dim font-bold">Synchronizing goals...</p>
            </div>
          ) : rows.length === 0 ? (
            <div className="py-24 text-center border border-dashed border-border-subtle rounded-2xl">
              <p className="text-sm text-text-dim font-bold">No strategic goals defined yet.</p>
            </div>
          ) : (
            <div className="overflow-y-auto max-h-[800px] pr-2 scrollbar-thin">
              <ul className="flex flex-col gap-2 list-none p-0 m-0">
                {rows.map((row) => (
                  <li 
                    key={row.goal.id} 
                    className="group rounded-xl border border-border-subtle bg-main/40 p-4 transition-all hover:border-brand/30"
                    style={{ marginLeft: `${row.depth * 20}px` }}
                  >
                    <div className="flex flex-wrap items-center justify-between gap-4">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-3">
                          {row.hasChildren ? (
                            <button
                              type="button"
                              className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded-lg border border-border-subtle bg-surface text-xs font-black text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm"
                              onClick={() => toggleExpand(row.goal.id)}
                            >
                              {expandedById[row.goal.id] ? "−" : "+"}
                            </button>
                          ) : (
                            <div className="h-1.5 w-1.5 rounded-full bg-brand/20 ml-2" />
                          )}
                          <strong className="truncate text-sm font-black">{row.goal.title ?? row.goal.id}</strong>
                          <span className={`rounded-lg border px-2 py-0.5 text-[10px] font-black uppercase tracking-widest ${statusClass(row.goal.status)}`}>
                            {statusLabel(row.goal.status)}
                          </span>
                        </div>
                        
                        <div className="mt-2 flex flex-col gap-1">
                          <p className="m-0 break-words text-[11px] font-medium text-text-dim/80 line-clamp-2 leading-relaxed italic">
                            {row.goal.description || "No description provided."}
                          </p>
                          <div className="mt-1 flex items-center gap-4">
                            <div className="flex items-center gap-2">
                              <div className="h-1.5 w-24 rounded-full bg-slate-200 dark:bg-slate-800 overflow-hidden">
                                <div className="h-full bg-brand transition-all duration-500" style={{ width: `${clampProgress(row.goal.progress ?? 0)}%` }} />
                              </div>
                              <span className="text-[10px] font-black text-brand tracking-tighter">{clampProgress(row.goal.progress ?? 0)}%</span>
                            </div>
                            <div className="h-1 w-1 rounded-full bg-border-subtle" />
                            <p className="text-[10px] font-bold text-text-dim uppercase tracking-wider">
                              OWNER: <span className="text-slate-700 dark:text-slate-300 font-black">{row.goal.agent_id ? agentNameById.get(row.goal.agent_id) ?? row.goal.agent_id : "UNASSIGNED"}</span>
                            </p>
                          </div>
                        </div>
                      </div>
                      
                      <div className="flex gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                        <button
                          className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-black text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm"
                          onClick={() => startEdit(row.goal)}
                        >
                          Edit
                        </button>
                        <button
                          className="rounded-lg border border-error/20 bg-error/10 px-3 py-1.5 text-[10px] font-black text-error hover:bg-error/20 transition-all shadow-sm"
                          onClick={() => void handleDelete(row.goal.id)}
                          disabled={pendingDeleteId === row.goal.id}
                        >
                          {pendingDeleteId === row.goal.id ? "..." : "Delete"}
                        </button>
                      </div>
                    </div>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </article>
      </div>
    </div>
  );
}
