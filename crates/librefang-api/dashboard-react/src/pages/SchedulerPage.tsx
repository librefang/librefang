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
      // Some legacy schedule entries store `agent` but not `agent_id`.
      // Normalize before run so the backend can resolve target agent.
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

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Scheduler</h1>
          <p className="text-sm text-slate-400">Schedules, triggers, and cron jobs.</p>
        </div>
        <div className="flex items-center gap-2">
          <span className="rounded-full border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs text-slate-300">
            {schedules.length} schedules · {triggers.length} triggers
          </span>
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void refreshAll()}
            disabled={schedulesQuery.isFetching || triggersQuery.isFetching || cronJobsQuery.isFetching}
          >
            Refresh
          </button>
        </div>
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

      <div className="grid gap-3 xl:grid-cols-[340px_1fr]">
        <aside className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="mb-3 mt-0 text-base font-semibold">Create Schedule</h2>
          <form className="flex flex-col gap-2" onSubmit={handleCreateSchedule}>
            <input
              type="text"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Name"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createScheduleMutation.isPending}
            />
            <input
              type="text"
              value={cron}
              onChange={(event) => setCron(event.target.value)}
              placeholder="Cron (e.g. 0 9 * * *)"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createScheduleMutation.isPending}
            />
            <select
              value={agentId}
              onChange={(event) => setAgentId(event.target.value)}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createScheduleMutation.isPending}
            >
              <option value="">Select agent</option>
              {agents.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}
                </option>
              ))}
            </select>
            <textarea
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              placeholder="Message to send when schedule fires"
              rows={4}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createScheduleMutation.isPending}
            />
            <button
              className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
              type="submit"
              disabled={
                createScheduleMutation.isPending ||
                name.trim().length === 0 ||
                cron.trim().length === 0 ||
                agentId.trim().length === 0
              }
            >
              {createScheduleMutation.isPending ? "Creating..." : "Create"}
            </button>
          </form>
        </aside>

        <section className="flex flex-col gap-3">
          <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h2 className="mb-3 mt-0 text-base font-semibold">Schedules</h2>
            {schedulesError ? <p className="text-sm text-rose-300">{schedulesError}</p> : null}
            {schedulesQuery.isLoading && schedules.length === 0 ? (
              <p className="text-sm text-slate-400">Loading schedules...</p>
            ) : null}
            {!schedulesQuery.isLoading && schedules.length === 0 ? (
              <p className="text-sm text-slate-400">No schedules found.</p>
            ) : null}
            <div className="grid gap-2">
              {schedules.map((schedule) => (
                <article key={schedule.id} className="rounded-lg border border-slate-700 bg-slate-950/70 p-3">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div>
                      <p className="text-sm font-semibold">{schedule.name ?? schedule.id}</p>
                      <p className="text-xs text-slate-400">
                        cron: {schedule.cron ?? schedule.schedule_input ?? "-"} · agent: {scheduleAgent(schedule)}
                      </p>
                      <p className="text-xs text-slate-500">
                        created: {dateText(schedule.created_at)} · last run: {dateText(schedule.last_run)}
                      </p>
                    </div>
                    <span
                      className={`rounded-full border px-2 py-1 text-[11px] ${
                        schedule.enabled
                          ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                          : "border-slate-700 bg-slate-800/60 text-slate-300"
                      }`}
                    >
                      {schedule.enabled ? "Enabled" : "Disabled"}
                    </span>
                  </div>
                  {schedule.description || schedule.message ? (
                    <p className="mt-2 text-xs text-slate-300">{schedule.description ?? schedule.message}</p>
                  ) : null}
                  <div className="mt-2 flex flex-wrap justify-end gap-2">
                    <button
                      className="rounded-lg border border-slate-600 bg-slate-800 px-2 py-1 text-[11px] text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
                      type="button"
                      onClick={() => void handleToggleSchedule(schedule)}
                      disabled={pendingScheduleToggleId === schedule.id}
                    >
                      {pendingScheduleToggleId === schedule.id
                        ? "Updating..."
                        : schedule.enabled
                          ? "Disable"
                          : "Enable"}
                    </button>
                    <button
                      className="rounded-lg border border-sky-700 bg-sky-700/20 px-2 py-1 text-[11px] text-sky-200 transition hover:bg-sky-700/30 disabled:cursor-not-allowed disabled:opacity-60"
                      type="button"
                      onClick={() => void handleRunSchedule(schedule)}
                      disabled={pendingScheduleRunId === schedule.id}
                    >
                      {pendingScheduleRunId === schedule.id ? "Running..." : "Run Now"}
                    </button>
                    <button
                      className="rounded-lg border border-rose-700 bg-rose-700/20 px-2 py-1 text-[11px] text-rose-200 transition hover:bg-rose-700/30 disabled:cursor-not-allowed disabled:opacity-60"
                      type="button"
                      onClick={() => void handleDeleteSchedule(schedule)}
                      disabled={pendingScheduleDeleteId === schedule.id}
                    >
                      {pendingScheduleDeleteId === schedule.id ? "Deleting..." : "Delete"}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          </article>

          <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h2 className="mb-3 mt-0 text-base font-semibold">Triggers</h2>
            {triggersError ? <p className="text-sm text-rose-300">{triggersError}</p> : null}
            {triggersQuery.isLoading && triggers.length === 0 ? (
              <p className="text-sm text-slate-400">Loading triggers...</p>
            ) : null}
            {!triggersQuery.isLoading && triggers.length === 0 ? (
              <p className="text-sm text-slate-400">No triggers found.</p>
            ) : null}
            <div className="grid gap-2">
              {triggers.map((trigger) => (
                <article key={trigger.id} className="rounded-lg border border-slate-700 bg-slate-950/70 p-3">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div>
                      <p className="text-sm font-semibold">{trigger.id}</p>
                      <p className="text-xs text-slate-400">agent: {trigger.agent_id ?? "-"}</p>
                      <p className="text-xs text-slate-500">
                        fires: {trigger.fire_count ?? 0}/{trigger.max_fires ?? 0}
                      </p>
                    </div>
                    <span
                      className={`rounded-full border px-2 py-1 text-[11px] ${
                        trigger.enabled
                          ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                          : "border-slate-700 bg-slate-800/60 text-slate-300"
                      }`}
                    >
                      {trigger.enabled ? "Enabled" : "Disabled"}
                    </span>
                  </div>
                  {trigger.prompt_template ? (
                    <p className="mt-2 line-clamp-2 text-xs text-slate-300">{trigger.prompt_template}</p>
                  ) : null}
                  <div className="mt-2 flex flex-wrap justify-end gap-2">
                    <button
                      className="rounded-lg border border-slate-600 bg-slate-800 px-2 py-1 text-[11px] text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
                      type="button"
                      onClick={() => void handleToggleTrigger(trigger)}
                      disabled={pendingTriggerToggleId === trigger.id}
                    >
                      {pendingTriggerToggleId === trigger.id
                        ? "Updating..."
                        : trigger.enabled
                          ? "Disable"
                          : "Enable"}
                    </button>
                    <button
                      className="rounded-lg border border-rose-700 bg-rose-700/20 px-2 py-1 text-[11px] text-rose-200 transition hover:bg-rose-700/30 disabled:cursor-not-allowed disabled:opacity-60"
                      type="button"
                      onClick={() => void handleDeleteTrigger(trigger)}
                      disabled={pendingTriggerDeleteId === trigger.id}
                    >
                      {pendingTriggerDeleteId === trigger.id ? "Deleting..." : "Delete"}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          </article>

          <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h2 className="mb-3 mt-0 text-base font-semibold">Cron Jobs</h2>
            {cronJobsError ? <p className="text-sm text-rose-300">{cronJobsError}</p> : null}
            {cronJobsQuery.isLoading && cronJobs.length === 0 ? (
              <p className="text-sm text-slate-400">Loading cron jobs...</p>
            ) : null}
            {!cronJobsQuery.isLoading && cronJobs.length === 0 ? (
              <p className="text-sm text-slate-400">No cron jobs found.</p>
            ) : null}
            <div className="grid gap-2">
              {cronJobs.map((job: CronJobItem, index: number) => (
                <article
                  key={(typeof job.id === "string" ? job.id : `cron-${index}`) as string}
                  className="rounded-lg border border-slate-700 bg-slate-950/70 p-3"
                >
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div>
                      <p className="text-sm font-semibold">
                        {typeof job.name === "string" && job.name.length > 0 ? job.name : job.id ?? `job-${index}`}
                      </p>
                      <p className="text-xs text-slate-400">{typeof job.schedule === "string" ? job.schedule : "-"}</p>
                    </div>
                    <span
                      className={`rounded-full border px-2 py-1 text-[11px] ${
                        job.enabled
                          ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                          : "border-slate-700 bg-slate-800/60 text-slate-300"
                      }`}
                    >
                      {job.enabled ? "Enabled" : "Disabled"}
                    </span>
                  </div>
                </article>
              ))}
            </div>
          </article>
        </section>
      </div>
    </section>
  );
}
