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
  if (value.includes("active")) return "border-success/20 bg-success/10 text-success";
  if (value.includes("paused")) return "border-warning/20 bg-warning/10 text-warning";
  if (value.includes("error")) return "border-error/20 bg-error/10 text-error";
  return "border-border-subtle bg-main text-text-dim";
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
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 11V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" /><path d="M14 10V4a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" /><path d="M10 10.5V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" />
            </svg>
            Capability Orchestration
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Hands</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Autonomous capability packages with lifecycle and runtime metrics.</p>
        </div>
        <button
          className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm disabled:opacity-50"
          onClick={() => void refreshAll()}
          disabled={handsQuery.isFetching || activeQuery.isFetching}
        >
          <svg className={`h-3.5 w-3.5 ${handsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
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
        <div className="rounded-xl border border-error/20 bg-error/5 p-4 text-error font-bold">{error}</div>
      ) : null}

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {[
          { label: "Available", value: hands.length, icon: <path d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-14v10m0 0l-8-4m8 4l8-4" /> },
          { label: "Active Instances", value: instances.length, icon: <path d="M13 10V3L4 14h7v7l9-11h-7z" /> },
          { label: "Ready Hands", value: hands.filter((hand) => hand.requirements_met).length, icon: <path d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" /> },
          { label: "Running Running", value: activeHandIds.size, icon: <circle cx="12" cy="12" r="10" /> },
        ].map((stat, i) => (
          <article key={i} className="rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm ring-1 ring-black/5 dark:ring-white/5 transition-all hover:border-brand/30">
            <div className="flex items-center justify-between">
              <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">{stat.label}</span>
              <svg className="h-4 w-4 text-brand/50" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">{stat.icon}</svg>
            </div>
            <strong className="mt-2 block text-3xl font-black tracking-tight">{stat.value}</strong>
          </article>
        ))}
      </div>

      <div className="grid gap-6 xl:grid-cols-2">
        <article className="flex flex-col rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <div className="mb-6">
            <h2 className="text-lg font-black tracking-tight">Available Hands</h2>
            <p className="text-xs font-medium text-text-dim">Ready to be deployed to an agent.</p>
          </div>
          
          {handsQuery.isLoading ? (
            <div className="py-12 text-center">
              <div className="mx-auto h-8 w-8 animate-spin rounded-full border-2 border-brand border-t-transparent mb-4" />
              <p className="text-sm text-text-dim font-medium">Loading hands...</p>
            </div>
          ) : hands.length === 0 ? (
            <div className="py-12 text-center border border-dashed border-border-subtle rounded-2xl bg-main/30">
              <p className="text-sm text-text-dim font-medium">No hands discovered.</p>
            </div>
          ) : (
            <ul className="flex max-h-[520px] list-none flex-col gap-3 overflow-y-auto pr-1 scrollbar-thin">
              {hands.map((hand) => {
                const requirementTotal = hand.requirements?.length ?? 0;
                const requirementOk = hand.requirements?.filter((item) => item.satisfied).length ?? 0;
                return (
                  <li key={hand.id} className="rounded-xl border border-border-subtle bg-main/40 p-4 transition-all hover:border-brand/30 group">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0 flex-1">
                        <p className="m-0 truncate text-sm font-black group-hover:text-brand transition-colors">{hand.name ?? hand.id}</p>
                        <p className="m-0 mt-1 text-xs font-medium text-text-dim italic leading-relaxed">{hand.description ?? "-"}</p>
                      </div>
                      <span
                        className={`shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-black uppercase tracking-widest ${
                          hand.requirements_met
                            ? "border-success/20 bg-success/10 text-success"
                            : "border-warning/20 bg-warning/10 text-warning"
                        }`}
                      >
                        {hand.requirements_met ? "Ready" : "Setup"}
                      </span>
                    </div>
                    <div className="mt-4 flex items-center justify-between border-t border-border-subtle/30 pt-4">
                      <p className="m-0 text-[10px] font-bold text-text-dim uppercase tracking-wider">
                        Req {requirementOk}/{requirementTotal} • Tools {(hand.tools ?? []).length}
                      </p>
                      <button
                        className="rounded-lg bg-brand px-4 py-1.5 text-xs font-bold text-white shadow-lg shadow-brand/20 hover:opacity-90 transition-all disabled:opacity-50"
                        onClick={() => void handleActivate(hand.id)}
                        disabled={pendingActivateId === hand.id}
                      >
                        Activate
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </article>

        <article className="flex flex-col rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <div className="mb-6">
            <h2 className="text-lg font-black tracking-tight">Active Instances</h2>
            <p className="text-xs font-medium text-text-dim">Real-time status of running capabilities.</p>
          </div>

          {activeQuery.isLoading ? (
            <div className="py-12 text-center">
              <div className="mx-auto h-8 w-8 animate-spin rounded-full border-2 border-brand border-t-transparent mb-4" />
              <p className="text-sm text-text-dim font-medium">Monitoring instances...</p>
            </div>
          ) : instances.length === 0 ? (
            <div className="py-12 text-center border border-dashed border-border-subtle rounded-2xl bg-main/30">
              <p className="text-sm text-text-dim font-medium">No instances currently running.</p>
            </div>
          ) : (
            <ul className="flex max-h-[520px] list-none flex-col gap-3 overflow-y-auto pr-1 scrollbar-thin">
              {instances.map((instance) => (
                <li key={instance.instance_id} className="rounded-xl border border-border-subtle bg-main/40 p-4 transition-all hover:border-brand/30 group">
                  <div className="flex items-center justify-between gap-4">
                    <div className="min-w-0">
                      <p className="m-0 text-sm font-black truncate">{instance.hand_id ?? "-"}</p>
                      <p className="m-0 mt-1 text-[10px] font-bold text-text-dim uppercase tracking-wider">
                        {instance.agent_name || instance.agent_id || "System"} • {dateText(instance.activated_at).split(',')[1]}
                      </p>
                    </div>
                    <span className={`shrink-0 rounded-lg border px-2 py-0.5 text-[10px] font-black uppercase tracking-widest ${statusClass(instance.status)}`}>
                      {instance.status ?? "unknown"}
                    </span>
                  </div>
                  
                  <div className="mt-4 flex flex-wrap gap-2 pt-4 border-t border-border-subtle/30">
                    <button
                      className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold text-text-dim hover:text-warning hover:border-warning/30 transition-all shadow-sm disabled:opacity-50"
                      onClick={() => void handlePause(instance.instance_id)}
                      disabled={pendingInstanceId === instance.instance_id}
                    >
                      {instance.status === 'paused' ? 'Paused' : 'Pause'}
                    </button>
                    <button
                      className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold text-text-dim hover:text-success hover:border-success/30 transition-all shadow-sm disabled:opacity-50"
                      onClick={() => void handleResume(instance.instance_id)}
                      disabled={pendingInstanceId === instance.instance_id}
                    >
                      Resume
                    </button>
                    <button
                      className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold text-text-dim hover:text-error hover:border-error/30 transition-all shadow-sm disabled:opacity-50"
                      onClick={() => void handleDeactivate(instance.instance_id)}
                      disabled={pendingInstanceId === instance.instance_id}
                    >
                      Stop
                    </button>
                    <button
                      className="ml-auto rounded-lg border border-brand/20 bg-brand/5 px-3 py-1.5 text-[10px] font-bold text-brand hover:bg-brand/10 transition-all shadow-sm"
                      onClick={() => void handleLoadStats(instance.instance_id)}
                    >
                      Telemetry
                    </button>
                  </div>

                  {statsByInstance[instance.instance_id]?.metrics ? (
                    <div className="mt-3 p-3 rounded-lg bg-surface/50 border border-border-subtle/50 animate-in slide-in-from-top-2">
                      <div className="grid grid-cols-2 gap-x-4 gap-y-2">
                        {Object.entries(statsByInstance[instance.instance_id].metrics ?? {}).map(([key, value]) => (
                          <div key={key} className="flex flex-col">
                            <span className="text-[9px] font-black text-text-dim uppercase tracking-widest">{key}</span>
                            <span className="text-xs font-bold truncate">
                              {typeof value?.value === "object" ? JSON.stringify(value?.value) : String(value?.value ?? "0")}
                            </span>
                          </div>
                        ))}
                      </div>
                    </div>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </article>
      </div>
    </div>
  );
}
