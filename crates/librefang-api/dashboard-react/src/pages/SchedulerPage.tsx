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
import { Card } from "../components/ui/Card";
import { Input } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
import { Button } from "../components/ui/Button";
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
          <Card padding="lg" hover className="h-fit">
            <h2 className="text-lg font-black tracking-tight">{t("scheduler.create_job")}</h2>
            <p className="mb-6 text-xs text-text-dim font-medium">{t("scheduler.create_job_desc")}</p>
            <form className="flex flex-col gap-4" onSubmit={handleCreateSchedule}>
              <Input label={t("scheduler.job_name")} value={name} onChange={(e) => setName(e.target.value)} placeholder={t("scheduler.job_name_placeholder")} />
              <Input label={t("scheduler.cron_exp")} value={cron} onChange={(e) => setCron(e.target.value)} placeholder={t("scheduler.cron_exp_placeholder")} />
              <Select
                label={t("scheduler.target_agent")}
                value={agentId}
                onChange={(e) => setAgentId(e.target.value)}
                options={[{ value: "", label: t("scheduler.select_agent") }, ...(agentsQuery.data?.map(a => ({ value: a.id, label: a.name })) ?? [])]}
              />
              <Button type="submit" variant="primary" disabled={createScheduleMutation.isPending || !name.trim()} className="mt-2">
                {createScheduleMutation.isPending ? t("common.loading") : t("common.save")}
              </Button>
            </form>
          </Card>

          <div className="flex flex-col gap-6">
            <Card padding="lg" hover>
              <h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.active_schedules")}</h2>
              {schedules.length === 0 ? (
                <EmptyState
                  title={t("common.no_data")}
                  icon={<Clock className="h-6 w-6" />}
                />
              ) : (
                <div className="grid gap-3 mt-6">
                  {schedules.map((s) => (
                    <Card key={s.id} hover padding="sm">
                      <div className="flex items-start justify-between">
                        <div><p className="text-sm font-black">{s.name || s.id}</p><p className="text-[10px] font-bold text-brand uppercase mt-1">CRON: <span className="text-text-dim">{s.cron || "-"}</span></p></div>
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => handleRunSchedule(s.id)}
                          disabled={runScheduleMutation.isPending}
                        >
                          {runScheduleMutation.isPending ? t("common.loading") : t("scheduler.run_now")}
                        </Button>
                      </div>
                    </Card>
                  ))}
                </div>
              )}
            </Card>

            <div className="grid gap-6 md:grid-cols-2">
              <Card padding="lg" hover>
                <h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.event_triggers")}</h2>
                {(!triggersQuery.data || !Array.isArray(triggersQuery.data) || triggersQuery.data.length === 0) ? (
                  <p className="text-xs text-text-dim italic mt-4">{t("common.no_data")}</p>
                ) : (
                  <div className="grid gap-3 mt-4">{triggersQuery.data.map(t_ => (<Card key={t_.id} padding="sm"><p className="text-xs font-black truncate">{t_.id}</p></Card>))}</div>
                )}
              </Card>
              <Card padding="lg" hover>
                <h2 className="text-lg font-black tracking-tight mb-1">{t("scheduler.system_cron")}</h2>
                {(!cronJobsQuery.data || !Array.isArray(cronJobsQuery.data) || cronJobsQuery.data.length === 0) ? (
                  <p className="text-xs text-text-dim italic mt-4">{t("common.no_data")}</p>
                ) : (
                  <div className="grid gap-3 mt-4">{cronJobsQuery.data.map((j, i) => (<Card key={i} padding="sm"><p className="text-xs font-black truncate">{j.name || j.id}</p></Card>))}</div>
                )}
              </Card>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
