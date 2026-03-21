import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { deleteSession, listAgents, listSessions, switchAgentSession } from "../api";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { Clock, RefreshCw, Search, MessageCircle, Trash2, Play, Users } from "lucide-react";

const REFRESH_MS = 30000;

export function SessionsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  const sessionsQuery = useQuery({ queryKey: ["sessions", "list"], queryFn: listSessions, refetchInterval: REFRESH_MS });
  const agentsQuery = useQuery({ queryKey: ["agents", "list", "sessions"], queryFn: listAgents });

  const switchMutation = useMutation({ mutationFn: ({ agentId, sessionId }: any) => switchAgentSession(agentId, sessionId) });
  const deleteMutation = useMutation({ mutationFn: (id: string) => deleteSession(id) });

  const agents = agentsQuery.data ?? [];
  const agentMap = useMemo(() => new Map(agents.map(a => [a.id, a])), [agents]);

  const sessions = useMemo(() => {
    const list = sessionsQuery.data ?? [];
    return list
      .filter(s => {
        if (!search) return true;
        const agent = agentMap.get(s.agent_id || "");
        return (agent?.name || "").toLowerCase().includes(search.toLowerCase()) || s.session_id.includes(search);
      })
      .sort((a, b) => {
        // Active first
        if ((a as any).active && !(b as any).active) return -1;
        if (!(a as any).active && (b as any).active) return 1;
        return (b.created_at || "").localeCompare(a.created_at || "");
      });
  }, [sessionsQuery.data, search, agentMap]);

  const activeCount = sessions.filter(s => (s as any).active).length;

  async function handleSwitch(agentId: string, sessionId: string) {
    setPendingId(sessionId);
    try {
      await switchMutation.mutateAsync({ agentId, sessionId });
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally { setPendingId(null); }
  }

  async function handleDelete(id: string) {
    if (confirmDeleteId !== id) { setConfirmDeleteId(id); return; }
    setConfirmDeleteId(null);
    setPendingId(id);
    try {
      await deleteMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally { setPendingId(null); }
  }

  const formatTime = (ts: string) => {
    if (!ts) return "-";
    const d = new Date(ts);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    if (diff < 60000) return t("sessions.just_now");
    if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`;
    if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`;
    return d.toLocaleDateString();
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      {/* Header */}
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <Clock className="h-4 w-4" />
            {t("nav.sessions")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("sessions.title")}</h1>
          <p className="mt-1 text-text-dim font-medium text-sm">{t("sessions.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          <Badge variant="brand">{activeCount} {t("sessions.active_count")}</Badge>
          <Badge variant="default">{sessions.length} {t("sessions.total")}</Badge>
          <Button variant="secondary" onClick={() => sessionsQuery.refetch()}>
            <RefreshCw className={`h-3.5 w-3.5 ${sessionsQuery.isFetching ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </header>

      {/* Search */}
      {sessions.length > 0 && (
        <div className="relative">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-text-dim/40" />
          <input type="text" value={search} onChange={e => setSearch(e.target.value)}
            placeholder={t("sessions.search_placeholder")}
            className="w-full pl-10 pr-4 py-2.5 rounded-xl border border-border-subtle bg-surface text-sm outline-none focus:border-brand" />
        </div>
      )}

      {/* Sessions */}
      {sessionsQuery.isLoading ? (
        <div className="space-y-3">
          {[1, 2, 3].map(i => (
            <div key={i} className="flex items-center gap-4 p-4 rounded-2xl border border-border-subtle animate-pulse">
              <div className="w-10 h-10 rounded-xl bg-main" />
              <div className="flex-1 space-y-2"><div className="h-4 w-40 bg-main rounded" /><div className="h-3 w-60 bg-main rounded" /></div>
            </div>
          ))}
        </div>
      ) : sessions.length === 0 ? (
        <div className="flex flex-col items-center py-20">
          <div className="relative mb-6">
            <div className="w-20 h-20 rounded-3xl bg-brand/10 flex items-center justify-center">
              <MessageCircle className="w-10 h-10 text-brand" />
            </div>
            <span className="absolute inset-0 rounded-3xl bg-brand/5 animate-ping" style={{ animationDuration: "3s" }} />
          </div>
          <h3 className="text-xl font-black tracking-tight">{t("sessions.empty_title")}</h3>
          <p className="text-sm text-text-dim mt-2 max-w-xs text-center">{t("sessions.empty_desc")}</p>
        </div>
      ) : (
        <div className="space-y-2">
          {sessions.map(s => {
            const agent = agentMap.get(s.agent_id || "");
            return (
              <div key={s.session_id}
                className={`flex items-center gap-4 p-4 rounded-2xl border transition-all ${
                  (s as any).active ? "border-success/30 bg-success/5" : "border-border-subtle hover:border-brand/30"
                }`}>
                {/* Agent avatar */}
                <div className={`relative w-10 h-10 rounded-xl flex items-center justify-center text-lg font-bold shrink-0 ${
                  (s as any).active ? "bg-success/20 text-success" : "bg-main text-text-dim/40"
                }`}>
                  {agent?.name?.charAt(0).toUpperCase() || <Users className="w-5 h-5" />}
                  {(s as any).active && <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-success border-2 border-white dark:border-surface animate-pulse" />}
                </div>

                {/* Info */}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-bold truncate">{agent?.name || t("sessions.unknown_agent")}</h3>
                    <Badge variant={(s as any).active ? "success" : "default"}>
                      {(s as any).active ? t("common.active") : t("common.idle")}
                    </Badge>
                  </div>
                  <div className="flex items-center gap-3 mt-1 text-[10px] text-text-dim/60">
                    <span className="font-mono">{s.session_id.slice(0, 8)}</span>
                    <span className="flex items-center gap-1"><Clock className="w-3 h-3" /> {formatTime(s.created_at || "")}</span>
                    {s.message_count !== undefined && (
                      <span className="flex items-center gap-1"><MessageCircle className="w-3 h-3" /> {s.message_count}</span>
                    )}
                  </div>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-1 shrink-0">
                  {!(s as any).active && s.agent_id && (
                    <Button variant="secondary" size="sm" onClick={() => handleSwitch(s.agent_id!, s.session_id)} disabled={pendingId === s.session_id}>
                      <Play className="w-3.5 h-3.5 mr-1" /> {t("common.resume")}
                    </Button>
                  )}
                  {confirmDeleteId === s.session_id ? (
                    <div className="flex items-center gap-1">
                      <button onClick={() => handleDelete(s.session_id)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                      <button onClick={() => setConfirmDeleteId(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                    </div>
                  ) : (
                    <button onClick={() => handleDelete(s.session_id)} disabled={pendingId === s.session_id}
                      className="p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-all">
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
  );
}
