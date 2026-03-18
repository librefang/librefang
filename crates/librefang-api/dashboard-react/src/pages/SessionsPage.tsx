import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { deleteSession, listAgents, listSessions, switchAgentSession } from "../api";

const REFRESH_MS = 30000;

export function SessionsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);

  const sessionsQuery = useQuery({ queryKey: ["sessions", "list"], queryFn: listSessions, refetchInterval: REFRESH_MS });
  const agentsQuery = useQuery({ queryKey: ["agents", "list", "sessions"], queryFn: listAgents });

  const switchMutation = useMutation({ mutationFn: ({ agentId, sessionId }: any) => switchAgentSession(agentId, sessionId) });
  const deleteMutation = useMutation({ mutationFn: (id: string) => deleteSession(id) });

  const sessions = sessionsQuery.data ?? [];
  const agents = agentsQuery.data ?? [];

  async function handleSwitch(agentId: string, sessionId: string) {
    setPendingId(sessionId);
    try { await switchMutation.mutateAsync({ agentId, sessionId }); await queryClient.invalidateQueries({ queryKey: ["sessions"] }); }
    finally { setPendingId(null); }
  }

  async function handleDelete(id: string) {
    if (!window.confirm(t("common.confirm"))) return;
    setPendingId(id);
    try { await deleteMutation.mutateAsync(id); await queryClient.invalidateQueries({ queryKey: ["sessions"] }); }
    finally { setPendingId(null); }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" /></svg>
            {t("nav.sessions")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("sessions.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("sessions.subtitle")}</p>
        </div>
        <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void sessionsQuery.refetch()}>
          {t("common.refresh")}
        </button>
      </header>

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
                  <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path d="M12 8v4l3 2" /></svg>
                  {t("sessions.last_activity")}: <span className="text-slate-700 dark:text-slate-200">{new Date(s.created_at || "").toLocaleTimeString()}</span>
                </div>
              </div>
              <div className="flex gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                {!s.active && <button onClick={() => handleSwitch(s.agent_id!, s.session_id)} className="px-4 py-1.5 rounded-lg border border-brand/20 bg-brand/5 text-brand text-[10px] font-black uppercase hover:bg-brand/10">{t("common.resume")}</button>}
                <button onClick={() => handleDelete(s.session_id)} className="px-4 py-1.5 rounded-lg border border-error/20 bg-error/5 text-error text-[10px] font-black uppercase hover:bg-error/10">{t("common.close")}</button>
              </div>
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}
