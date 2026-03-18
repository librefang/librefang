import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  createSchedule, deleteSchedule, deleteTrigger, listAgents, listCronJobs,
  listSchedules, listTriggers, runSchedule, updateSchedule, updateTrigger,
  type ApiActionResponse, type CronJobItem, type ScheduleItem, type TriggerItem
} from "../api";

const REFRESH_MS = 30000;

export function SchedulerPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState<{type: "ok" | "error", text: string} | null>(null);
  const [name, setName] = useState("");
  const [cron, setCron] = useState("0 9 * * *");
  const [agentId, setAgentId] = useState("");
  const [message, setMessage] = useState("");

  const agentsQuery = useQuery({ queryKey: ["agents", "list", "scheduler"], queryFn: listAgents });
  const schedulesQuery = useQuery({ queryKey: ["schedules", "list"], queryFn: listSchedules, refetchInterval: REFRESH_MS });
  const triggersQuery = useQuery({ queryKey: ["triggers", "list"], queryFn: listTriggers });
  const cronJobsQuery = useQuery({ queryKey: ["cron-jobs", "list"], queryFn: listCronJobs });

  const createScheduleMutation = useMutation({ mutationFn: createSchedule });
  const runScheduleMutation = useMutation({ mutationFn: runSchedule });

  const schedules = useMemo(() => [...(schedulesQuery.data ?? [])].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")), [schedulesQuery.data]);

  const handleCreateSchedule = async (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    try {
      await createScheduleMutation.mutateAsync({ name, cron, agent_id: agentId, message, enabled: true });
      setFeedback({ type: "ok", text: t("common.success") });
      setName(""); setMessage("");
      await queryClient.invalidateQueries({ queryKey: ["schedules"] });
    } catch (e: any) { setFeedback({ type: "error", text: e.message }); }
  };

  const inputClass = "rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand outline-none transition-all";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 8V12L15 14" /><circle cx="12" cy="12" r="9" /></svg>
            {t("nav.automation")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("scheduler.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("scheduler.subtitle")}</p>
        </div>
        <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void schedulesQuery.refetch()}>
          <svg className={`h-3.5 w-3.5 ${schedulesQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" /></svg>
          {t("common.refresh")}
        </button>
      </header>

      <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
        <aside className="h-fit rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight">{t("scheduler.create_job")}</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">{t("scheduler.create_job_desc")}</p>
          <form className="flex flex-col gap-4" onSubmit={handleCreateSchedule}>
            <div><label className="text-[10px] font-black uppercase text-text-dim px-1">{t("scheduler.job_name")}</label><input value={name} onChange={(e) => setName(e.target.value)} placeholder={t("scheduler.job_name_placeholder")} className={`w-full ${inputClass}`} /></div>
            <div><label className="text-[10px] font-black uppercase text-text-dim px-1">{t("scheduler.cron_exp")}</label><input value={cron} onChange={(e) => setCron(e.target.value)} placeholder={t("scheduler.cron_exp_placeholder")} className={`w-full ${inputClass}`} /></div>
            <div><label className="text-[10px] font-black uppercase text-text-dim px-1">{t("scheduler.target_agent")}</label><select value={agentId} onChange={(e) => setAgentId(e.target.value)} className={`w-full ${inputClass}`}><option value="">{t("scheduler.select_agent")}</option>{agentsQuery.data?.map(a => <option key={a.id} value={a.id}>{a.name}</option>)}</select></div>
            <button type="submit" className="mt-2 rounded-xl bg-brand py-3 text-sm font-bold text-white shadow-lg hover:opacity-90 transition-all">{t("common.save")}</button>
          </form>
        </aside>

        <div className="flex flex-col gap-6">
          <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
            <h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.active_schedules")}</h2>
            <div className="grid gap-3 mt-6">
              {schedules.map((s) => (
                <article key={s.id} className="group rounded-xl border border-border-subtle bg-main/40 p-4 transition-all hover:border-brand/30">
                  <div className="flex items-start justify-between">
                    <div><p className="text-sm font-black">{s.name || s.id}</p><p className="text-[10px] font-bold text-brand uppercase mt-1">CRON: <span className="text-text-dim">{s.cron || "-"}</span></p></div>
                    <button className="rounded-lg border border-brand/20 bg-brand/10 px-3 py-1.5 text-[10px] font-bold text-brand hover:bg-brand/20 transition-all shadow-sm">{t("scheduler.run_now")}</button>
                  </div>
                </article>
              ))}
            </div>
          </section>
          
          <div className="grid gap-6 md:grid-cols-2">
            <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm"><h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.event_triggers")}</h2><div className="grid gap-3 mt-4">{(triggersQuery.data ?? []).map(t_ => (<article key={t_.id} className="rounded-xl border border-border-subtle bg-main/40 p-3"><p className="text-xs font-black truncate">{t_.id}</p></article>))}</div></section>
            <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm"><h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.system_cron")}</h2><div className="grid gap-3 mt-4">{(cronJobsQuery.data ?? []).map((j, i) => (<article key={i} className="rounded-xl border border-border-subtle bg-main/40 p-3"><p className="text-xs font-black truncate">{j.name || j.id}</p></article>))}</div></section>
          </div>
        </div>
      </div>
    </div>
  );
}
