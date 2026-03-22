import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot, getVersionInfo, getQueueStatus } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Activity, Cpu, HardDrive, Zap, Timer, Layers, CheckCircle2, GitCommit, Calendar } from "lucide-react";

const REFRESH_MS = 30000;

export function RuntimePage() {
  const { t } = useTranslation();
  const snapshotQuery = useQuery({ queryKey: ["dashboard", "snapshot", "runtime"], queryFn: loadDashboardSnapshot, refetchInterval: REFRESH_MS });
  const versionQuery = useQuery({ queryKey: ["version"], queryFn: getVersionInfo, refetchInterval: REFRESH_MS * 2 });
  const queueQuery = useQuery({ queryKey: ["queue", "status"], queryFn: getQueueStatus, refetchInterval: 5000 });

  const snapshot = snapshotQuery.data ?? null;
  const version = versionQuery.data ?? null;
  const queue = queueQuery.data ?? null;

  const uptimeSecs = snapshot?.status?.uptime_seconds || 0;
  const uptimeStr = uptimeSecs
    ? `${Math.floor(uptimeSecs / 3600)}h ${Math.floor((uptimeSecs % 3600) / 60)}m`
    : "-";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("runtime.kernel")}
        title={t("runtime.title")}
        subtitle={t("runtime.subtitle")}
        isFetching={snapshotQuery.isFetching}
        onRefresh={() => void snapshotQuery.refetch()}
        icon={<Activity className="h-4 w-4" />}
      />

      {snapshotQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-4"><CardSkeleton /><CardSkeleton /><CardSkeleton /><CardSkeleton /></div>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-2 sm:gap-4 xl:grid-cols-4 stagger-children">
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("runtime.system_uptime")}</span>
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center"><Timer className="w-4 h-4 text-success" /></div>
              </div>
              <p className="text-3xl font-black tracking-tight mt-2">{uptimeStr}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("runtime.active_agents")}</span>
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Layers className="w-4 h-4 text-brand" /></div>
              </div>
              <p className="text-3xl font-black tracking-tight mt-2 text-brand">{snapshot?.status?.agent_count || 0}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("runtime.pending_tasks")}</span>
                <div className="w-8 h-8 rounded-lg bg-warning/10 flex items-center justify-center"><Zap className="w-4 h-4 text-warning" /></div>
              </div>
              <p className="text-3xl font-black tracking-tight mt-2">{(queue as any)?.pending ?? 0}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("runtime.status")}</span>
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center"><CheckCircle2 className="w-4 h-4 text-success" /></div>
              </div>
              <div className="mt-2 flex items-center gap-2">
                <span className="relative flex h-2.5 w-2.5"><span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-ping" /><span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-success" /></span>
                <Badge variant="success">{t("status.nominal")}</Badge>
              </div>
            </Card>
          </div>

          <div className="grid gap-3 sm:gap-6 md:grid-cols-2 lg:grid-cols-3 stagger-children">
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Cpu className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.engine")}</h2>
              </div>
              <div className="space-y-4">
                <div className="flex items-center gap-3">
                  <Activity className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
                  <span className="text-xs text-text-dim flex-1">{t("runtime.engine_version")}</span>
                  <span className="font-mono text-sm font-black text-brand">{version?.version || snapshot?.status?.version || t("common.unknown")}</span>
                </div>
                <div className="flex items-center gap-3">
                  <GitCommit className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
                  <span className="text-xs text-text-dim flex-1">{t("runtime.git_hash")}</span>
                  <span className="font-mono text-xs text-text-dim truncate max-w-[140px]">{version?.git_sha || "-"}</span>
                </div>
                <div className="flex items-center gap-3">
                  <Calendar className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
                  <span className="text-xs text-text-dim flex-1">{t("runtime.build_time")}</span>
                  <span className="text-xs text-text-dim">{version?.build_date || "-"}</span>
                </div>
              </div>
            </Card>

            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center"><HardDrive className="h-4 w-4 text-success" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.system")}</h2>
              </div>
              <div className="space-y-4">
                <div className="flex items-center gap-3">
                  <Timer className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
                  <span className="text-xs text-text-dim flex-1">{t("runtime.system_uptime")}</span>
                  <span className="text-sm font-black">{uptimeStr}</span>
                </div>
                <div className="flex items-center gap-3">
                  <Layers className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
                  <span className="text-xs text-text-dim flex-1">{t("runtime.active_agents")}</span>
                  <span className="text-sm font-black text-success">{snapshot?.status?.agent_count || 0}</span>
                </div>
                <div className="flex items-center gap-3">
                  <CheckCircle2 className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
                  <span className="text-xs text-text-dim flex-1">{t("runtime.status")}</span>
                  <Badge variant="success">{t("status.nominal")}</Badge>
                </div>
              </div>
            </Card>

            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-warning/10 flex items-center justify-center"><Zap className="h-4 w-4 text-warning" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.queue")}</h2>
              </div>
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <span className="text-xs text-text-dim">{t("runtime.pending_tasks")}</span>
                  <div className="flex items-center gap-2">
                    <div className="h-1.5 w-16 rounded-full bg-main overflow-hidden">
                      <div className="h-full rounded-full bg-warning" style={{ width: `${Math.min(((queue as any)?.pending ?? 0) * 10, 100)}%` }} />
                    </div>
                    <span className="text-sm font-black w-8 text-right">{(queue as any)?.pending ?? 0}</span>
                  </div>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-xs text-text-dim">{t("runtime.running_tasks")}</span>
                  <div className="flex items-center gap-2">
                    <div className="h-1.5 w-16 rounded-full bg-main overflow-hidden">
                      <div className="h-full rounded-full bg-brand" style={{ width: `${Math.min(((queue as any)?.running ?? 0) * 10, 100)}%` }} />
                    </div>
                    <span className="text-sm font-black text-brand w-8 text-right">{(queue as any)?.running ?? 0}</span>
                  </div>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-xs text-text-dim">{t("runtime.completed_today")}</span>
                  <div className="flex items-center gap-2">
                    <div className="h-1.5 w-16 rounded-full bg-main overflow-hidden">
                      <div className="h-full rounded-full bg-success" style={{ width: `${Math.min(((queue as any)?.completed_today ?? 0) * 5, 100)}%` }} />
                    </div>
                    <span className="text-sm font-black text-success w-8 text-right">{(queue as any)?.completed_today ?? 0}</span>
                  </div>
                </div>
              </div>
            </Card>
          </div>
        </>
      )}
    </div>
  );
}
