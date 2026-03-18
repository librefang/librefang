import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import {
  activateHand,
  deactivateHand,
  getHandStats,
  listActiveHands,
  listHands,
  pauseHand,
  resumeHand,
  type ApiActionResponse,
  type HandStatsResponse
} from "../api";

const REFRESH_MS = 15000;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

function actionText(action: ApiActionResponse): string {
  if (typeof action.message === "string" && action.message.trim().length > 0) return action.message;
  if (typeof action.status === "string" && action.status.trim().length > 0) return action.status;
  if (typeof action.error === "string" && action.error.trim().length > 0) return action.error;
  return JSON.stringify(action);
}

function dateText(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function statusClass(status?: string): string {
  const value = (status ?? "").toLowerCase();
  if (value.includes("active")) return "border-emerald-700 bg-emerald-700/15 text-emerald-100";
  if (value.includes("paused")) return "border-amber-700 bg-amber-700/15 text-amber-100";
  if (value.includes("error")) return "border-rose-700 bg-rose-700/15 text-rose-100";
  return "border-slate-700 bg-slate-800/60 text-slate-100";
}

export function HandsPage() {
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [pendingActivateId, setPendingActivateId] = useState<string | null>(null);
  const [pendingInstanceId, setPendingInstanceId] = useState<string | null>(null);
  const [statsByInstance, setStatsByInstance] = useState<Record<string, HandStatsResponse>>({});

  const handsQuery = useQuery({
    queryKey: ["hands", "list"],
    queryFn: listHands,
    refetchInterval: REFRESH_MS
  });
  const activeQuery = useQuery({
    queryKey: ["hands", "active"],
    queryFn: listActiveHands,
    refetchInterval: REFRESH_MS
  });

  const activateMutation = useMutation({
    mutationFn: (handId: string) => activateHand(handId)
  });
  const pauseMutation = useMutation({
    mutationFn: (instanceId: string) => pauseHand(instanceId)
  });
  const resumeMutation = useMutation({
    mutationFn: (instanceId: string) => resumeHand(instanceId)
  });
  const deactivateMutation = useMutation({
    mutationFn: (instanceId: string) => deactivateHand(instanceId)
  });

  const hands = handsQuery.data ?? [];
  const instances = activeQuery.data ?? [];
  const activeHandIds = useMemo(
    () => new Set(instances.map((instance) => instance.hand_id).filter(Boolean) as string[]),
    [instances]
  );

  const error = (() => {
    if (handsQuery.error instanceof Error) return handsQuery.error.message;
    if (activeQuery.error instanceof Error) return activeQuery.error.message;
    return "";
  })();

  async function refreshAll() {
    await queryClient.invalidateQueries({ queryKey: ["hands"] });
    await Promise.all([handsQuery.refetch(), activeQuery.refetch()]);
  }

  async function handleActivate(handId: string) {
    if (activateMutation.isPending) return;
    setPendingActivateId(handId);
    try {
      const result = await activateMutation.mutateAsync(handId);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to activate hand."
      });
    } finally {
      setPendingActivateId(null);
    }
  }

  async function handlePause(instanceId: string) {
    if (pauseMutation.isPending) return;
    setPendingInstanceId(instanceId);
    try {
      const result = await pauseMutation.mutateAsync(instanceId);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to pause hand."
      });
    } finally {
      setPendingInstanceId(null);
    }
  }

  async function handleResume(instanceId: string) {
    if (resumeMutation.isPending) return;
    setPendingInstanceId(instanceId);
    try {
      const result = await resumeMutation.mutateAsync(instanceId);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to resume hand."
      });
    } finally {
      setPendingInstanceId(null);
    }
  }

  async function handleDeactivate(instanceId: string) {
    if (deactivateMutation.isPending) return;
    if (!window.confirm("Deactivate this hand instance?")) return;
    setPendingInstanceId(instanceId);
    try {
      const result = await deactivateMutation.mutateAsync(instanceId);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to deactivate hand."
      });
    } finally {
      setPendingInstanceId(null);
    }
  }

  async function handleLoadStats(instanceId: string) {
    try {
      const stats = await getHandStats(instanceId);
      setStatsByInstance((current) => ({ ...current, [instanceId]: stats }));
    } catch (statsError) {
      setFeedback({
        type: "error",
        text: statsError instanceof Error ? statsError.message : "Failed to load hand stats."
      });
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Hands</h1>
          <p className="text-sm text-slate-400">Autonomous capability packages with lifecycle and runtime metrics.</p>
        </div>
        <button
          className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
          onClick={() => void refreshAll()}
          disabled={handsQuery.isFetching || activeQuery.isFetching}
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
          <span className="text-sm text-slate-400">Available</span>
          <strong className="mt-1 block text-2xl">{hands.length}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Active Instances</span>
          <strong className="mt-1 block text-2xl">{instances.length}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Ready Hands</span>
          <strong className="mt-1 block text-2xl">
            {hands.filter((hand) => hand.requirements_met).length}
          </strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Running Hands</span>
          <strong className="mt-1 block text-2xl">{activeHandIds.size}</strong>
        </article>
      </div>

      <div className="grid gap-3 xl:grid-cols-2">
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Available Hands</h2>
          {handsQuery.isLoading ? (
            <p className="mt-2 text-sm text-slate-400">Loading hands...</p>
          ) : hands.length === 0 ? (
            <p className="mt-2 text-sm text-slate-400">No hands found.</p>
          ) : (
            <ul className="mt-3 flex max-h-[520px] list-none flex-col gap-2 overflow-y-auto p-0">
              {hands.map((hand) => {
                const requirementTotal = hand.requirements?.length ?? 0;
                const requirementOk = hand.requirements?.filter((item) => item.satisfied).length ?? 0;
                return (
                  <li key={hand.id} className="rounded-lg border border-slate-800 bg-slate-950/70 p-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="m-0 truncate text-sm font-semibold">{hand.name ?? hand.id}</p>
                        <p className="m-0 mt-1 text-xs text-slate-400">{hand.description ?? "-"}</p>
                      </div>
                      <span
                        className={`rounded-full border px-2 py-1 text-xs ${
                          hand.requirements_met
                            ? "border-emerald-700 bg-emerald-700/15 text-emerald-100"
                            : "border-amber-700 bg-amber-700/15 text-amber-100"
                        }`}
                      >
                        {hand.requirements_met ? "Ready" : "Setup"}
                      </span>
                    </div>
                    <p className="m-0 mt-2 text-xs text-slate-500">
                      req {requirementOk}/{requirementTotal} · tools {(hand.tools ?? []).length}
                    </p>
                    <button
                      className="mt-2 rounded-lg border border-sky-500 bg-sky-600 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
                      onClick={() => void handleActivate(hand.id)}
                      disabled={pendingActivateId === hand.id}
                    >
                      Activate
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </article>

        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Active Instances</h2>
          {activeQuery.isLoading ? (
            <p className="mt-2 text-sm text-slate-400">Loading active instances...</p>
          ) : instances.length === 0 ? (
            <p className="mt-2 text-sm text-slate-400">No active hand instances.</p>
          ) : (
            <ul className="mt-3 flex max-h-[520px] list-none flex-col gap-2 overflow-y-auto p-0">
              {instances.map((instance) => (
                <li key={instance.instance_id} className="rounded-lg border border-slate-800 bg-slate-950/70 p-3">
                  <div className="flex items-center justify-between gap-2">
                    <p className="m-0 text-sm font-semibold">{instance.hand_id ?? "-"}</p>
                    <span className={`rounded-full border px-2 py-1 text-xs ${statusClass(instance.status)}`}>
                      {instance.status ?? "unknown"}
                    </span>
                  </div>
                  <p className="m-0 mt-1 text-xs text-slate-500">
                    {instance.agent_name ?? instance.agent_id ?? "-"} · {dateText(instance.activated_at)}
                  </p>
                  <div className="mt-2 flex flex-wrap gap-2">
                    <button
                      className="rounded-lg border border-amber-700 bg-amber-700/10 px-2 py-1 text-xs text-amber-200 transition hover:bg-amber-700/20 disabled:cursor-not-allowed disabled:opacity-60"
                      onClick={() => void handlePause(instance.instance_id)}
                      disabled={pendingInstanceId === instance.instance_id}
                    >
                      Pause
                    </button>
                    <button
                      className="rounded-lg border border-emerald-700 bg-emerald-700/10 px-2 py-1 text-xs text-emerald-200 transition hover:bg-emerald-700/20 disabled:cursor-not-allowed disabled:opacity-60"
                      onClick={() => void handleResume(instance.instance_id)}
                      disabled={pendingInstanceId === instance.instance_id}
                    >
                      Resume
                    </button>
                    <button
                      className="rounded-lg border border-rose-700 bg-rose-700/10 px-2 py-1 text-xs text-rose-200 transition hover:bg-rose-700/20 disabled:cursor-not-allowed disabled:opacity-60"
                      onClick={() => void handleDeactivate(instance.instance_id)}
                      disabled={pendingInstanceId === instance.instance_id}
                    >
                      Deactivate
                    </button>
                    <button
                      className="rounded-lg border border-slate-600 bg-slate-800 px-2 py-1 text-xs text-slate-200 transition hover:border-slate-400 hover:bg-slate-700"
                      onClick={() => void handleLoadStats(instance.instance_id)}
                    >
                      Load Stats
                    </button>
                  </div>
                  {statsByInstance[instance.instance_id]?.metrics ? (
                    <ul className="mt-2 flex list-none flex-col gap-1 p-0 text-xs text-slate-300">
                      {Object.entries(statsByInstance[instance.instance_id].metrics ?? {}).map(([key, value]) => (
                        <li key={key} className="rounded border border-slate-800 bg-slate-900/70 px-2 py-1">
                          <span className="text-slate-400">{key}:</span>{" "}
                          <span>{typeof value?.value === "object" ? JSON.stringify(value?.value) : String(value?.value ?? "-")}</span>
                        </li>
                      ))}
                    </ul>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </article>
      </div>
    </section>
  );
}
