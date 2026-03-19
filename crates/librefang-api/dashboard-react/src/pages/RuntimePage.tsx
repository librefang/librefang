import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot, getVersionInfo, getQueueStatus } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Activity, Cpu, HardDrive, Zap, Clock } from "lucide-react";

const REFRESH_MS = 30000;

export function RuntimePage() {
  const { t } = useTranslation();
  const snapshotQuery = useQuery({ queryKey: ["dashboard", "snapshot", "runtime"], queryFn: loadDashboardSnapshot, refetchInterval: REFRESH_MS });
  const versionQuery = useQuery({ queryKey: ["version"], queryFn: getVersionInfo, refetchInterval: REFRESH_MS * 2 });
  const queueQuery = useQuery({ queryKey: ["queue", "status"], queryFn: getQueueStatus, refetchInterval: 5000 });

  const snapshot = snapshotQuery.data ?? null;
  const version = versionQuery.data ?? null;
  const queue = queueQuery.data ?? null;

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
        <div className="grid gap-6 md:grid-cols-2">
          <CardSkeleton />
        </div>
      ) : (
        <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
          {/* 版本信息 */}
          <Card padding="lg">
            <div className="flex items-center gap-2 mb-4">
              <Cpu className="h-4 w-4 text-brand" />
              <h2 className="text-lg font-black tracking-tight">{t("runtime.engine")}</h2>
            </div>
            <div className="space-y-3">
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.engine_version")}</span>
                <span className="font-mono text-sm font-black text-brand">{version?.version || snapshot?.status.version || t("common.unknown")}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.git_hash")}</span>
                <span className="font-mono text-xs text-text-dim truncate max-w-[120px]">{version?.git_hash || "-"}</span>
              </div>
              <div className="flex justify-between items-center py-2">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.build_time")}</span>
                <span className="text-xs text-text-dim">{version?.build_time || "-"}</span>
              </div>
            </div>
          </Card>

          {/* 系统状态 */}
          <Card padding="lg">
            <div className="flex items-center gap-2 mb-4">
              <HardDrive className="h-4 w-4 text-success" />
              <h2 className="text-lg font-black tracking-tight">{t("runtime.system")}</h2>
            </div>
            <div className="space-y-3">
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.system_uptime")}</span>
                <span className="text-sm font-black">{snapshot?.status.uptime_seconds ? `${Math.floor(snapshot.status.uptime_seconds / 3600)}h ${Math.floor((snapshot.status.uptime_seconds % 3600) / 60)}m` : "-"}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.active_agents")}</span>
                <span className="text-sm font-black text-success">{snapshot?.status.agent_count || 0}</span>
              </div>
              <div className="flex justify-between items-center py-2">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.status")}</span>
                <span className="px-2 py-0.5 rounded-full text-[10px] font-black uppercase bg-success/10 text-success">{t("status.nominal")}</span>
              </div>
            </div>
          </Card>

          {/* 队列状态 */}
          <Card padding="lg">
            <div className="flex items-center gap-2 mb-4">
              <Zap className="h-4 w-4 text-warning" />
              <h2 className="text-lg font-black tracking-tight">{t("runtime.queue")}</h2>
            </div>
            <div className="space-y-3">
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.pending_tasks")}</span>
                <span className="text-sm font-black">{queue?.pending ?? 0}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.running_tasks")}</span>
                <span className="text-sm font-black text-brand">{queue?.running ?? 0}</span>
              </div>
              <div className="flex justify-between items-center py-2">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.completed_today")}</span>
                <span className="text-sm font-black text-success">{queue?.completed_today ?? 0}</span>
              </div>
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
