import { formatRelativeTime } from "../lib/datetime";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertCircle, RefreshCw } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { useAgents } from "../lib/queries/agents";
import { useSessions } from "../lib/queries/sessions";
import { useDeleteAgentSession } from "../lib/mutations/agents";
import { useSetSessionLabel } from "../lib/mutations/sessions";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { Clock, Search, MessageCircle, Trash2, Users, Tag, Check, X } from "lucide-react";
import { truncateId } from "../lib/string";
import { StaggerList } from "../components/ui/StaggerList";

export function SessionsPage() {
  const { t, i18n } = useTranslation();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [editingLabelId, setEditingLabelId] = useState<string | null>(null);
  const [labelValue, setLabelValue] = useState("");
  const addToast = useUIStore((s) => s.addToast);

  const sessionsQuery = useSessions();
  const agentsQuery = useAgents();

  const deleteMutation = useDeleteAgentSession();
  const navigate = useNavigate();
  const labelMutation = useSetSessionLabel();

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
        if (a.active && !b.active) return -1;
        if (!a.active && b.active) return 1;
        return (b.created_at || "").localeCompare(a.created_at || "");
      });
  }, [sessionsQuery.data, search, agentMap]);

  const activeCount = sessions.filter(s => s.active).length;

  function saveLabel(sessionId: string, label: string, agentId?: string) {
    labelMutation.mutate({ sessionId, label, agentId }, { onSuccess: () => setEditingLabelId(null) });
  }

  // Open this session in the chat page, pinning `?sessionId=` so the chat
  // tab routes its WS + send traffic to that exact session (#2959). This is
  // what users expect when they click ▶ on a session row — the previous
  // behavior (registry-pointer swap via `switch_agent_session`) did not
  // affect any chat tab pinned to its own session id, and was misleading
  // (#4292). The "make this the agent's default session" action is left to
  // a future explicit affordance instead of overloading Play.
  function handleOpenInChat(agentId: string, sessionId: string) {
    navigate({ to: "/chat", search: { agentId, sessionId } });
  }

  async function handleDelete(sessionId: string, agentId?: string) {
    if (confirmDeleteId !== sessionId) { setConfirmDeleteId(sessionId); return; }
    setConfirmDeleteId(null);
    setPendingId(sessionId);
    try {
      await deleteMutation.mutateAsync({ sessionId, agentId });
    } catch (e) {
      addToast(toastErr(e, t("common.error")), "error");
    } finally { setPendingId(null); }
  }

  const nowMs = Date.now();
  const locale = i18n.language ?? "en";
  const formatTime = (ts: string) => {
    if (!ts) return "-";
    const diff = nowMs - new Date(ts).getTime();
    if (diff < 60000) return t("sessions.just_now");
    return formatRelativeTime(ts, locale, nowMs);
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
          data-shortcut-search
        />
      )}

      {agentsQuery.isError && !agentsQuery.isLoading && (
        <div className="flex items-center gap-2 px-3 py-2 rounded-xl border border-warning/30 bg-warning/5 text-warning text-xs">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{t("sessions.agents_load_warning", { defaultValue: "Could not load agent names — session list may show unknown agents." })}</span>
        </div>
      )}

      {/* Sessions */}
      {sessionsQuery.isLoading ? (
        <ListSkeleton rows={3} />
      ) : sessionsQuery.isError ? (
        <div className="flex flex-col items-center justify-center gap-3 py-12 text-text-dim">
          <AlertCircle className="w-8 h-8 text-error" />
          <p className="text-sm">{t("sessions.load_error", { defaultValue: "Failed to load sessions." })}</p>
          <p className="text-xs text-text-dim/60">{String(sessionsQuery.error)}</p>
          <Button variant="secondary" size="sm" onClick={() => sessionsQuery.refetch()}>
            <RefreshCw className="w-3.5 h-3.5" /> {t("common.retry", { defaultValue: "Retry" })}
          </Button>
        </div>
      ) : sessions.length === 0 ? (
        <EmptyState
          icon={<MessageCircle className="w-7 h-7" />}
          title={t("sessions.empty_title")}
          description={t("sessions.empty_desc")}
        />
      ) : (
        <StaggerList className="space-y-2">
          {sessions.map(s => {
            const agent = agentMap.get(s.agent_id || "");
            return (
              <div key={s.session_id}
                className={`flex items-center gap-3 p-3 sm:p-4 rounded-xl sm:rounded-2xl border transition-all duration-300 card-glow ${
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
                          onKeyDown={e => { if (e.key === "Enter") saveLabel(s.session_id, labelValue, s.agent_id ?? undefined); if (e.key === "Escape") setEditingLabelId(null); }}
                          className="px-1.5 py-0.5 rounded border border-brand bg-main text-[10px] w-24 outline-none"
                          placeholder={t("sessions.label_placeholder", { defaultValue: "Label..." })}
                        />
                        <button onClick={() => saveLabel(s.session_id, labelValue, s.agent_id ?? undefined)} className="text-success"><Check className="w-3 h-3" /></button>
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
                  {s.agent_id && (
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => handleOpenInChat(s.agent_id!, s.session_id)}
                      disabled={pendingId === s.session_id}
                      title={t("sessions.open_in_chat", { defaultValue: "Open this session in the chat page" })}
                      aria-label={t("sessions.open_in_chat", { defaultValue: "Open this session in the chat page" })}
                    >
                      <MessageCircle className="w-3.5 h-3.5" />
                    </Button>
                  )}
                  {confirmDeleteId === s.session_id ? (
                    <div className="flex items-center gap-1">
                      <button onClick={() => handleDelete(s.session_id, s.agent_id ?? undefined)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                      <button onClick={() => setConfirmDeleteId(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                    </div>
                  ) : (
                    <button onClick={() => handleDelete(s.session_id, s.agent_id ?? undefined)} disabled={pendingId === s.session_id}
                      className="p-1.5 sm:p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-colors">
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </StaggerList>
      )}
    </div>
  );
}
