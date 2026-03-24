import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type { DashboardSnapshot, VersionResponse, QueueStatusResponse, HealthCheck } from "../api";
import { loadDashboardSnapshot, getVersionInfo, getQueueStatus } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import {
  Activity, Cpu, HardDrive, Zap, Timer, Layers, CheckCircle2, GitCommit,
  Calendar, Server, Monitor, Settings, HeartPulse, Box, Globe, FolderOpen,
  FileText, Gauge, Network, XCircle,
} from "lucide-react";

const REFRESH_MS = 30000;

function formatUptime(seconds?: number): string {
  if (seconds === undefined || seconds <= 0) return "-";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return "<1m";
}

function InfoRow({ icon: Icon, label, value, mono, color }: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: React.ReactNode;
  mono?: boolean;
  color?: string;
}) {
  return (
    <div className="flex items-center gap-3">
      <Icon className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
      <span className="text-xs text-text-dim flex-1">{label}</span>
      <span className={`text-sm ${mono ? "font-mono" : ""} ${color ?? "text-text"} truncate max-w-[200px]`}>{value}</span>
    </div>
  );
}

export function RuntimePage() {
  const { t } = useTranslation();

  const snapshotQuery = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "snapshot", "runtime"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS,
  });
  const versionQuery = useQuery<VersionResponse>({
    queryKey: ["version"],
    queryFn: getVersionInfo,
    refetchInterval: REFRESH_MS * 2,
  });
  const queueQuery = useQuery<QueueStatusResponse>({
    queryKey: ["queue", "status"],
    queryFn: getQueueStatus,
    refetchInterval: 5000,
  });

  const snapshot = snapshotQuery.data ?? null;
  const version = versionQuery.data ?? null;
  const queue = queueQuery.data ?? null;
  const status = snapshot?.status;

  const uptimeStr = formatUptime(status?.uptime_seconds);
  const healthChecks = snapshot?.health?.checks ?? [];
  const allHealthy = healthChecks.length > 0 && healthChecks.every((c: HealthCheck) => c.status === "ok" || c.status === "pass" || c.status === "healthy");
  const lanes = queue?.lanes ?? [];
  const queueConfig = queue?.config;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("runtime.kernel")}
        title={t("runtime.title")}
        subtitle={t("runtime.subtitle")}
        isFetching={snapshotQuery.isFetching}
        onRefresh={() => { snapshotQuery.refetch(); versionQuery.refetch(); queueQuery.refetch(); }}
        icon={<Activity className="h-4 w-4" />}
      />

      {snapshotQuery.isLoading ? (
        <div className="grid gap-4 grid-cols-2 md:grid-cols-4 stagger-children">
          {[1, 2, 3, 4].map(i => <CardSkeleton key={i} />)}
        </div>
      ) : (
        <>
          {/* KPI Cards */}
          <div className="grid grid-cols-2 gap-2 sm:gap-4 xl:grid-cols-4 stagger-children">
            {[
              { icon: Timer, label: t("runtime.system_uptime"), value: uptimeStr, color: "text-success", bg: "bg-success/10" },
              { icon: Layers, label: t("runtime.active_agents"), value: `${status?.active_agent_count ?? 0} / ${status?.agent_count ?? 0}`, color: "text-brand", bg: "bg-brand/10" },
              { icon: Monitor, label: t("runtime.sessions"), value: String(status?.session_count ?? 0), color: "text-purple-500", bg: "bg-purple-500/10" },
              { icon: HardDrive, label: t("runtime.memory_used"), value: status?.memory_used_mb ? `${status.memory_used_mb} MB` : "-", color: "text-warning", bg: "bg-warning/10" },
            ].map((kpi, i) => (
              <Card key={i} hover padding="md">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
                  <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}>
                    <kpi.icon className={`w-4 h-4 ${kpi.color}`} />
                  </div>
                </div>
                <p className={`text-2xl sm:text-3xl font-black tracking-tight mt-1 sm:mt-2 ${kpi.color}`}>{kpi.value}</p>
              </Card>
            ))}
          </div>

          {/* Engine Info + Runtime Config */}
          <div className="grid gap-3 sm:gap-6 md:grid-cols-2 stagger-children">
            {/* Engine & Build */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Cpu className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.engine")}</h2>
              </div>
              <div className="space-y-3">
                <InfoRow icon={Activity} label={t("runtime.engine_version")} value={version?.version || status?.version || t("common.unknown")} mono color="font-bold text-brand" />
                <InfoRow icon={GitCommit} label={t("runtime.git_hash")} value={version?.git_sha ? version.git_sha.slice(0, 12) : "-"} mono />
                <InfoRow icon={Calendar} label={t("runtime.build_time")} value={version?.build_date || "-"} />
                <InfoRow icon={FileText} label={t("runtime.rust_version")} value={version?.rust_version || "-"} mono />
                <InfoRow icon={Server} label={t("runtime.platform")} value={version?.platform && version?.arch ? `${version.platform} / ${version.arch}` : "-"} />
                <InfoRow icon={Globe} label={t("runtime.hostname")} value={version?.hostname || "-"} />
              </div>
            </Card>

            {/* Runtime Config */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-purple-500/10 flex items-center justify-center"><Settings className="h-4 w-4 text-purple-500" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.config")}</h2>
              </div>
              <div className="space-y-3">
                <InfoRow icon={Box} label={t("runtime.default_provider")} value={status?.default_provider || "-"} color="font-bold" />
                <InfoRow icon={Cpu} label={t("runtime.default_model")} value={status?.default_model || "-"} mono />
                <InfoRow icon={Network} label={t("runtime.api_listen")} value={status?.api_listen || "-"} mono />
                <InfoRow icon={FolderOpen} label={t("runtime.home_dir")} value={status?.home_dir || "-"} mono />
                <InfoRow icon={Gauge} label={t("runtime.log_level")} value={
                  status?.log_level ? <Badge variant="info">{status.log_level}</Badge> : "-"
                } />
                <InfoRow icon={Globe} label={t("runtime.network_enabled")} value={
                  status?.network_enabled !== undefined
                    ? <Badge variant={status.network_enabled ? "success" : "default"}>{status.network_enabled ? t("runtime.enabled") : t("runtime.disabled")}</Badge>
                    : "-"
                } />
              </div>
            </Card>
          </div>

          {/* Health Checks + Status */}
          <div className="grid gap-3 sm:gap-6 md:grid-cols-2 lg:grid-cols-3 stagger-children">
            {/* Health Checks */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className={`w-8 h-8 rounded-lg ${allHealthy ? "bg-success/10" : "bg-warning/10"} flex items-center justify-center`}>
                  <HeartPulse className={`h-4 w-4 ${allHealthy ? "text-success" : "text-warning"}`} />
                </div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.health_checks")}</h2>
                <Badge variant={allHealthy ? "success" : "warning"} className="ml-auto">
                  {allHealthy ? t("runtime.all_passed") : t("runtime.degraded")}
                </Badge>
              </div>
              {healthChecks.length === 0 ? (
                <p className="text-xs text-text-dim">{t("common.no_data")}</p>
              ) : (
                <div className="space-y-2.5">
                  {healthChecks.map((check: HealthCheck) => {
                    const ok = check.status === "ok" || check.status === "pass" || check.status === "healthy";
                    return (
                      <div key={check.name} className="flex items-center gap-2.5">
                        {ok
                          ? <CheckCircle2 className="w-3.5 h-3.5 text-success shrink-0" />
                          : <XCircle className="w-3.5 h-3.5 text-error shrink-0" />
                        }
                        <span className="text-xs flex-1">{check.name}</span>
                        <Badge variant={ok ? "success" : "error"}>{check.status}</Badge>
                      </div>
                    );
                  })}
                </div>
              )}
            </Card>

            {/* Task Queue Lanes */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-warning/10 flex items-center justify-center"><Zap className="h-4 w-4 text-warning" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.queue")}</h2>
              </div>
              {lanes.length === 0 ? (
                <p className="text-xs text-text-dim">{t("runtime.no_lanes")}</p>
              ) : (
                <div className="space-y-3">
                  {lanes.map((lane) => {
                    const active = lane.active ?? 0;
                    const capacity = lane.capacity ?? 1;
                    const pct = capacity > 0 ? Math.min((active / capacity) * 100, 100) : 0;
                    const color = pct >= 80 ? "bg-error" : pct >= 50 ? "bg-warning" : "bg-brand";
                    return (
                      <div key={lane.lane ?? "default"}>
                        <div className="flex items-center justify-between mb-1">
                          <span className="text-xs font-medium">{lane.lane || "default"}</span>
                          <span className="text-xs text-text-dim font-mono">{active} / {capacity}</span>
                        </div>
                        <div className="h-2 rounded-full bg-main overflow-hidden">
                          <div className={`h-full rounded-full ${color} transition-all duration-500`} style={{ width: `${pct}%` }} />
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
              {queueConfig && (
                <div className="mt-4 pt-4 border-t border-border-subtle space-y-2">
                  <p className="text-[10px] font-bold uppercase tracking-wider text-text-dim/50 mb-2">{t("runtime.queue_config")}</p>
                  <div className="grid grid-cols-3 gap-2 text-center">
                    <div>
                      <p className="text-lg font-black text-brand">{queueConfig.max_depth_per_agent ?? "-"}</p>
                      <p className="text-[9px] text-text-dim uppercase">{t("runtime.max_depth_agent")}</p>
                    </div>
                    <div>
                      <p className="text-lg font-black text-brand">{queueConfig.max_depth_global ?? "-"}</p>
                      <p className="text-[9px] text-text-dim uppercase">{t("runtime.max_depth_global")}</p>
                    </div>
                    <div>
                      <p className="text-lg font-black text-brand">{queueConfig.task_ttl_secs ? `${queueConfig.task_ttl_secs}s` : "-"}</p>
                      <p className="text-[9px] text-text-dim uppercase">{t("runtime.task_ttl")}</p>
                    </div>
                  </div>
                </div>
              )}
            </Card>

            {/* Resource Summary */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center"><Layers className="h-4 w-4 text-success" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.resources")}</h2>
              </div>
              <div className="grid grid-cols-2 gap-4">
                {[
                  { label: t("runtime.providers"), value: snapshot?.providers?.length ?? 0, sub: `${snapshot?.providers?.filter(p => p.auth_status === "configured").length ?? 0} ${t("status.configured").toLowerCase()}`, color: "text-brand" },
                  { label: t("runtime.channels"), value: snapshot?.channels?.length ?? 0, sub: `${snapshot?.channels?.filter(c => c.configured).length ?? 0} ${t("status.configured").toLowerCase()}`, color: "text-purple-500" },
                  { label: t("runtime.skills"), value: snapshot?.skillCount ?? 0, sub: t("status.active").toLowerCase(), color: "text-success" },
                  { label: t("runtime.workflows"), value: snapshot?.workflowCount ?? 0, sub: t("common.config").toLowerCase(), color: "text-warning" },
                ].map((item) => (
                  <div key={item.label} className="text-center">
                    <p className={`text-2xl font-black ${item.color}`}>{item.value}</p>
                    <p className="text-xs font-bold">{item.label}</p>
                    <p className="text-[10px] text-text-dim">{item.sub}</p>
                  </div>
                ))}
              </div>

              {/* Overall system status */}
              <div className="mt-5 pt-4 border-t border-border-subtle flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <span className="relative flex h-2.5 w-2.5">
                    <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-ping" />
                    <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-success" />
                  </span>
                  <span className="text-xs font-bold">{t("runtime.status")}</span>
                </div>
                <Badge variant="success">{t("status.nominal")}</Badge>
              </div>
            </Card>
          </div>
        </>
      )}
    </div>
  );
}
