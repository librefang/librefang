import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot, getUsageSummary, listUsageByAgent } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { BarChart3 } from "lucide-react";

const REFRESH_MS = 30000;

export function AnalyticsPage() {
  const { t } = useTranslation();
  const snapshotQuery = useQuery({ queryKey: ["dashboard", "snapshot", "analytics"], queryFn: loadDashboardSnapshot, refetchInterval: REFRESH_MS });
  const usageQuery = useQuery({ queryKey: ["usage", "summary"], queryFn: getUsageSummary, refetchInterval: REFRESH_MS });
  const usageByAgentQuery = useQuery({ queryKey: ["usage", "byAgent"], queryFn: listUsageByAgent, refetchInterval: REFRESH_MS });

  const snapshot = snapshotQuery.data ?? null;
  const usage = usageQuery.data ?? null;
  const usageByAgent = usageByAgentQuery.data ?? [];

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
            <Card padding="lg">
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
            </Card>
            <Card padding="lg">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("analytics.runtime")}</h2>
              <p className="mb-6 text-xs text-text-dim font-medium">{t("analytics.runtime_desc")}</p>
              <div className="grid grid-cols-2 gap-4">
                {[{ l: t("analytics.active_agents"), v: snapshot?.status.agent_count || 0 }, { l: t("analytics.configured_channels"), v: snapshot?.channels.length || 0 }, { l: t("analytics.available_skills"), v: snapshot?.skillCount || 0 }, { l: t("analytics.health_checks"), v: snapshot?.health.checks?.length || 0 }].map((s, i) => (
                  <div key={i} className="p-4 rounded-xl bg-main border border-border-subtle/50"><p className="text-[10px] font-black text-text-dim uppercase mb-1">{s.l}</p><p className="text-2xl font-black">{s.v}</p></div>
                ))}
              </div>
            </Card>
          </div>

          {/* 使用统计 */}
          <div className="grid gap-6 md:grid-cols-2">
            <Card padding="lg">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("analytics.usage_summary")}</h2>
              <p className="mb-6 text-xs text-text-dim font-medium">{t("analytics.usage_summary_desc")}</p>
              <div className="grid grid-cols-2 gap-4">
                {[
                  { label: t("analytics.total_requests"), value: usage?.total_requests ?? 0 },
                  { label: t("analytics.total_tokens"), value: usage?.total_tokens ?? 0 },
                  { label: t("analytics.total_cost"), value: `$${(usage?.total_cost ?? 0).toFixed(4)}` },
                  { label: t("analytics.avg_latency"), value: `${(usage?.avg_latency_ms ?? 0).toFixed(0)}ms` },
                ].map((s, i) => (
                  <div key={i} className="p-4 rounded-xl bg-main border border-border-subtle/50">
                    <p className="text-[10px] font-black text-text-dim uppercase mb-1">{s.label}</p>
                    <p className="text-xl font-black">{s.value}</p>
                  </div>
                ))}
              </div>
            </Card>

            <Card padding="lg">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("analytics.usage_by_agent")}</h2>
              <p className="mb-6 text-xs text-text-dim font-medium">{t("analytics.usage_by_agent_desc")}</p>
              <div className="space-y-3 max-h-64 overflow-y-auto">
                {usageByAgent.slice(0, 10).map((u, i) => (
                  <div key={u.agent_id || i} className="flex justify-between items-center p-3 rounded-lg bg-main/50">
                    <div className="min-w-0">
                      <p className="text-sm font-bold truncate">{u.agent_id}</p>
                      <p className="text-[10px] text-text-dim">{u.request_count} requests</p>
                    </div>
                    <div className="text-right shrink-0">
                      <p className="text-sm font-black text-brand">${u.total_cost?.toFixed(4) || "0.0000"}</p>
                      <p className="text-[10px] text-text-dim">{u.total_tokens} tokens</p>
                    </div>
                  </div>
                ))}
                {usageByAgent.length === 0 && (
                  <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
                )}
              </div>
            </Card>
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
