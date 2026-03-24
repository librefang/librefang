import { useQuery, useQueryClient } from "@tanstack/react-query";
import { formatTime } from "../lib/datetime";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { listA2AAgents, discoverA2AAgent, sendA2ATask, getA2ATaskStatus } from "../api";
import type { A2AAgentItem, A2ATaskStatus } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { EmptyState } from "../components/ui/EmptyState";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Globe, Search, Send, ExternalLink, Clock, CheckCircle2, XCircle, Loader2, Plus } from "lucide-react";

const REFRESH_MS = 15000;

export function A2APage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [discoverUrl, setDiscoverUrl] = useState("");
  const [isDiscovering, setIsDiscovering] = useState(false);
  const [showDiscover, setShowDiscover] = useState(false);

  // Send task state
  const [taskAgent, setTaskAgent] = useState<A2AAgentItem | null>(null);
  const [taskMessage, setTaskMessage] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [trackedTasks, setTrackedTasks] = useState<A2ATaskStatus[]>([]);

  const agentsQuery = useQuery({
    queryKey: ["a2a", "agents"],
    queryFn: listA2AAgents,
    refetchInterval: REFRESH_MS,
  });

  const agents = agentsQuery.data ?? [];

  async function handleDiscover() {
    if (!discoverUrl.trim()) return;
    setIsDiscovering(true);
    try {
      await discoverA2AAgent(discoverUrl.trim());
      setDiscoverUrl("");
      setShowDiscover(false);
      void queryClient.invalidateQueries({ queryKey: ["a2a", "agents"] });
    } catch {
      // error handled by UI
    } finally {
      setIsDiscovering(false);
    }
  }

  async function handleSendTask() {
    if (!taskAgent?.url || !taskMessage.trim()) return;
    setIsSending(true);
    try {
      const result = await sendA2ATask({
        agent_url: taskAgent.url,
        message: taskMessage.trim(),
      });
      // Track the task if we get an ID back
      const taskId = (result as Record<string, unknown>).task_id as string | undefined;
      if (taskId) {
        setTrackedTasks((prev) => [
          { id: taskId, status: "pending", created_at: new Date().toISOString() },
          ...prev,
        ]);
      }
      setTaskMessage("");
      setTaskAgent(null);
    } catch {
      // error silenced
    } finally {
      setIsSending(false);
    }
  }

  async function refreshTaskStatus(taskId: string) {
    try {
      const status = await getA2ATaskStatus(taskId);
      setTrackedTasks((prev) =>
        prev.map((t) => (t.id === taskId ? { ...t, ...status } : t))
      );
    } catch {
      // ignore
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("a2a.section")}
        title={t("a2a.title")}
        subtitle={t("a2a.subtitle")}
        isFetching={agentsQuery.isFetching}
        onRefresh={() => void agentsQuery.refetch()}
        icon={<Globe className="h-4 w-4" />}
        actions={
          <button
            onClick={() => setShowDiscover((v) => !v)}
            className="flex h-9 items-center gap-2 rounded-xl border border-brand/30 bg-brand/10 px-4 text-sm font-bold text-brand hover:bg-brand/20 transition-all"
          >
            <Search className="h-3.5 w-3.5" />
            {t("a2a.discover")}
          </button>
        }
      />

      {/* Discover overlay */}
      {showDiscover && (
        <Card padding="md" className="animate-fade-in-scale">
          <h3 className="text-sm font-black mb-3">{t("a2a.discover_agent")}</h3>
          <div className="flex gap-3">
            <input
              type="url"
              value={discoverUrl}
              onChange={(e) => setDiscoverUrl(e.target.value)}
              placeholder={t("a2a.discover_placeholder")}
              className="flex-1 rounded-xl border border-border-subtle bg-main px-4 py-2.5 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none"
              onKeyDown={(e) => e.key === "Enter" && handleDiscover()}
            />
            <button
              onClick={handleDiscover}
              disabled={isDiscovering || !discoverUrl.trim()}
              className="flex items-center gap-2 rounded-xl bg-brand px-5 py-2.5 text-sm font-bold text-white hover:bg-brand/90 disabled:opacity-40 transition-all"
            >
              {isDiscovering ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Plus className="h-4 w-4" />
              )}
              {t("a2a.discover")}
            </button>
          </div>
        </Card>
      )}

      {agentsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-2">
          <CardSkeleton />
          <CardSkeleton />
        </div>
      ) : (
        <>
          {/* Agent list */}
          <div>
            <h2 className="text-lg font-black tracking-tight mb-4">
              {t("a2a.external_agents")}
              <Badge variant="brand" className="ml-3">{agents.length}</Badge>
            </h2>

            {agents.length === 0 ? (
              <EmptyState
                icon={<Globe className="h-8 w-8" />}
                title={t("a2a.no_agents")}
                description={t("a2a.no_agents_desc")}
                action={
                  <button
                    onClick={() => setShowDiscover(true)}
                    className="flex items-center gap-2 rounded-xl bg-brand px-5 py-2.5 text-sm font-bold text-white hover:bg-brand/90 transition-all"
                  >
                    <Search className="h-4 w-4" />
                    {t("a2a.discover")}
                  </button>
                }
              />
            ) : (
              <div className="grid gap-3 md:grid-cols-2 stagger-children">
                {agents.map((agent, idx) => (
                  <Card key={agent.url || idx} hover padding="md">
                    <div className="flex items-start justify-between">
                      <div className="flex items-center gap-3">
                        <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-accent/20 to-brand/20 flex items-center justify-center">
                          <ExternalLink className="w-5 h-5 text-accent" />
                        </div>
                        <div className="min-w-0">
                          <p className="text-sm font-bold truncate">{agent.name || agent.url || t("common.unknown")}</p>
                          {agent.url && (
                            <p className="text-[10px] text-text-dim font-mono truncate max-w-[240px]">{agent.url}</p>
                          )}
                        </div>
                      </div>
                      <Badge
                        variant={agent.status === "available" ? "success" : "warning"}
                        dot
                      >
                        {agent.status || t("common.unknown")}
                      </Badge>
                    </div>
                    {agent.description && (
                      <p className="text-xs text-text-dim mt-2 line-clamp-2">{agent.description}</p>
                    )}
                    {agent.skills && agent.skills.length > 0 && (
                      <div className="flex flex-wrap gap-1 mt-2">
                        {agent.skills.map((s) => (
                          <Badge key={s} variant="default">{s}</Badge>
                        ))}
                      </div>
                    )}
                    <div className="mt-3 flex justify-end">
                      <button
                        onClick={() => setTaskAgent(agent)}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold text-brand hover:bg-brand/10 transition-all"
                      >
                        <Send className="h-3 w-3" />
                        {t("a2a.send_task")}
                      </button>
                    </div>
                  </Card>
                ))}
              </div>
            )}
          </div>

          {/* Send task modal */}
          {taskAgent && (
            <div className="fixed inset-0 z-[200] flex items-end sm:items-center justify-center bg-black/60 backdrop-blur-sm" onClick={() => setTaskAgent(null)}>
              <div className="w-full sm:max-w-lg sm:mx-4 animate-fade-in-scale" onClick={(e) => e.stopPropagation()}>
                <Card padding="lg">
                  <h3 className="text-lg font-black mb-1">{t("a2a.send_task")}</h3>
                  <p className="text-xs text-text-dim mb-4">
                    {t("a2a.send_to")} <span className="font-bold text-brand">{taskAgent.name || taskAgent.url}</span>
                  </p>
                  <textarea
                    value={taskMessage}
                    onChange={(e) => setTaskMessage(e.target.value)}
                    placeholder={t("a2a.task_placeholder")}
                    rows={4}
                    className="w-full rounded-xl border border-border-subtle bg-main px-4 py-3 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none resize-none"
                  />
                  <div className="flex justify-end gap-3 mt-4">
                    <button
                      onClick={() => setTaskAgent(null)}
                      className="px-4 py-2 rounded-xl text-sm font-bold text-text-dim hover:bg-surface-hover transition-all"
                    >
                      {t("common.cancel")}
                    </button>
                    <button
                      onClick={handleSendTask}
                      disabled={isSending || !taskMessage.trim()}
                      className="flex items-center gap-2 rounded-xl bg-brand px-5 py-2 text-sm font-bold text-white hover:bg-brand/90 disabled:opacity-40 transition-all"
                    >
                      {isSending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
                      {t("a2a.send")}
                    </button>
                  </div>
                </Card>
              </div>
            </div>
          )}

          {/* Tracked tasks */}
          {trackedTasks.length > 0 && (
            <div>
              <h2 className="text-lg font-black tracking-tight mb-4">{t("a2a.tracked_tasks")}</h2>
              <div className="space-y-2 stagger-children">
                {trackedTasks.map((task) => (
                  <Card key={task.id} padding="sm">
                    <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-2">
                      <div className="flex items-center gap-2 sm:gap-3 min-w-0 flex-1 flex-wrap">
                        {task.status === "completed" ? (
                          <CheckCircle2 className="w-4 h-4 text-success" />
                        ) : task.status === "failed" ? (
                          <XCircle className="w-4 h-4 text-error" />
                        ) : (
                          <Loader2 className="w-4 h-4 text-brand animate-spin" />
                        )}
                        <span className="text-xs sm:text-sm font-mono font-bold truncate">{task.id}</span>
                        <Badge
                          variant={
                            task.status === "completed" ? "success" :
                            task.status === "failed" ? "error" : "brand"
                          }
                        >
                          {task.status}
                        </Badge>
                      </div>
                      <div className="flex items-center gap-2">
                        {task.created_at && (
                          <span className="text-[10px] text-text-dim flex items-center gap-1">
                            <Clock className="w-3 h-3" />
                            {formatTime(task.created_at)}
                          </span>
                        )}
                        <button
                          onClick={() => task.id && refreshTaskStatus(task.id)}
                          className="text-xs font-bold text-brand hover:text-brand/80 transition-colors"
                        >
                          {t("common.refresh")}
                        </button>
                      </div>
                    </div>
                    {task.result && (
                      <p className="text-xs text-text-dim mt-2 bg-main rounded-lg p-2 font-mono">{task.result}</p>
                    )}
                    {task.error && (
                      <p className="text-xs text-error mt-2">{task.error}</p>
                    )}
                  </Card>
                ))}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
