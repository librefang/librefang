import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  createSchedule, listAgents,
  listSchedules, listTriggers, listCronJobs, runSchedule
} from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton, ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { useUIStore } from "../lib/store";
import { Clock } from "lucide-react";

const REFRESH_MS = 30000;

export function SchedulerPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
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
      addToast(t("common.success"), "success");
      setName(""); setMessage("");
      await queryClient.invalidateQueries({ queryKey: ["schedules"] });
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const handleRunSchedule = async (id: string) => {
    try {
      await runScheduleMutation.mutateAsync(id);
      addToast(t("common.success"), "success");
    } catch (err: any) {
      addToast(err.message || t("common.error"), "error");
    }
  };

  const inputClass = "rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand outline-none transition-all";
  const isLoading = schedulesQuery.isLoading || triggersQuery.isLoading || cronJobsQuery.isLoading;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.automation")}
        title={t("scheduler.title")}
        subtitle={t("scheduler.subtitle")}
        isFetching={schedulesQuery.isFetching}
        onRefresh={() => void schedulesQuery.refetch()}
        icon={<Clock className="h-4 w-4" />}
      />

      {isLoading ? (
        <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
          <CardSkeleton />
          <div className="space-y-6">
            <ListSkeleton rows={2} />
          </div>
        </div>
      ) : (
        <div className="grid gap-6 xl:grid-cols-[360px_1fr]">
          <aside className="h-fit rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5 hover:border-brand/30 transition-all">
            <h2 className="text-lg font-black tracking-tight">{t("scheduler.create_job")}</h2>
            <p className="mb-6 text-xs text-text-dim font-medium">{t("scheduler.create_job_desc")}</p>
            <form className="flex flex-col gap-4" onSubmit={handleCreateSchedule}>
              <div><label className="text-[10px] font-black uppercase text-text-dim px-1">{t("scheduler.job_name")}</label><input value={name} onChange={(e) => setName(e.target.value)} placeholder={t("scheduler.job_name_placeholder")} className={`w-full ${inputClass}`} /></div>
              <div><label className="text-[10px] font-black uppercase text-text-dim px-1">{t("scheduler.cron_exp")}</label><input value={cron} onChange={(e) => setCron(e.target.value)} placeholder={t("scheduler.cron_exp_placeholder")} className={`w-full ${inputClass}`} /></div>
              <div><label className="text-[10px] font-black uppercase text-text-dim px-1">{t("scheduler.target_agent")}</label><select value={agentId} onChange={(e) => setAgentId(e.target.value)} className={`w-full ${inputClass}`}><option value="">{t("scheduler.select_agent")}</option>{agentsQuery.data?.map(a => <option key={a.id} value={a.id}>{a.name}</option>)}</select></div>
              <button type="submit" disabled={createScheduleMutation.isPending || !name.trim()} className="mt-2 rounded-xl bg-brand py-3 text-sm font-bold text-white shadow-lg hover:opacity-90 disabled:opacity-50 transition-all">{createScheduleMutation.isPending ? t("common.loading") : t("common.save")}</button>
            </form>
          </aside>

          <div className="flex flex-col gap-6">
            <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm hover:border-brand/30 transition-all">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.active_schedules")}</h2>
              {schedules.length === 0 ? (
                <EmptyState
                  title={t("common.no_data")}
                  icon={<Clock className="h-6 w-6" />}
                />
              ) : (
                <div className="grid gap-3 mt-6">
                  {schedules.map((s) => (
                    <article key={s.id} className="group rounded-xl border border-border-subtle bg-main/40 p-4 transition-all hover:border-brand/30">
                      <div className="flex items-start justify-between">
                        <div><p className="text-sm font-black">{s.name || s.id}</p><p className="text-[10px] font-bold text-brand uppercase mt-1">CRON: <span className="text-text-dim">{s.cron || "-"}</span></p></div>
                        <button
                          onClick={() => handleRunSchedule(s.id)}
                          disabled={runScheduleMutation.isPending}
                          className="rounded-lg border border-brand/20 bg-brand/10 px-3 py-1.5 text-[10px] font-bold text-brand hover:bg-brand/20 transition-all shadow-sm disabled:opacity-50"
                        >
                          {runScheduleMutation.isPending ? t("common.loading") : t("scheduler.run_now")}
                        </button>
                      </div>
                    </article>
                  ))}
                </div>
              )}
            </section>

            <div className="grid gap-6 md:grid-cols-2">
              <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm hover:border-brand/30 transition-all">
                <h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.event_triggers")}</h2>
                {(triggersQuery.data ?? []).length === 0 ? (
                  <p className="text-xs text-text-dim italic mt-4">{t("common.no_data")}</p>
                ) : (
                  <div className="grid gap-3 mt-4">{(triggersQuery.data ?? []).map(t_ => (<article key={t_.id} className="rounded-xl border border-border-subtle bg-main/40 p-3"><p className="text-xs font-black truncate">{t_.id}</p></article>))}</div>
                )}
              </section>
              <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm hover:border-brand/30 transition-all">
                <h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.system_cron")}</h2>
                {(cronJobsQuery.data ?? []).length === 0 ? (
                  <p className="text-xs text-text-dim italic mt-4">{t("common.no_data")}</p>
                ) : (
                  <div className="grid gap-3 mt-4">{(cronJobsQuery.data ?? []).map((j, i) => (<article key={i} className="rounded-xl border border-border-subtle bg-main/40 p-3"><p className="text-xs font-black truncate">{j.name || j.id}</p></article>))}</div>
                )}
              </section>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
