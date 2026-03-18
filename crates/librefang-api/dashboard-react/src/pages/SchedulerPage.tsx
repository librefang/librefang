import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import {
  createSchedule,
  deleteSchedule,
  deleteTrigger,
  listAgents,
  listCronJobs,
  listSchedules,
  listTriggers,
  runSchedule,
  updateSchedule,
  updateTrigger,
  type ApiActionResponse,
  type CronJobItem,
  type ScheduleItem,
  type TriggerItem
} from "../api";

const REFRESH_MS = 30000;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

function actionText(action: ApiActionResponse): string {
  if (typeof action.message === "string" && action.message.trim().length > 0) return action.message;
  if (typeof action.status === "string" && action.status.trim().length > 0) return action.status;
  return JSON.stringify(action);
}

function dateText(value?: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function scheduleAgent(schedule: ScheduleItem): string {
  return schedule.agent_id ?? schedule.agent ?? "-";
}

function scheduleSort(a: ScheduleItem, b: ScheduleItem): number {
  return (b.created_at ?? "").localeCompare(a.created_at ?? "");
}

export function SchedulerPage() {
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [name, setName] = useState("");
  const [cron, setCron] = useState("0 9 * * *");
  const [agentId, setAgentId] = useState("");
  const [message, setMessage] = useState("");

  const [pendingScheduleToggleId, setPendingScheduleToggleId] = useState<string | null>(null);
  const [pendingScheduleDeleteId, setPendingScheduleDeleteId] = useState<string | null>(null);
  const [pendingScheduleRunId, setPendingScheduleRunId] = useState<string | null>(null);
  const [pendingTriggerToggleId, setPendingTriggerToggleId] = useState<string | null>(null);
  const [pendingTriggerDeleteId, setPendingTriggerDeleteId] = useState<string | null>(null);

  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "scheduler-helper"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });
  const schedulesQuery = useQuery({
    queryKey: ["schedules", "list"],
    queryFn: listSchedules,
    refetchInterval: REFRESH_MS
  });
  const triggersQuery = useQuery({
    queryKey: ["triggers", "list"],
    queryFn: listTriggers,
    refetchInterval: REFRESH_MS
  });
  const cronJobsQuery = useQuery({
    queryKey: ["cron-jobs", "list"],
    queryFn: listCronJobs,
    refetchInterval: REFRESH_MS
  });

  const createScheduleMutation = useMutation({
    mutationFn: createSchedule
  });
  const updateScheduleMutation = useMutation({
    mutationFn: ({
      scheduleId,
      payload
    }: {
      scheduleId: string;
      payload: { enabled?: boolean; name?: string; cron?: string; agent_id?: string; message?: string };
    }) => updateSchedule(scheduleId, payload)
  });
  const deleteScheduleMutation = useMutation({
    mutationFn: deleteSchedule
  });
  const runScheduleMutation = useMutation({
    mutationFn: runSchedule
  });

  const updateTriggerMutation = useMutation({
    mutationFn: ({ triggerId, enabled }: { triggerId: string; enabled: boolean }) =>
      updateTrigger(triggerId, { enabled })
  });
  const deleteTriggerMutation = useMutation({
    mutationFn: deleteTrigger
  });

  const schedules = useMemo(
    () => [...(schedulesQuery.data ?? [])].sort(scheduleSort),
    [schedulesQuery.data]
  );
  const triggers = triggersQuery.data ?? [];
  const cronJobs = cronJobsQuery.data ?? [];
  const agents = agentsQuery.data ?? [];

  const schedulesError = schedulesQuery.error instanceof Error ? schedulesQuery.error.message : "";
  const triggersError = triggersQuery.error instanceof Error ? triggersQuery.error.message : "";
  const cronJobsError = cronJobsQuery.error instanceof Error ? cronJobsQuery.error.message : "";

  async function refreshAll() {
    await queryClient.invalidateQueries({ queryKey: ["schedules"] });
    await queryClient.invalidateQueries({ queryKey: ["triggers"] });
    await queryClient.invalidateQueries({ queryKey: ["cron-jobs"] });
    await Promise.all([schedulesQuery.refetch(), triggersQuery.refetch(), cronJobsQuery.refetch()]);
  }

  async function handleCreateSchedule(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const inputName = name.trim();
    const inputCron = cron.trim();
    const inputAgent = agentId.trim();
    if (!inputName || !inputCron || !inputAgent || createScheduleMutation.isPending) return;

    try {
      const result = await createScheduleMutation.mutateAsync({
        name: inputName,
        cron: inputCron,
        agent_id: inputAgent,
        message: message.trim(),
        enabled: true
      });
      setFeedback({ type: "ok", text: `created: ${result.id}` });
      setName("");
      setMessage("");
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Schedule creation failed."
      });
    }
  }

  async function handleToggleSchedule(schedule: ScheduleItem) {
    if (updateScheduleMutation.isPending) return;
    const nextEnabled = !Boolean(schedule.enabled);
    setPendingScheduleToggleId(schedule.id);
    try {
      const result = await updateScheduleMutation.mutateAsync({
        scheduleId: schedule.id,
        payload: { enabled: nextEnabled }
      });
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Schedule update failed."
      });
    } finally {
      setPendingScheduleToggleId(null);
    }
  }

  async function handleRunSchedule(schedule: ScheduleItem) {
    if (runScheduleMutation.isPending) return;
    setPendingScheduleRunId(schedule.id);
    try {
      if (!schedule.agent_id && schedule.agent) {
        await updateScheduleMutation.mutateAsync({
          scheduleId: schedule.id,
          payload: { agent_id: schedule.agent }
        });
      }

      const result = await runScheduleMutation.mutateAsync(schedule.id);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Schedule run failed."
      });
    } finally {
      setPendingScheduleRunId(null);
    }
  }

  async function handleDeleteSchedule(schedule: ScheduleItem) {
    if (deleteScheduleMutation.isPending) return;
    if (!window.confirm(`Delete schedule "${schedule.name ?? schedule.id}"?`)) return;

    setPendingScheduleDeleteId(schedule.id);
    try {
      const result = await deleteScheduleMutation.mutateAsync(schedule.id);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Schedule deletion failed."
      });
    } finally {
      setPendingScheduleDeleteId(null);
    }
  }

  async function handleToggleTrigger(trigger: TriggerItem) {
    if (updateTriggerMutation.isPending) return;
    const nextEnabled = !Boolean(trigger.enabled);
    setPendingTriggerToggleId(trigger.id);
    try {
      const result = await updateTriggerMutation.mutateAsync({
        triggerId: trigger.id,
        enabled: nextEnabled
      });
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Trigger update failed."
      });
    } finally {
      setPendingTriggerToggleId(null);
    }
  }

  async function handleDeleteTrigger(trigger: TriggerItem) {
    if (deleteTriggerMutation.isPending) return;
    if (!window.confirm(`Delete trigger "${trigger.id}"?`)) return;

    setPendingTriggerDeleteId(trigger.id);
    try {
      const result = await deleteTriggerMutation.mutateAsync(trigger.id);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Trigger deletion failed."
      });
    } finally {
      setPendingTriggerDeleteId(null);
    }
  }

  const inputClass = "rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand focus:ring-2 focus:ring-brand/20 transition-all outline-none disabled:opacity-50";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M12 8V12L15 14" /><circle cx="12" cy="12" r="9" />
            </svg>
            Automation Engine
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">Scheduler</h1>
          <p className="mt-1 text-text-dim font-medium">Manage cron jobs, event triggers, and periodic tasks.</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim sm:block">
            {schedules.length} schedules • {triggers.length} triggers
          </div>
          <button
            className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm disabled:opacity-50"
            type="button"
            onClick={() => void refreshAll()}
            disabled={schedulesQuery.isFetching || triggersQuery.isFetching || cronJobsQuery.refetch === undefined}
          >
            <svg className={`h-3.5 w-3.5 ${schedulesQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
              <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            Refresh
          </button>
        </div>
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

      <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
        {/* Creation Form */}
        <aside className="h-fit rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight">Create Schedule</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">Define a new automated task sequence.</p>
          
          <form className="flex flex-col gap-4" onSubmit={handleCreateSchedule}>
            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Job Name</label>
              <input
                type="text"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="e.g. Weekly Health Check"
                className={inputClass}
                disabled={createScheduleMutation.isPending}
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Cron Expression</label>
              <input
                type="text"
                value={cron}
                onChange={(event) => setCron(event.target.value)}
                placeholder="0 9 * * *"
                className={inputClass}
                disabled={createScheduleMutation.isPending}
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Target Agent</label>
              <select
                value={agentId}
                onChange={(event) => setAgentId(event.target.value)}
                className={inputClass}
                disabled={createScheduleMutation.isPending}
              >
                <option value="">Select agent...</option>
                {agents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1">Prompt Message</label>
              <textarea
                value={message}
                onChange={(event) => setMessage(event.target.value)}
                placeholder="Message to send to agent..."
                rows={4}
                className={`${inputClass} resize-none`}
                disabled={createScheduleMutation.isPending}
              />
            </div>

            <button
              className="mt-2 rounded-xl bg-brand py-3 text-sm font-bold text-white shadow-lg shadow-brand/20 hover:opacity-90 transition-all disabled:opacity-50 disabled:shadow-none"
              type="submit"
              disabled={
                createScheduleMutation.isPending ||
                name.trim().length === 0 ||
                cron.trim().length === 0 ||
                agentId.trim().length === 0
              }
            >
              {createScheduleMutation.isPending ? "Configuring..." : "Create Schedule"}
            </button>
          </form>
        </aside>

        <div className="flex flex-col gap-6">
          {/* Schedules Section */}
          <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
            <h2 className="text-lg font-black tracking-tight mb-1">Active Schedules</h2>
            <p className="mb-6 text-xs text-text-dim font-medium">Currently configured periodic agent interactions.</p>
            
            {schedulesError ? <p className="text-sm text-error font-bold mb-4">Error: {schedulesError}</p> : null}
            
            {schedulesQuery.isLoading && schedules.length === 0 ? (
              <div className="py-12 text-center">
                <div className="mx-auto h-8 w-8 animate-spin rounded-full border-2 border-brand border-t-transparent mb-4" />
                <p className="text-sm text-text-dim">Fetching schedules...</p>
              </div>
            ) : null}

            {!schedulesQuery.isLoading && schedules.length === 0 ? (
              <div className="py-12 text-center border border-dashed border-border-subtle rounded-2xl">
                <p className="text-sm text-text-dim font-medium">No schedules found. Create one to get started.</p>
              </div>
            ) : null}

            <div className="grid gap-3">
              {schedules.map((schedule) => (
                <article key={schedule.id} className="group rounded-xl border border-border-subtle bg-main/40 p-4 transition-all hover:border-brand/30">
                  <div className="flex flex-wrap items-start justify-between gap-4">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <p className="text-sm font-black">{schedule.name ?? schedule.id}</p>
                        <span className={`h-1.5 w-1.5 rounded-full ${schedule.enabled ? 'bg-success animate-pulse' : 'bg-text-dim/30'}`} />
                      </div>
                      <div className="mt-1 flex flex-wrap gap-x-4 gap-y-1">
                        <p className="text-[10px] font-bold text-brand uppercase tracking-wider">
                          CRON: <span className="text-text-dim">{schedule.cron ?? schedule.schedule_input ?? "-"}</span>
                        </p>
                        <p className="text-[10px] font-bold text-brand uppercase tracking-wider">
                          AGENT: <span className="text-text-dim">{scheduleAgent(schedule)}</span>
                        </p>
                      </div>
                      <p className="mt-1 text-[10px] font-medium text-text-dim/60">
                        Created: {dateText(schedule.created_at)} • Last Run: {dateText(schedule.last_run)}
                      </p>
                    </div>
                    
                    <div className="flex items-center gap-2">
                      <span
                        className={`rounded-lg border px-2 py-1 text-[10px] font-black uppercase tracking-widest ${
                          schedule.enabled
                            ? "border-success/20 bg-success/10 text-success"
                            : "border-border-subtle bg-surface text-text-dim"
                        }`}
                      >
                        {schedule.enabled ? "Active" : "Paused"}
                      </span>
                    </div>
                  </div>

                  {schedule.description || schedule.message ? (
                    <div className="mt-3 rounded-lg bg-surface/50 border border-border-subtle/50 p-2.5">
                      <p className="text-[11px] font-medium text-text-dim italic leading-relaxed">
                        "{schedule.description ?? schedule.message}"
                      </p>
                    </div>
                  ) : null}

                  <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-border-subtle/30 pt-3">
                    <button
                      className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm"
                      type="button"
                      onClick={() => void handleToggleSchedule(schedule)}
                      disabled={pendingScheduleToggleId === schedule.id}
                    >
                      {pendingScheduleToggleId === schedule.id
                        ? "..."
                        : schedule.enabled
                          ? "Pause"
                          : "Resume"}
                    </button>
                    <button
                      className="rounded-lg border border-brand/20 bg-brand/10 px-3 py-1.5 text-[10px] font-bold text-brand hover:bg-brand/20 transition-all shadow-sm"
                      type="button"
                      onClick={() => void handleRunSchedule(schedule)}
                      disabled={pendingScheduleRunId === schedule.id}
                    >
                      {pendingScheduleRunId === schedule.id ? "Running..." : "Run Now"}
                    </button>
                    <button
                      className="rounded-lg border border-error/20 bg-error/10 px-3 py-1.5 text-[10px] font-bold text-error hover:bg-error/20 transition-all shadow-sm"
                      type="button"
                      onClick={() => void handleDeleteSchedule(schedule)}
                      disabled={pendingScheduleDeleteId === schedule.id}
                    >
                      {pendingScheduleDeleteId === schedule.id ? "..." : "Delete"}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          </section>

          {/* Triggers & Cron Section Combined */}
          <div className="grid gap-6 md:grid-cols-2">
            <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
              <h2 className="text-lg font-black tracking-tight mb-1">Event Triggers</h2>
              <p className="mb-6 text-[10px] text-text-dim font-bold uppercase tracking-wider">Dynamic Condition Hooks</p>
              
              <div className="grid gap-3">
                {triggers.map((trigger) => (
                  <article key={trigger.id} className="rounded-xl border border-border-subtle bg-main/40 p-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="text-xs font-black truncate">{trigger.id}</p>
                        <p className="text-[10px] font-bold text-brand uppercase mt-0.5">
                          Agent: <span className="text-text-dim">{trigger.agent_id ?? "-"}</span>
                        </p>
                      </div>
                      <span className={`h-1.5 w-1.5 rounded-full ${trigger.enabled ? 'bg-success animate-pulse' : 'bg-text-dim/30'}`} />
                    </div>
                    <div className="mt-3 flex justify-end gap-2">
                      <button
                        className="rounded-lg border border-border-subtle bg-surface px-2 py-1 text-[9px] font-bold text-text-dim hover:text-brand transition-all"
                        onClick={() => void handleToggleTrigger(trigger)}
                      >
                        {trigger.enabled ? "Disable" : "Enable"}
                      </button>
                      <button
                        className="rounded-lg border border-error/20 bg-error/5 px-2 py-1 text-[9px] font-bold text-error hover:bg-error/10 transition-all"
                        onClick={() => void handleDeleteTrigger(trigger)}
                      >
                        Delete
                      </button>
                    </div>
                  </article>
                ))}
                {triggers.length === 0 && <p className="text-xs text-text-dim font-medium py-4 text-center border border-dashed border-border-subtle rounded-xl">No active triggers</p>}
              </div>
            </section>

            <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
              <h2 className="text-lg font-black tracking-tight mb-1">System Cron</h2>
              <p className="mb-6 text-[10px] text-text-dim font-bold uppercase tracking-wider">Low-level worker jobs</p>
              
              <div className="grid gap-3">
                {cronJobs.map((job: CronJobItem, index: number) => (
                  <article key={index} className="rounded-xl border border-border-subtle bg-main/40 p-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="text-xs font-black truncate">{job.name || job.id || `job-${index}`}</p>
                        <p className="text-[10px] font-bold text-accent uppercase mt-0.5">
                          Schedule: <span className="text-text-dim">{job.schedule || "-"}</span>
                        </p>
                      </div>
                      <div className={`h-1.5 w-1.5 rounded-full ${job.enabled ? 'bg-success shadow-[0_0_5px_var(--success-color)]' : 'bg-text-dim/30'}`} />
                    </div>
                  </article>
                ))}
                {cronJobs.length === 0 && <p className="text-xs text-text-dim font-medium py-4 text-center border border-dashed border-border-subtle rounded-xl">No worker jobs found</p>}
              </div>
            </section>
          </div>
        </div>
      </div>
    </div>
  );
}
