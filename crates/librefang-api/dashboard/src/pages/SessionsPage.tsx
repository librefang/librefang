import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { formatRelativeTime } from "../lib/datetime";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { deleteSession, listAgents, listSessions, switchAgentSession, setSessionLabel } from "../api";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { useUIStore } from "../lib/store";
import { Clock, Search, MessageCircle, Trash2, Play, Users, Tag, Check, X } from "lucide-react";
import { truncateId } from "../lib/string";

const REFRESH_MS = 30000;

export function SessionsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [editingLabelId, setEditingLabelId] = useState<string | null>(null);
  const [labelValue, setLabelValue] = useState("");
  const addToast = useUIStore((s) => s.addToast);

  const sessionsQuery = useQuery({ queryKey: ["sessions", "list"], queryFn: listSessions, refetchInterval: REFRESH_MS });
  const agentsQuery = useQuery({ queryKey: ["agents", "list", "sessions"], queryFn: listAgents });

  const switchMutation = useMutation({ mutationFn: ({ agentId, sessionId }: any) => switchAgentSession(agentId, sessionId) });
  const deleteMutation = useMutation({ mutationFn: (id: string) => deleteSession(id) });
  const labelMutation = useMutation({
    mutationFn: ({ sessionId, label }: { sessionId: string; label: string }) => setSessionLabel(sessionId, label),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
      setEditingLabelId(null);
    },
  });

  const agents = agentsQuery.data ?? [];
  const agentMap = useMemo(() => new Map(agents.map(a => [a.id, a])), [agents]);

  const sessions = useMemo(() => {
    const list = sessionsQuery.data ?? [];
    return list
      .filter(s => {
        // Hide sessions for agents that no longer exist
        if (!agentMap.has(s.agent_id || "")) return false;
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

  const activeCount = sessions.filter(s => s.active).length;

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
    return formatRelativeTime(ts);
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.sessions")}
        title={t("sessions.title")}
        subtitle={t("sessions.subtitle")}
        isFetching={sessionsQuery.isFetching}
        onRefresh={() => void sessionsQuery.refetch()}
        icon={<Clock className="h-4 w-4" />}
        helpText={t("sessions.help")}
        actions={
          <div className="flex items-center gap-3">
            <Badge variant="brand">{activeCount} {t("sessions.active_count")}</Badge>
            <Badge variant="default">{sessions.length} {t("sessions.total")}</Badge>
          </div>
        }
      />

      {/* Search */}
      {sessions.length > 0 && (
        <Input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("sessions.search_placeholder")}
          leftIcon={<Search className="h-4 w-4" />}
        />
      )}

      {/* Sessions */}
      {sessionsQuery.isLoading ? (
        <ListSkeleton rows={3} />
      ) : sessions.length === 0 ? (
        <EmptyState
          icon={<MessageCircle className="w-7 h-7" />}
          title={t("sessions.empty_title")}
          description={t("sessions.empty_desc")}
        />
      ) : (
        <div className="space-y-2 stagger-children">
          {sessions.map(s => {
            const agent = agentMap.get(s.agent_id || "");
            return (
              <div key={s.session_id}
                className={`flex items-center gap-3 p-3 sm:p-4 rounded-xl sm:rounded-2xl border transition-all duration-300 card-glow cursor-pointer ${
                  s.active ? "border-success/30 bg-success/5" : "border-border-subtle hover:border-brand/30 hover:-translate-y-0.5"
                }`}>
                {/* Agent avatar */}
                <div className={`relative w-9 h-9 sm:w-10 sm:h-10 rounded-lg sm:rounded-xl flex items-center justify-center text-base sm:text-lg font-bold shrink-0 ${
                  s.active ? "bg-success/20 text-success" : "bg-main text-text-dim/40"
                }`}>
                  {agent?.name?.charAt(0).toUpperCase() || <Users className="w-4 h-4 sm:w-5 sm:h-5" />}
                  {s.active && <span className="absolute -bottom-0.5 -right-0.5 w-2 h-2 sm:w-2.5 sm:h-2.5 rounded-full bg-success border-2 border-white dark:border-surface animate-pulse" />}
                </div>

                {/* Info */}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 sm:gap-2">
                    <h3 className="text-xs sm:text-sm font-bold truncate">{agent?.name || t("sessions.unknown_agent")}</h3>
                    <Badge variant={s.active ? "success" : "default"}>
                      {s.active ? t("common.active") : t("common.idle")}
                    </Badge>
                  </div>
                  <div className="flex items-center gap-2 sm:gap-3 mt-0.5 sm:mt-1 text-[9px] sm:text-[10px] text-text-dim/60 flex-wrap">
                    <span className="font-mono">{truncateId(s.session_id)}</span>
                    <span className="flex items-center gap-1"><Clock className="w-3 h-3" /> {formatTime(s.created_at || "")}</span>
                    {s.message_count !== undefined && (
                      <span className="flex items-center gap-1 hidden sm:flex"><MessageCircle className="w-3 h-3" /> {s.message_count}</span>
                    )}
                    {editingLabelId === s.session_id ? (
                      <span className="flex items-center gap-1" onClick={e => e.stopPropagation()}>
                        <input
                          autoFocus
                          value={labelValue}
                          onChange={e => setLabelValue(e.target.value)}
                          onKeyDown={e => { if (e.key === "Enter") labelMutation.mutate({ sessionId: s.session_id, label: labelValue }); if (e.key === "Escape") setEditingLabelId(null); }}
                          className="px-1.5 py-0.5 rounded border border-brand bg-main text-[10px] w-24 outline-none"
                          placeholder={t("sessions.label_placeholder", { defaultValue: "Label..." })}
                        />
                        <button onClick={() => labelMutation.mutate({ sessionId: s.session_id, label: labelValue })} className="text-success"><Check className="w-3 h-3" /></button>
                        <button onClick={() => setEditingLabelId(null)} className="text-text-dim"><X className="w-3 h-3" /></button>
                      </span>
                    ) : (
                      <button
                        onClick={e => { e.stopPropagation(); setEditingLabelId(s.session_id); setLabelValue(s.label || ""); }}
                        className="flex items-center gap-0.5 hover:text-brand transition-colors"
                        title={t("sessions.set_label", { defaultValue: "Set label" })}
                      >
                        <Tag className="w-3 h-3" />
                        {s.label ? <span className="text-brand font-bold">{s.label}</span> : <span className="italic">{t("sessions.no_label", { defaultValue: "add label" })}</span>}
                      </button>
                    )}
                  </div>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-1 shrink-0">
                  {!s.active && s.agent_id && (
                    <Button variant="secondary" size="sm" onClick={() => handleSwitch(s.agent_id!, s.session_id)} disabled={pendingId === s.session_id}>
                      <Play className="w-3.5 h-3.5" />
                    </Button>
                  )}
                  {confirmDeleteId === s.session_id ? (
                    <div className="flex items-center gap-1">
                      <button onClick={() => handleDelete(s.session_id)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                      <button onClick={() => setConfirmDeleteId(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                    </div>
                  ) : (
                    <button onClick={() => handleDelete(s.session_id)} disabled={pendingId === s.session_id}
                      className="p-1.5 sm:p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-colors">
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
