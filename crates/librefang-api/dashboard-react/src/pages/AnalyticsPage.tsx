import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { BarChart3 } from "lucide-react";

const REFRESH_MS = 30000;

export function AnalyticsPage() {
  const { t } = useTranslation();
  const snapshotQuery = useQuery({ queryKey: ["dashboard", "snapshot", "analytics"], queryFn: loadDashboardSnapshot, refetchInterval: REFRESH_MS });
  const snapshot = snapshotQuery.data ?? null;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("analytics.intelligence")}
        title={t("analytics.title")}
        subtitle={t("analytics.subtitle")}
        isFetching={snapshotQuery.isFetching}
        onRefresh={() => void snapshotQuery.refetch()}
        icon={<BarChart3 className="h-4 w-4" />}
      />

      {snapshotQuery.isLoading ? (
        <div className="grid gap-6 md:grid-cols-2">
          <CardSkeleton />
          <CardSkeleton />
        </div>
      ) : (
        <>
          <div className="grid gap-6 md:grid-cols-2">
            <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("analytics.compute")}</h2>
              <p className="mb-6 text-xs text-text-dim font-medium">{t("analytics.compute_desc")}</p>
              <div className="space-y-4">
                {snapshot?.providers.map((p) => (
                  <div key={p.id}>
                    <div className="flex justify-between mb-1 text-xs"><span className="font-bold">{p.display_name || p.id}</span><span className="text-text-dim">{p.latency_ms ? `${p.latency_ms}ms` : "-"}</span></div>
                    <div className="h-2 w-full rounded-full bg-main overflow-hidden"><div className="h-full bg-brand shadow-[0_0_8px_var(--brand-color)]" style={{ width: `${Math.min(100, (p.model_count || 0) * 10)}%` }} /></div>
                  </div>
                ))}
                {(!snapshot?.providers || snapshot.providers.length === 0) && (
                  <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
                )}
              </div>
            </section>
            <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("analytics.runtime")}</h2>
              <p className="mb-6 text-xs text-text-dim font-medium">{t("analytics.runtime_desc")}</p>
              <div className="grid grid-cols-2 gap-4">
                {[{ l: t("analytics.active_agents"), v: snapshot?.status.agent_count || 0 }, { l: t("analytics.configured_channels"), v: snapshot?.channels.length || 0 }, { l: t("analytics.available_skills"), v: snapshot?.skillCount || 0 }, { l: t("analytics.health_checks"), v: snapshot?.health.checks?.length || 0 }].map((s, i) => (
                  <div key={i} className="p-4 rounded-xl bg-main border border-border-subtle/50"><p className="text-[10px] font-black text-text-dim uppercase mb-1">{s.l}</p><p className="text-2xl font-black">{s.v}</p></div>
                ))}
              </div>
            </section>
          </div>

          <div className="rounded-2xl border border-dashed border-border-subtle p-12 text-center bg-surface/30">
            <h3 className="text-lg font-black tracking-tight">{t("analytics.advanced")}</h3>
            <p className="text-sm text-text-dim mt-1">{t("analytics.advanced_desc")}</p>
          </div>
        </>
      )}
    </div>
  );
}
