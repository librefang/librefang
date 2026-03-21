import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createSchedule, deleteSchedule, listAgents, listSchedules, listTriggers, listCronJobs, runSchedule } from "../api";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { PageHeader } from "../components/ui/PageHeader";
import { useUIStore } from "../lib/store";
import { Clock, Plus, Play, Trash2, Calendar, Zap, X, Loader2, AlertCircle } from "lucide-react";
import { ListSkeleton } from "../components/ui/Skeleton";

const REFRESH_MS = 30000;

export function SchedulerPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState("");
  const [cron, setCron] = useState("0 9 * * *");
  const [agentId, setAgentId] = useState("");
  const [message, setMessage] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const agentsQuery = useQuery({ queryKey: ["agents", "list", "scheduler"], queryFn: listAgents });
  const schedulesQuery = useQuery({ queryKey: ["schedules", "list"], queryFn: listSchedules, refetchInterval: REFRESH_MS });
  const triggersQuery = useQuery({ queryKey: ["triggers", "list"], queryFn: listTriggers });
  const cronJobsQuery = useQuery({ queryKey: ["cron-jobs", "list"], queryFn: listCronJobs });

  const createMut = useMutation({ mutationFn: createSchedule });
  const runMut = useMutation({ mutationFn: runSchedule });
  const deleteMut = useMutation({ mutationFn: deleteSchedule });

  const agents = agentsQuery.data ?? [];
  const agentMap = useMemo(() => new Map(agents.map(a => [a.id, a])), [agents]);
  const schedules = useMemo(() => [...(schedulesQuery.data ?? [])].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")), [schedulesQuery.data]);
  const triggers = triggersQuery.data ?? [];
  const cronJobs = cronJobsQuery.data ?? [];

  const handleCreate = async (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    try {
      await createMut.mutateAsync({ name, cron, agent_id: agentId, message, enabled: true });
      setShowCreate(false); setName(""); setMessage(""); setCron("0 9 * * *"); setAgentId("");
      await queryClient.invalidateQueries({ queryKey: ["schedules"] });
    } catch (err: any) { addToast(err.message || t("common.error"), "error"); }
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) { setConfirmDeleteId(id); return; }
    setConfirmDeleteId(null);
    try {
      await deleteMut.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["schedules"] });
    } catch (err: any) { addToast(err.message || t("common.error"), "error"); }
  };

  const cronHint = (expr: string) => {
    if (!expr) return "";
    const parts = expr.split(" ");
    if (parts.length !== 5) return expr;
    const [min, hr, , , dow] = parts;
    if (hr === "*" && min === "*") return t("scheduler.every_minute");
    if (hr.startsWith("*/")) return t("scheduler.every_n_hours", { n: hr.slice(2) });
    if (dow === "0" || dow === "7") return t("scheduler.weekly");
    if (min !== "*" && hr !== "*") return `${hr}:${min.padStart(2, "0")}`;
    return expr;
  };

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.automation")}
        title={t("scheduler.title")}
        subtitle={t("scheduler.subtitle")}
        isFetching={schedulesQuery.isFetching}
        onRefresh={() => { schedulesQuery.refetch(); triggersQuery.refetch(); cronJobsQuery.refetch(); }}
        icon={<Calendar className="h-4 w-4" />}
        actions={
          <Button variant="primary" onClick={() => setShowCreate(true)}>
            <Plus className="w-4 h-4" /> {t("scheduler.create_job")}
          </Button>
        }
      />

      {/* Stats */}
      <div className="flex gap-3">
        <Badge variant="brand">{schedules.length} {t("scheduler.schedules")}</Badge>
        <Badge variant="default">{triggers.length} {t("scheduler.triggers_label")}</Badge>
        <Badge variant="default">{cronJobs.length} {t("scheduler.cron_jobs")}</Badge>
      </div>

      {/* Schedule List */}
      <div>
        <h2 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("scheduler.active_schedules")}</h2>
        {schedulesQuery.isLoading ? (
          <ListSkeleton rows={2} />
        ) : schedules.length === 0 ? (
          <div className="text-center py-12 rounded-2xl border border-dashed border-border-subtle">
            <Calendar className="w-8 h-8 text-text-dim/30 mx-auto mb-2" />
            <p className="text-sm text-text-dim">{t("scheduler.no_schedules")}</p>
          </div>
        ) : (
          <div className="space-y-2">
            {schedules.map(s => {
              const agent = agentMap.get(s.agent_id || "");
              return (
                <div key={s.id} className="flex items-center gap-4 p-4 rounded-2xl border border-border-subtle hover:border-brand/30 transition-all">
                  <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
                    <Clock className="w-5 h-5 text-brand" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <h3 className="text-sm font-bold truncate">{s.name || s.description || s.id.slice(0, 8)}</h3>
                      {s.enabled !== false && <Badge variant="success">{t("common.active")}</Badge>}
                    </div>
                    <div className="flex items-center gap-3 mt-1 text-[10px] text-text-dim/60">
                      <span className="font-mono bg-main px-1.5 py-0.5 rounded">{s.cron}</span>
                      <span className="text-text-dim">{cronHint(s.cron || "")}</span>
                      {agent && <span className="font-bold text-brand">{agent.name}</span>}
                      {!agent && s.agent && <span className="font-bold text-brand">{s.agent}</span>}
                    </div>
                  </div>
                  <div className="flex items-center gap-1 shrink-0">
                    <Button variant="secondary" size="sm" onClick={() => runMut.mutate(s.id)} disabled={runMut.isPending}>
                      {runMut.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
                    </Button>
                    {confirmDeleteId === s.id ? (
                      <div className="flex items-center gap-1">
                        <button onClick={() => handleDelete(s.id)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                        <button onClick={() => setConfirmDeleteId(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                      </div>
                    ) : (
                      <button onClick={() => handleDelete(s.id)} className="p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-all">
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Triggers & Cron Jobs */}
      <div className="grid gap-6 md:grid-cols-2">
        <div>
          <h2 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("scheduler.event_triggers")}</h2>
          {triggers.length === 0 ? (
            <div className="text-center py-8 rounded-2xl border border-dashed border-border-subtle">
              <Zap className="w-6 h-6 text-text-dim/30 mx-auto mb-1" />
              <p className="text-xs text-text-dim">{t("common.no_data")}</p>
            </div>
          ) : (
            <div className="space-y-1.5">
              {triggers.map((tr: any) => (
                <div key={tr.id} className="flex items-center gap-3 p-3 rounded-xl border border-border-subtle hover:border-brand/30 transition-all">
                  <Zap className="w-4 h-4 text-warning shrink-0" />
                  <div className="min-w-0 flex-1">
                    <p className="text-xs font-bold truncate">{tr.pattern || tr.name || tr.id?.slice(0, 12)}</p>
                    {tr.prompt_template && <p className="text-[9px] text-text-dim truncate">{tr.prompt_template}</p>}
                  </div>
                  {tr.enabled !== false && <Badge variant="success" className="shrink-0">ON</Badge>}
                </div>
              ))}
            </div>
          )}
        </div>
        <div>
          <h2 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("scheduler.system_cron")}</h2>
          {cronJobs.length === 0 ? (
            <div className="text-center py-8 rounded-2xl border border-dashed border-border-subtle">
              <Clock className="w-6 h-6 text-text-dim/30 mx-auto mb-1" />
              <p className="text-xs text-text-dim">{t("common.no_data")}</p>
            </div>
          ) : (
            <div className="space-y-1.5">
              {cronJobs.map((j: any, i: number) => (
                <div key={j.id || i} className="flex items-center gap-3 p-3 rounded-xl border border-border-subtle hover:border-brand/30 transition-all">
                  <Clock className="w-4 h-4 text-brand shrink-0" />
                  <div className="min-w-0 flex-1">
                    <p className="text-xs font-bold truncate">{j.name || j.id?.slice(0, 12)}</p>
                    <p className="text-[9px] text-text-dim font-mono">{j.cron || j.schedule || "-"}</p>
                  </div>
                  {j.enabled !== false && <Badge variant="success" className="shrink-0">ON</Badge>}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Create Modal */}
      {showCreate && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => setShowCreate(false)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[440px] max-w-[90vw] animate-fade-in-up" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <h3 className="text-sm font-bold">{t("scheduler.create_job")}</h3>
              <button onClick={() => setShowCreate(false)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <form onSubmit={handleCreate} className="p-5 space-y-4">
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("scheduler.job_name")}</label>
                <input value={name} onChange={e => setName(e.target.value)} placeholder={t("scheduler.job_name_placeholder")} className={inputClass} />
              </div>
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("scheduler.cron_exp")}</label>
                <input value={cron} onChange={e => setCron(e.target.value)} placeholder="0 9 * * *" className={`${inputClass} font-mono`} />
                <p className="text-[9px] text-text-dim/50 mt-1">{cronHint(cron)}</p>
              </div>
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("scheduler.target_agent")}</label>
                <select value={agentId} onChange={e => setAgentId(e.target.value)} className={inputClass}>
                  <option value="">{t("scheduler.select_agent")}</option>
                  {agents.map(a => <option key={a.id} value={a.id}>{a.name}</option>)}
                </select>
              </div>
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("scheduler.message")}</label>
                <textarea value={message} onChange={e => setMessage(e.target.value)} rows={3}
                  placeholder={t("scheduler.message_placeholder")} className={`${inputClass} resize-none`} />
              </div>
              {createMut.error && (
                <div className="flex items-center gap-2 text-error text-xs"><AlertCircle className="w-4 h-4" /> {(createMut.error as any)?.message}</div>
              )}
              <div className="flex gap-2 pt-2">
                <Button type="submit" variant="primary" className="flex-1" disabled={createMut.isPending || !name.trim()}>
                  {createMut.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
                  {t("scheduler.create_job")}
                </Button>
                <Button type="button" variant="secondary" onClick={() => setShowCreate(false)}>{t("common.cancel")}</Button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}
