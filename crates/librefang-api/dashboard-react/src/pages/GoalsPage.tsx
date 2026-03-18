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
  if (value === "completed") return "border-emerald-700 bg-emerald-700/15 text-emerald-100";
  if (value === "in_progress") return "border-amber-700 bg-amber-700/15 text-amber-100";
  if (value === "cancelled") return "border-slate-700 bg-slate-800/60 text-slate-200";
  return "border-sky-700 bg-sky-700/15 text-sky-100";
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

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Goals</h1>
          <p className="text-sm text-slate-400">Hierarchical goals with status tracking, ownership, and progress.</p>
        </div>
        <button
          className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
          onClick={() => void refreshGoals()}
          disabled={goalsQuery.isFetching}
        >
          Refresh
        </button>
      </header>

      {feedback ? (
        <div
          className={`rounded-xl border p-3 text-sm ${
            feedback.type === "ok"
              ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
              : "border-rose-700 bg-rose-700/10 text-rose-200"
          }`}
        >
          {feedback.text}
        </div>
      ) : null}
      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Total</span>
          <strong className="mt-1 block text-2xl">{stats.total}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Pending</span>
          <strong className="mt-1 block text-2xl">{stats.pending}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">In Progress</span>
          <strong className="mt-1 block text-2xl">{stats.inProgress}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Completed</span>
          <strong className="mt-1 block text-2xl">{stats.completed}</strong>
        </article>
      </div>

      <div className="grid gap-3 xl:grid-cols-[360px_1fr]">
        <aside className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Create Goal</h2>
          <form className="mt-3 flex flex-col gap-2" onSubmit={handleCreateGoal}>
            <input
              value={createDraft.title}
              onChange={(event) =>
                setCreateDraft((current) => ({ ...current, title: event.target.value }))
              }
              placeholder="Title"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            />
            <textarea
              value={createDraft.description}
              onChange={(event) =>
                setCreateDraft((current) => ({ ...current, description: event.target.value }))
              }
              rows={3}
              placeholder="Description"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            />
            <select
              value={createDraft.status}
              onChange={(event) =>
                setCreateDraft((current) => ({ ...current, status: event.target.value }))
              }
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            >
              <option value="pending">Pending</option>
              <option value="in_progress">In Progress</option>
              <option value="completed">Completed</option>
              <option value="cancelled">Cancelled</option>
            </select>
            <input
              type="number"
              min={0}
              max={100}
              value={createDraft.progress}
              onChange={(event) =>
                setCreateDraft((current) => ({
                  ...current,
                  progress: clampProgress(Number(event.target.value))
                }))
              }
              placeholder="Progress 0-100"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            />
            <select
              value={createDraft.parent_id}
              onChange={(event) =>
                setCreateDraft((current) => ({ ...current, parent_id: event.target.value }))
              }
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            >
              <option value="">No parent</option>
              {goals.map((goal) => (
                <option key={goal.id} value={goal.id}>
                  {goal.title ?? goal.id}
                </option>
              ))}
            </select>
            <select
              value={createDraft.agent_id}
              onChange={(event) =>
                setCreateDraft((current) => ({ ...current, agent_id: event.target.value }))
              }
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            >
              <option value="">No agent</option>
              {agents.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}
                </option>
              ))}
            </select>
            <button
              type="submit"
              className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
              disabled={createDraft.title.trim().length === 0 || createMutation.isPending}
            >
              Create Goal
            </button>
          </form>

          {editingId ? (
            <div className="mt-4 rounded-lg border border-slate-800 bg-slate-950/70 p-3">
              <h3 className="m-0 text-sm font-semibold">Edit Goal</h3>
              <form className="mt-2 flex flex-col gap-2" onSubmit={handleSaveEdit}>
                <input
                  value={editDraft.title}
                  onChange={(event) =>
                    setEditDraft((current) => ({ ...current, title: event.target.value }))
                  }
                  className="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                />
                <textarea
                  value={editDraft.description}
                  onChange={(event) =>
                    setEditDraft((current) => ({ ...current, description: event.target.value }))
                  }
                  rows={3}
                  className="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                />
                <select
                  value={editDraft.status}
                  onChange={(event) =>
                    setEditDraft((current) => ({ ...current, status: event.target.value }))
                  }
                  className="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                >
                  <option value="pending">Pending</option>
                  <option value="in_progress">In Progress</option>
                  <option value="completed">Completed</option>
                  <option value="cancelled">Cancelled</option>
                </select>
                <input
                  type="number"
                  min={0}
                  max={100}
                  value={editDraft.progress}
                  onChange={(event) =>
                    setEditDraft((current) => ({
                      ...current,
                      progress: clampProgress(Number(event.target.value))
                    }))
                  }
                  className="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                />
                <div className="flex gap-2">
                  <button
                    type="submit"
                    className="flex-1 rounded-lg border border-emerald-700 bg-emerald-700/15 px-3 py-2 text-sm font-medium text-emerald-200 transition hover:bg-emerald-700/25 disabled:cursor-not-allowed disabled:opacity-60"
                    disabled={updateMutation.isPending}
                  >
                    Save
                  </button>
                  <button
                    type="button"
                    className="flex-1 rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-200 transition hover:border-slate-400 hover:bg-slate-700"
                    onClick={() => setEditingId(null)}
                  >
                    Cancel
                  </button>
                </div>
              </form>
            </div>
          ) : null}
        </aside>

        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Goal Tree</h2>
          {goalsQuery.isLoading ? (
            <p className="mt-2 text-sm text-slate-400">Loading goals...</p>
          ) : rows.length === 0 ? (
            <p className="mt-2 text-sm text-slate-400">No goals yet.</p>
          ) : (
            <ul className="mt-3 flex max-h-[640px] list-none flex-col gap-2 overflow-y-auto p-0">
              {rows.map((row) => (
                <li key={row.goal.id} className="rounded-lg border border-slate-800 bg-slate-950/70 p-3">
                  <div
                    className="flex flex-wrap items-center justify-between gap-2"
                    style={{ paddingLeft: `${row.depth * 16}px` }}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        {row.hasChildren ? (
                          <button
                            type="button"
                            className="h-5 w-5 rounded border border-slate-700 bg-slate-900 text-xs text-slate-300"
                            onClick={() => toggleExpand(row.goal.id)}
                          >
                            {expandedById[row.goal.id] ? "-" : "+"}
                          </button>
                        ) : (
                          <span className="inline-block h-5 w-5" />
                        )}
                        <strong className="truncate text-sm">{row.goal.title ?? row.goal.id}</strong>
                        <span className={`rounded-full border px-2 py-1 text-xs ${statusClass(row.goal.status)}`}>
                          {statusLabel(row.goal.status)}
                        </span>
                      </div>
                      <p className="m-0 mt-1 break-words text-xs text-slate-400">{row.goal.description ?? "-"}</p>
                      <p className="m-0 mt-1 text-xs text-slate-500">
                        progress {clampProgress(row.goal.progress ?? 0)}% ·{" "}
                        {row.goal.agent_id ? agentNameById.get(row.goal.agent_id) ?? row.goal.agent_id : "no agent"}
                      </p>
                    </div>
                    <div className="flex gap-2">
                      <button
                        className="rounded-lg border border-slate-600 bg-slate-800 px-2 py-1 text-xs text-slate-100 transition hover:border-slate-400 hover:bg-slate-700"
                        onClick={() => startEdit(row.goal)}
                      >
                        Edit
                      </button>
                      <button
                        className="rounded-lg border border-rose-700 bg-rose-700/10 px-2 py-1 text-xs text-rose-200 transition hover:bg-rose-700/20 disabled:cursor-not-allowed disabled:opacity-60"
                        onClick={() => void handleDelete(row.goal.id)}
                        disabled={pendingDeleteId === row.goal.id}
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </article>
      </div>
    </section>
  );
}
