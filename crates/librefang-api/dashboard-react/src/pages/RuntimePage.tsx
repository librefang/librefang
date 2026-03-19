import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Activity } from "lucide-react";

const REFRESH_MS = 30000;

export function RuntimePage() {
  const { t } = useTranslation();
  const snapshotQuery = useQuery({ queryKey: ["dashboard", "snapshot", "runtime"], queryFn: loadDashboardSnapshot, refetchInterval: REFRESH_MS });
  const snapshot = snapshotQuery.data ?? null;

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
        <div className="grid gap-6 md:grid-cols-2">
          <Card padding="lg">
            <h2 className="text-lg font-black tracking-tight mb-6">{t("runtime.environment")}</h2>
            <div className="space-y-4">
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.engine_version")}</span>
                <span className="font-mono text-sm font-black text-brand">{snapshot?.status.version || t("common.unknown")}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.system_uptime")}</span>
                <span className="text-sm font-black">{snapshot?.status.uptime_seconds || "-"}s</span>
              </div>
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
