import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { deleteSession, listAgents, listSessions, switchAgentSession } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { useUIStore } from "../lib/store";
import { Clock } from "lucide-react";

const REFRESH_MS = 30000;

export function SessionsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  const sessionsQuery = useQuery({ queryKey: ["sessions", "list"], queryFn: listSessions, refetchInterval: REFRESH_MS });
  const agentsQuery = useQuery({ queryKey: ["agents", "list", "sessions"], queryFn: listAgents });

  const switchMutation = useMutation({ mutationFn: ({ agentId, sessionId }: any) => switchAgentSession(agentId, sessionId) });
  const deleteMutation = useMutation({ mutationFn: (id: string) => deleteSession(id) });

  const sessions = sessionsQuery.data ?? [];

  async function handleSwitch(agentId: string, sessionId: string) {
    setPendingId(sessionId);
    try {
      await switchMutation.mutateAsync({ agentId, sessionId });
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setPendingId(null);
    }
  }

  async function handleDelete(id: string) {
    if (!window.confirm(t("common.confirm"))) return;
    setPendingId(id);
    try {
      await deleteMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setPendingId(null);
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.sessions")}
        title={t("sessions.title")}
        subtitle={t("sessions.subtitle")}
        isFetching={sessionsQuery.isFetching}
        onRefresh={() => void sessionsQuery.refetch()}
        icon={<Clock className="h-4 w-4" />}
      />

      {sessionsQuery.isLoading ? (
        <ListSkeleton rows={4} />
      ) : sessions.length === 0 ? (
        <EmptyState
          title={t("common.no_data")}
          icon={<Clock className="h-6 w-6" />}
        />
      ) : (
        <div className="grid gap-4">
          {sessions.map((s) => (
            <article key={s.session_id} className="group rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm transition-all hover:border-brand/30">
              <div className="flex items-center justify-between gap-4">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-3 mb-1">
                    <h2 className="text-sm font-black truncate">{s.session_id.slice(0, 12)}...</h2>
                    <span className={`px-2 py-0.5 rounded-lg border text-[9px] font-black uppercase tracking-widest ${s.active ? 'border-success/20 bg-success/10 text-success' : 'border-border-subtle bg-main text-text-dim'}`}>
                      {s.active ? t("common.active") : t("common.idle")}
                    </span>
                  </div>
                  <div className="flex items-center gap-1.5 mt-2 text-[10px] font-bold text-text-dim uppercase tracking-wider">
                    <Clock className="h-3 w-3" />
                    {t("sessions.last_activity")}: <span className="text-slate-700 dark:text-slate-200">{new Date(s.created_at || "").toLocaleTimeString()}</span>
                  </div>
                </div>
                <div className="flex gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                  {!s.active && <button onClick={() => handleSwitch(s.agent_id!, s.session_id)} disabled={pendingId === s.session_id} className="px-4 py-1.5 rounded-lg border border-brand/20 bg-brand/5 text-brand text-[10px] font-black uppercase hover:bg-brand/10 disabled:opacity-50">{t("common.resume")}</button>}
                  <button onClick={() => handleDelete(s.session_id)} disabled={pendingId === s.session_id} className="px-4 py-1.5 rounded-lg border border-error/20 bg-error/5 text-error text-[10px] font-black uppercase hover:bg-error/10 disabled:opacity-50">{t("common.close")}</button>
                </div>
              </div>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}
