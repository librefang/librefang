import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createSchedule, deleteSchedule, updateSchedule, listAgents, listSchedules, listTriggers, listWorkflows, updateTrigger, deleteTrigger, runSchedule } from "../api";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { PageHeader } from "../components/ui/PageHeader";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { Clock, Plus, Play, Trash2, Calendar, Zap, X, Loader2, AlertCircle, ChevronRight } from "lucide-react";
import { ScheduleModal } from "../components/ui/ScheduleModal";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { truncateId } from "../lib/string";

const REFRESH_MS = 30000;

export function SchedulerPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [showCreate, setShowCreate] = useState(false);
  useCreateShortcut(() => setShowCreate(true));
  const [showCronPicker, setShowCronPicker] = useState(false);
  const [name, setName] = useState("");
  const [cron, setCron] = useState("0 9 * * *");
  const [targetType, setTargetType] = useState<"agent" | "workflow">("agent");
  const [agentId, setAgentId] = useState("");
  const [workflowId, setWorkflowId] = useState("");
  const [message, setMessage] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<{ type: "schedule" | "trigger"; id: string } | null>(null);

  const agentsQuery = useQuery({ queryKey: ["agents", "list", "scheduler"], queryFn: listAgents });
  const schedulesQuery = useQuery({ queryKey: ["schedules", "list"], queryFn: listSchedules, refetchInterval: REFRESH_MS });
  const triggersQuery = useQuery({ queryKey: ["triggers", "list"], queryFn: listTriggers });
  const workflowsQuery = useQuery({ queryKey: ["workflows", "list", "scheduler"], queryFn: listWorkflows });

  const createMut = useMutation({ mutationFn: createSchedule });
  const runMut = useMutation({ mutationFn: runSchedule });
  const deleteScheduleMut = useMutation({ mutationFn: deleteSchedule });
  const toggleScheduleMut = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) => updateSchedule(id, { enabled }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["schedules"] }),
  });
  const toggleTriggerMut = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) => updateTrigger(id, { enabled }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["triggers"] }),
  });
  const deleteTriggerMut = useMutation({ mutationFn: deleteTrigger });

  const agents = agentsQuery.data ?? [];
  const workflows = workflowsQuery.data ?? [];
  const agentMap = useMemo(() => new Map(agents.map(a => [a.id, a])), [agents]);
  const schedules = useMemo(() => [...(schedulesQuery.data ?? [])].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")), [schedulesQuery.data]);
  const triggers = triggersQuery.data ?? [];

  const handleCreate = async (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    try {
      await createMut.mutateAsync({
        name, cron, message, enabled: true,
        ...(targetType === "agent" ? { agent_id: agentId } : { workflow_id: workflowId }),
      });
      setShowCreate(false); setName(""); setMessage(""); setCron("0 9 * * *"); setAgentId(""); setWorkflowId(""); setTargetType("agent");
      await queryClient.invalidateQueries({ queryKey: ["schedules"] });
    } catch (err: any) { addToast(err.message || t("common.error"), "error"); }
  };

  const handleDeleteSchedule = async (id: string) => {
    if (!confirmDelete || confirmDelete.type !== "schedule" || confirmDelete.id !== id) {
      setConfirmDelete({ type: "schedule", id });
      return;
    }
    setConfirmDelete(null);
    try {
      await deleteScheduleMut.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["schedules"] });
    } catch (err: any) { addToast(err.message || t("common.error"), "error"); }
  };

  const handleDeleteTrigger = async (id: string) => {
    if (!confirmDelete || confirmDelete.type !== "trigger" || confirmDelete.id !== id) {
      setConfirmDelete({ type: "trigger", id });
      return;
    }
    setConfirmDelete(null);
    try {
      await deleteTriggerMut.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["triggers"] });
    } catch (err: any) { addToast(err.message || t("common.error"), "error"); }
  };

  const cronHint = (expr: string) => {
    if (!expr) return "";
    const parts = expr.split(" ");
    if (parts.length !== 5) return expr;
    const [min, hr, , , dow] = parts;
    if (hr === "*" && min === "*") return t("scheduler.every_minute");
    if (min.startsWith("*/")) return t("scheduler.every_n_minutes", { defaultValue: `Every ${min.slice(2)} min`, n: min.slice(2) });
    if (hr.startsWith("*/")) return t("scheduler.every_n_hours", { n: hr.slice(2) });
    if (dow === "1-5" && min !== "*" && hr !== "*") return `${t("scheduler.weekdays", { defaultValue: "Weekdays" })} ${hr}:${min.padStart(2, "0")}`;
    if ((dow === "0" || dow === "7") && min !== "*" && hr !== "*") return `${t("scheduler.weekly")} ${hr}:${min.padStart(2, "0")}`;
    if (min !== "*" && hr !== "*") return `${hr}:${min.padStart(2, "0")}`;
    return expr;
  };

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

  const isConfirmingDelete = (type: "schedule" | "trigger", id: string) =>
    confirmDelete?.type === type && confirmDelete?.id === id;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.automation")}
        title={t("scheduler.title")}
        subtitle={t("scheduler.subtitle")}
        isFetching={schedulesQuery.isFetching}
        onRefresh={() => { schedulesQuery.refetch(); triggersQuery.refetch(); }}
        icon={<Calendar className="h-4 w-4" />}
        helpText={t("scheduler.help")}
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
      </div>

      {/* Schedule List */}
      <div>
        <h2 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("scheduler.active_schedules")}</h2>
        {schedulesQuery.isLoading ? (
          <ListSkeleton rows={2} />
        ) : schedules.length === 0 ? (
          <EmptyState
            icon={<Calendar className="w-7 h-7" />}
            title={t("scheduler.no_schedules")}
          />
        ) : (
          <div className="space-y-2 stagger-children">
            {schedules.map(s => {
              const agent = agentMap.get(s.agent_id || "");
              const isEnabled = s.enabled !== false;
              return (
                <div key={s.id} className={`p-3 sm:p-4 rounded-xl sm:rounded-2xl border transition-colors space-y-1.5 ${isEnabled ? "border-border-subtle hover:border-brand/30" : "border-border-subtle/50 opacity-50"}`}>
                  <div className="flex items-center gap-2 sm:gap-3">
                    <div className={`w-7 h-7 sm:w-8 sm:h-8 rounded-lg flex items-center justify-center shrink-0 ${isEnabled ? "bg-brand/10" : "bg-main"}`}>
                      <Clock className={`w-3.5 h-3.5 sm:w-4 sm:h-4 ${isEnabled ? "text-brand" : "text-text-dim/30"}`} />
                    </div>
                    <h3 className="text-xs sm:text-sm font-bold truncate flex-1 min-w-0">{s.name || s.description || truncateId(s.id)}</h3>
                    <button
                      onClick={() => toggleScheduleMut.mutate({ id: s.id, enabled: !isEnabled })}
                      className={`px-2 py-0.5 rounded-full text-[10px] font-bold transition-colors ${isEnabled ? "bg-success/10 text-success hover:bg-success/20" : "bg-main text-text-dim/40 hover:text-text-dim"}`}
                      disabled={toggleScheduleMut.isPending}
                    >
                      {isEnabled ? t("common.active") : t("common.disabled", { defaultValue: "OFF" })}
                    </button>
                    <div className="flex items-center gap-1 shrink-0">
                      <Button variant="secondary" size="sm" onClick={() => runMut.mutate(s.id)} disabled={runMut.isPending || !isEnabled}>
                        {runMut.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
                      </Button>
                      {isConfirmingDelete("schedule", s.id) ? (
                        <div className="flex items-center gap-1">
                          <button onClick={() => handleDeleteSchedule(s.id)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                          <button onClick={() => setConfirmDelete(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                        </div>
                      ) : (
                        <button onClick={() => handleDeleteSchedule(s.id)} className="p-1.5 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-colors">
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-2 sm:gap-3 pl-9 sm:pl-11 text-[9px] sm:text-[10px] text-text-dim/60 flex-wrap">
                    <span className="font-mono bg-main px-1 sm:px-1.5 py-0.5 rounded">{s.cron}</span>
                    <span className="text-text-dim hidden sm:inline">{cronHint(s.cron || "")}</span>
                    {agent && <span className="font-bold text-brand truncate">{t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}</span>}
                    {s.next_run && <span className="text-text-dim/40">{t("scheduler.next_run", { defaultValue: "Next" })}: {new Date(s.next_run).toLocaleString()}</span>}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Event Triggers */}
      <div>
        <h2 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("scheduler.event_triggers")}</h2>
        {triggers.length === 0 ? (
          <EmptyState
            icon={<Zap className="w-7 h-7" />}
            title={t("common.no_data")}
          />
        ) : (
          <div className="space-y-2 stagger-children">
            {triggers.map((tr: any) => {
              const isEnabled = tr.enabled !== false;
              return (
                <div key={tr.id} className={`p-3 sm:p-4 rounded-xl sm:rounded-2xl border transition-colors space-y-1.5 ${isEnabled ? "border-border-subtle hover:border-warning/30" : "border-border-subtle/50 opacity-50"}`}>
                  <div className="flex items-center gap-2 sm:gap-3">
                    <div className={`w-7 h-7 sm:w-8 sm:h-8 rounded-lg flex items-center justify-center shrink-0 ${isEnabled ? "bg-warning/10" : "bg-main"}`}>
                      <Zap className={`w-3.5 h-3.5 sm:w-4 sm:h-4 ${isEnabled ? "text-warning" : "text-text-dim/30"}`} />
                    </div>
                    <div className="min-w-0 flex-1">
                      <h3 className="text-xs sm:text-sm font-bold truncate">{tr.pattern || tr.name || truncateId(tr.id, 12)}</h3>
                    </div>
                    <button
                      onClick={() => toggleTriggerMut.mutate({ id: tr.id, enabled: !isEnabled })}
                      className={`px-2 py-0.5 rounded-full text-[10px] font-bold transition-colors ${isEnabled ? "bg-success/10 text-success hover:bg-success/20" : "bg-main text-text-dim/40 hover:text-text-dim"}`}
                      disabled={toggleTriggerMut.isPending}
                    >
                      {isEnabled ? t("common.active") : t("common.disabled", { defaultValue: "OFF" })}
                    </button>
                    <div className="flex items-center gap-1 shrink-0">
                      {isConfirmingDelete("trigger", tr.id) ? (
                        <div className="flex items-center gap-1">
                          <button onClick={() => handleDeleteTrigger(tr.id)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                          <button onClick={() => setConfirmDelete(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                        </div>
                      ) : (
                        <button onClick={() => handleDeleteTrigger(tr.id)} className="p-1.5 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-colors">
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      )}
                    </div>
                  </div>
                  {tr.prompt_template && (
                    <div className="pl-9 sm:pl-11">
                      <p className="text-[9px] sm:text-[10px] text-text-dim/60 truncate">{tr.prompt_template}</p>
                    </div>
                  )}
                  {tr.fire_count != null && (
                    <div className="flex items-center gap-2 pl-9 sm:pl-11 text-[9px] sm:text-[10px] text-text-dim/40">
                      <span>{t("scheduler.fired", { defaultValue: "Fired" })}: {tr.fire_count}{tr.max_fires ? `/${tr.max_fires}` : ""}</span>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Create Modal */}
      {showCreate && (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => setShowCreate(false)}>
          <div className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[440px] sm:max-w-[90vw] animate-fade-in-scale" onClick={e => e.stopPropagation()}>
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
                <button
                  type="button"
                  onClick={() => setShowCronPicker(true)}
                  className="w-full flex items-center justify-between px-3 py-2 rounded-xl border border-border-subtle bg-main hover:border-brand transition-colors text-left"
                >
                  <div>
                    <p className="text-sm">{cronHint(cron)}</p>
                    <p className="text-[10px] font-mono text-text-dim/50">{cron}</p>
                  </div>
                  <ChevronRight className="w-4 h-4 text-text-dim/40 flex-shrink-0" />
                </button>
              </div>
              {showCronPicker && (
                <ScheduleModal
                  title={t("scheduler.cron_exp")}
                  initialCron={cron}
                  onSave={(c) => { setCron(c); setShowCronPicker(false); }}
                  onClose={() => setShowCronPicker(false)}
                />
              )}
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("scheduler.target", { defaultValue: "Target" })}</label>
                <div className="flex gap-1 mb-2">
                  <button type="button" onClick={() => setTargetType("agent")}
                    className={`flex-1 py-1.5 rounded-lg text-[11px] font-bold transition-colors ${targetType === "agent" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
                    {t("scheduler.target_agent")}
                  </button>
                  <button type="button" onClick={() => setTargetType("workflow")}
                    className={`flex-1 py-1.5 rounded-lg text-[11px] font-bold transition-colors ${targetType === "workflow" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
                    {t("scheduler.target_workflow", { defaultValue: "Workflow" })}
                  </button>
                </div>
                {targetType === "agent" ? (
                  <select value={agentId} onChange={e => setAgentId(e.target.value)} className={inputClass}>
                    <option value="">{t("scheduler.select_agent")}</option>
                    {agents.map(a => <option key={a.id} value={a.id}>{a.name}</option>)}
                  </select>
                ) : (
                  <select value={workflowId} onChange={e => setWorkflowId(e.target.value)} className={inputClass}>
                    <option value="">{t("scheduler.select_workflow", { defaultValue: "Select workflow..." })}</option>
                    {workflows.map(w => <option key={w.id} value={w.id}>{w.name}</option>)}
                  </select>
                )}
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
