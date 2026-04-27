import { formatTime } from "../lib/datetime";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { sendA2ATask, getA2ATaskStatus } from "../lib/http/client";
import type { A2AAgentItem, A2ATaskStatus } from "../lib/http/client";
import { useA2AAgents } from "../lib/queries/network";
import { useDiscoverA2AAgent } from "../lib/mutations/network";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Modal } from "../components/ui/Modal";
import { Badge } from "../components/ui/Badge";
import { EmptyState } from "../components/ui/EmptyState";
import { CardSkeleton } from "../components/ui/Skeleton";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { Globe, Search, Send, ExternalLink, Clock, CheckCircle2, XCircle, Loader2, Plus } from "lucide-react";

export function A2APage() {
  const { t } = useTranslation();

  const [discoverUrl, setDiscoverUrl] = useState("");
  const [showDiscover, setShowDiscover] = useState(false);
  useCreateShortcut(() => setShowDiscover(true));

  // Send task state
  const [taskAgent, setTaskAgent] = useState<A2AAgentItem | null>(null);
  const [taskMessage, setTaskMessage] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [trackedTasks, setTrackedTasks] = useState<A2ATaskStatus[]>([]);

  const agentsQuery = useA2AAgents();
  const discoverMutation = useDiscoverA2AAgent();

  const agents = agentsQuery.data ?? [];

  async function handleDiscover() {
    if (!discoverUrl.trim()) return;
    try {
      await discoverMutation.mutateAsync(discoverUrl.trim());
      setDiscoverUrl("");
      setShowDiscover(false);
    } catch {
      // error handled by UI
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
      const taskId = result.task_id as string | undefined;
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
        helpText={t("a2a.help")}
        actions={
          <button
            onClick={() => setShowDiscover((v) => !v)}
            className="flex h-9 items-center gap-2 rounded-xl border border-primary/30 bg-primary/10 px-4 text-sm font-bold text-primary hover:bg-primary/20 transition-colors"
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
          <div className="flex flex-col sm:flex-row gap-3">
            <input
              type="url"
              value={discoverUrl}
              onChange={(e) => setDiscoverUrl(e.target.value)}
              placeholder={t("a2a.discover_placeholder")}
              className="flex-1 rounded-xl border border-border-subtle bg-main px-4 py-2.5 text-sm focus:border-primary focus:ring-2 focus:ring-primary/10 outline-none"
              onKeyDown={(e) => e.key === "Enter" && handleDiscover()}
            />
            <button
              onClick={handleDiscover}
              disabled={discoverMutation.isPending || !discoverUrl.trim()}
              className="flex items-center gap-2 rounded-xl bg-primary px-5 py-2.5 text-sm font-bold text-white hover:bg-primary/90 disabled:opacity-40 transition-colors"
            >
              {discoverMutation.isPending ? (
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
                    className="flex items-center gap-2 rounded-xl bg-primary px-5 py-2.5 text-sm font-bold text-white hover:bg-primary/90 transition-colors"
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
                        <div className="w-10 h-10 rounded-xl bg-linear-to-br from-accent/20 to-brand/20 flex items-center justify-center">
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
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold text-primary hover:bg-primary/10 transition-colors"
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

          {/* Send task panel */}
          {taskAgent && (
            <Modal
              isOpen
              onClose={() => setTaskAgent(null)}
              variant="panel-right"
              size="lg"
              title={t("a2a.send_task")}
              zIndex={200}
            >
              <div className="p-5 space-y-4">
                <p className="text-xs text-text-dim">
                  {t("a2a.send_to")} <span className="font-bold text-primary">{taskAgent.name || taskAgent.url}</span>
                </p>
                <textarea
                  value={taskMessage}
                  onChange={(e) => setTaskMessage(e.target.value)}
                  placeholder={t("a2a.task_placeholder")}
                  rows={4}
                  className="w-full rounded-xl border border-border-subtle bg-main px-4 py-3 text-sm focus:border-primary focus:ring-2 focus:ring-primary/10 outline-none resize-none"
                />
                <div className="flex justify-end gap-3">
                  <button
                    onClick={() => setTaskAgent(null)}
                    className="px-4 py-2 rounded-xl text-sm font-bold text-text-dim hover:bg-surface-hover transition-colors"
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    onClick={handleSendTask}
                    disabled={isSending || !taskMessage.trim()}
                    className="flex items-center gap-2 rounded-xl bg-primary px-5 py-2 text-sm font-bold text-white hover:bg-primary/90 disabled:opacity-40 transition-colors"
                  >
                    {isSending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
                    {t("a2a.send")}
                  </button>
                </div>
              </div>
            </Modal>
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
                          <Loader2 className="w-4 h-4 text-primary animate-spin" />
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
                          className="text-xs font-bold text-primary hover:text-primary/80 transition-colors"
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
