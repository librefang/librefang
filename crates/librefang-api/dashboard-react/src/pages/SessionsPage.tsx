import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { deleteSession, listAgents, listSessions, switchAgentSession, type AgentItem, type SessionItem } from "../api";

const REFRESH_MS = 30000;

export function SessionsPage() {
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);

  const sessionsQuery = useQuery({
    queryKey: ["sessions", "list"],
    queryFn: listSessions,
    refetchInterval: REFRESH_MS
  });

  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "sessions-helper"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const switchMutation = useMutation({
    mutationFn: ({ agentId, sessionId }: { agentId: string; sessionId: string }) =>
      switchAgentSession(agentId, sessionId)
  });

  const deleteMutation = useMutation({
    mutationFn: (sessionId: string) => deleteSession(sessionId)
  });

  const sessions = sessionsQuery.data ?? [];
  const agents = agentsQuery.data ?? [];

  async function handleSwitch(agentId: string, sessionId: string) {
    setPendingId(sessionId);
    try {
      await switchMutation.mutateAsync({ agentId, sessionId });
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
    } finally {
      setPendingId(null);
    }
  }

  async function handleDelete(sessionId: string) {
    if (!window.confirm("Close this session? All transient context will be lost.")) return;
    setPendingId(sessionId);
    try {
      await deleteMutation.mutateAsync(sessionId);
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
    } finally {
      setPendingId(null);
    }
  }

  const getAgentName = (id: string) => agents.find(a => a.id === id)?.name || id;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" />
            </svg>
            Session Manager
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Sessions</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Active agent conversations and historical execution contexts.</p>
        </div>
        <button
          className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm"
          onClick={() => void sessionsQuery.refetch()}
        >
          <svg className={`h-3.5 w-3.5 ${sessionsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
            <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
          Refresh
        </button>
      </header>

      <div className="grid gap-4">
        {sessions.map((s: SessionItem) => (
          <article key={s.session_id} className="group rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm transition-all hover:border-brand/30 ring-1 ring-black/5 dark:ring-white/5">
            <div className="flex items-center justify-between gap-4">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-3 mb-1">
                  <h2 className="text-sm font-black truncate">{s.session_id.slice(0, 12)}...</h2>
                  <span className={`px-2 py-0.5 rounded-lg border text-[9px] font-black uppercase tracking-widest ${s.active ? 'border-success/20 bg-success/10 text-success' : 'border-border-subtle bg-main text-text-dim'}`}>
                    {s.active ? 'Active' : 'Idle'}
                  </span>
                </div>
                <div className="flex flex-wrap gap-4 mt-2">
                  <div className="flex items-center gap-1.5 text-[10px] font-bold text-text-dim uppercase tracking-wider">
                    <div className="h-4 w-4 rounded bg-brand/10 flex items-center justify-center text-brand">A</div>
                    Agent: <span className="text-slate-700 dark:text-slate-200">{getAgentName(s.agent_id)}</span>
                  </div>
                  <div className="flex items-center gap-1.5 text-[10px] font-bold text-text-dim uppercase tracking-wider">
                    <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path d="M12 8v4l3 2" /></svg>
                    Last Activity: <span className="text-slate-700 dark:text-slate-200">{new Date(s.updated_at || "").toLocaleTimeString()}</span>
                  </div>
                </div>
              </div>
              
              <div className="flex gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                {!s.active && (
                  <button
                    onClick={() => void handleSwitch(s.agent_id, s.session_id)}
                    disabled={pendingId === s.session_id}
                    className="px-4 py-1.5 rounded-lg border border-brand/20 bg-brand/5 text-brand text-[10px] font-black uppercase hover:bg-brand/10 transition-all disabled:opacity-50"
                  >
                    Resume
                  </button>
                )}
                <button
                  onClick={() => void handleDelete(s.session_id)}
                  disabled={pendingId === s.session_id}
                  className="px-4 py-1.5 rounded-lg border border-error/20 bg-error/5 text-error text-[10px] font-black uppercase hover:bg-error/10 transition-all disabled:opacity-50"
                >
                  Close
                </button>
              </div>
            </div>
          </article>
        ))}

        {sessions.length === 0 && !sessionsQuery.isLoading && (
          <div className="py-24 text-center border border-dashed border-border-subtle rounded-3xl bg-surface/30">
            <p className="text-sm text-text-dim font-black tracking-tight">No active sessions found.</p>
          </div>
        )}
      </div>
    </div>
  );
}
