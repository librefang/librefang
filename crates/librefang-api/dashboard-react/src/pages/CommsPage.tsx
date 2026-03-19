import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listChannels, loadDashboardSnapshot } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";

const REFRESH_MS = 30000;

export function CommsPage() {
  const { t } = useTranslation();
  const channelsQuery = useQuery({
    queryKey: ["channels", "list", "comms"],
    queryFn: listChannels,
    refetchInterval: REFRESH_MS
  });

  const snapshotQuery = useQuery({
    queryKey: ["dashboard", "snapshot", "comms"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const channels = channelsQuery.data ?? [];
  const snapshot = snapshotQuery.data ?? null;
  const isLoading = channelsQuery.isLoading || snapshotQuery.isLoading;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("comms.bus")}
        title={t("nav.comms")}
        subtitle={t("comms.subtitle")}
        isFetching={isLoading}
        onRefresh={() => { void channelsQuery.refetch(); void snapshotQuery.refetch(); }}
        icon={<polyline points="22 12 18 12 15 21 9 3 6 12 2 12" />}
      />

      {isLoading ? (
        <div className="grid gap-6 lg:grid-cols-2">
          <CardSkeleton />
          <CardSkeleton />
        </div>
      ) : (
        <>
          <div className="grid gap-6 lg:grid-cols-2">
            <Card padding="lg">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("comms.active_channels")}</h2>
              <p className="mb-6 text-xs text-text-dim font-medium">{t("comms.active_channels_description")}</p>

              <div className="space-y-3">
                {channels.map((c) => (
                  <div key={c.name} className="flex items-center justify-between p-3 rounded-xl bg-main/40 border border-border-subtle/50">
                    <div className="flex items-center gap-3">
                      <div className={`h-2 w-2 rounded-full ${c.configured ? 'bg-success shadow-[0_0_8px_var(--success-color)]' : 'bg-text-dim/30'}`} />
                      <span className="text-sm font-bold">{c.display_name || c.name}</span>
                    </div>
                    <Badge variant={c.configured ? "success" : "default"}>
                      {c.configured ? t("common.online") : t("comms.unconfigured")}
                    </Badge>
                  </div>
                ))}
                {channels.length === 0 && <p className="text-xs text-text-dim italic py-4 text-center">{t("comms.no_channels")}</p>}
              </div>
            </Card>

            <Card padding="lg">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("overview.system_status")}</h2>
              <p className="mb-6 text-xs text-text-dim font-medium">{t("comms.health_description")}</p>

              <div className="grid gap-3">
                {snapshot?.health.checks?.map((check, i) => (
                  <div key={i} className="flex items-center justify-between p-3 rounded-xl bg-main/40 border border-border-subtle/50">
                    <span className="text-sm font-bold">{check.name}</span>
                    <Badge variant={check.status === 'ok' ? "success" : "error"}>
                      {check.status === 'ok' ? t("common.ok") : t("common.error")}
                    </Badge>
                  </div>
                )) ?? <p className="text-xs text-text-dim italic py-4 text-center">{t("comms.awaiting_telemetry")}</p>}
              </div>
            </Card>
          </div>

          <div className="rounded-2xl bg-brand-muted border border-brand/5 p-8 shadow-sm">
            <h3 className="text-sm font-black uppercase tracking-widest text-brand mb-2">{t("comms.topology")}</h3>
            <p className="text-xs text-text-dim leading-relaxed max-w-xl">
              {t("comms.topology_description")}
            </p>
          </div>
        </>
      )}
    </div>
  );
}
