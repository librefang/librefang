import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { DashboardSnapshot, HealthCheck, VersionResponse } from "../api";
import { loadDashboardSnapshot, getVersionInfo, postQuickInit } from "../api";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Home, RefreshCw, Users, Layers, Server, Network, Zap, MessageCircle, User, Clock, Shield, Sparkles, Calendar, HardDrive, Activity, Globe, Rocket } from "lucide-react";
import { truncateId } from "../lib/string";
import { isProviderAvailable } from "../lib/status";
import { getStatusVariant } from "../lib/status";

const REFRESH_MS = 30000;

export function OverviewPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const snapshotQuery = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "snapshot"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const versionQuery = useQuery<VersionResponse>({
    queryKey: ["version"],
    queryFn: getVersionInfo,
    staleTime: Infinity,
  });

  const snapshot = snapshotQuery.data ?? null;
  const versionInfo = versionQuery.data;
  const isLoading = snapshotQuery.isLoading;

  const queryClient = useQueryClient();
  const [initLoading, setInitLoading] = useState(false);
  const needsInit = snapshot?.status?.config_exists === false;

  const handleInit = async () => {
    setInitLoading(true);
    try {
      await postQuickInit();
      await queryClient.invalidateQueries({ queryKey: ["dashboard", "snapshot"] });
    } catch {
      // ignore — banner will remain if init failed
    } finally {
      setInitLoading(false);
    }
  };

  const agentsActive = snapshot?.status?.active_agent_count ?? 0;
  const agentsTotal = snapshot?.status?.agent_count ?? 0;
  const providersReady = snapshot?.providers?.filter(p => isProviderAvailable(p.auth_status)).length ?? 0;
  const providersTotal = snapshot?.providers?.length ?? 0;
  const channelsReady = snapshot?.channels?.filter(c => c.configured).length ?? 0;
  const skillsCount = snapshot?.skillCount ?? 0;
  const sessionsCount = snapshot?.status?.session_count ?? 0;

  const formatUptime = (seconds?: number): string => {
    if (seconds === undefined || seconds < 0) return "-";
    const d = Math.floor(seconds / 86400);
    const h = Math.floor((seconds % 86400) / 3600);
    const m = Math.floor((seconds % 3600) / 60);

    if (d > 0) return `${d}d ${h}h`;
    if (h > 0) return `${h}h ${m}m`;
    if (m > 0) return `${m}m`;
    return "<1m";
  };

  const translateStatus = (s?: string) => {
    if (!s) return t("status.unknown");
    const key = `status.${s.toLowerCase()}`;
    return t(key, { defaultValue: s });
  };

  // Stats card data
  const statsCards = [
    {
      title: t("overview.active_agents"),
      value: agentsTotal,
      subValue: `${agentsActive} ${t("overview.active")}`,
      icon: Users,
      color: "brand",
      link: "/agents",
      progress: agentsTotal > 0 ? (agentsActive / agentsTotal) * 100 : 0,
    },
    {
      title: t("overview.workflows"),
      value: snapshot?.workflowCount ?? 0,
      subValue: t("common.active"),
      icon: Layers,
      color: "accent",
      link: "/canvas",
    },
    {
      title: t("nav.providers"),
      value: providersReady,
      subValue: `/ ${providersTotal}`,
      icon: Server,
      color: "success",
      link: "/providers",
      progress: providersTotal > 0 ? (providersReady / providersTotal) * 100 : 0,
    },
    {
      title: t("nav.channels"),
      value: channelsReady,
      subValue: t("status.configured"),
      icon: Network,
      color: "warning",
      link: "/channels",
    },
  ];

  // Quick actions
  const quickActions = [
    { label: t("overview.new_workflow"), to: "/canvas", icon: Zap, primary: true },
    { label: t("overview.deploy_agent"), to: "/agents", icon: Users },
    { label: t("overview.open_chat"), to: "/chat", icon: MessageCircle },
    { label: t("nav.scheduler"), to: "/scheduler", icon: Calendar },
  ];

  return (
    <div className="flex flex-col gap-4 sm:gap-6 pb-12 transition-colors duration-300">
      {/* Header */}
      <header className="flex flex-col justify-between gap-3 sm:gap-4 md:flex-row md:items-end">
        <div>
          <div className="hidden sm:flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <Home className="h-4 w-4" />
            {t("overview.system_overview")}
          </div>
          <div className="flex items-center gap-3 sm:block">
            <h1 className="text-xl sm:text-3xl font-extrabold tracking-tight md:text-4xl sm:mt-2">{t("overview.welcome")}</h1>
            <div className="flex items-center gap-2 rounded-full border border-border-subtle bg-surface px-3 py-1 sm:hidden shrink-0">
              <div className={`h-2 w-2 rounded-full ${snapshot?.health?.status === "ok" ? "bg-success" : "bg-warning animate-pulse"}`} />
              <span className="text-[10px] font-semibold text-slate-600 dark:text-slate-300">
                {snapshot?.health?.status === "ok" ? "OK" : "!"}
              </span>
            </div>
          </div>
          <p className="mt-1 sm:mt-2 text-text-dim max-w-2xl font-medium text-xs sm:text-base hidden sm:block">{t("overview.description")}</p>
        </div>
        <div className="hidden sm:flex items-center gap-3">
          <div className="flex items-center gap-2 rounded-full border border-border-subtle bg-surface px-4 py-1.5 shadow-sm">
            <div className={`h-2 w-2 rounded-full ${snapshot?.health?.status === "ok" ? "bg-success shadow-[0_0_8px_var(--success-color)]" : "bg-warning animate-pulse"}`} />
            <span className="text-xs font-semibold text-slate-600 dark:text-slate-300">
              {snapshot?.health?.status === "ok" ? t("overview.operational") : t("overview.alert")}
            </span>
          </div>
          <button
            onClick={() => void snapshotQuery.refetch()}
            className="flex h-9 w-9 items-center justify-center rounded-full border border-border-subtle bg-surface text-text-dim hover:text-brand transition-colors shadow-sm"
          >
            <RefreshCw className={`h-4 w-4 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} />
          </button>
        </div>
      </header>

      {/* Setup Banner */}
      {needsInit && (
        <Card padding="lg" className="border-brand/30 bg-gradient-to-r from-brand/5 via-brand/10 to-brand/5">
          <div className="flex flex-col sm:flex-row items-start sm:items-center gap-4">
            <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-brand/15">
              <Rocket className="h-6 w-6 text-brand" />
            </div>
            <div className="flex-1 min-w-0">
              <h3 className="text-sm font-bold">{t("overview.setup_title")}</h3>
              <p className="mt-1 text-xs text-text-dim">{t("overview.setup_description")}</p>
            </div>
            <button
              onClick={handleInit}
              disabled={initLoading}
              className="shrink-0 rounded-xl bg-brand px-5 py-2.5 text-xs font-bold text-white shadow-lg shadow-brand/20 transition-all hover:shadow-xl hover:shadow-brand/30 hover:-translate-y-0.5 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {initLoading ? t("overview.setup_running") : t("overview.setup_button")}
            </button>
          </div>
        </Card>
      )}

      {/* Stats Cards */}
      <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4 stagger-children">
        {isLoading ? (
          // Loading skeletons
          <>
            {[1, 2, 3, 4].map(i => (
              <CardSkeleton key={i} />
            ))}
          </>
        ) : (
          statsCards.map((stat, i) => (
            <Card
              key={i}
              hover
              padding="md"
              className="cursor-pointer relative overflow-hidden group"
              onClick={() => navigate({ to: stat.link as any })}
            >
              <div className="absolute right-2 top-2 text-brand/30 transition-transform group-hover:scale-110 group-hover:text-brand/40">
                <stat.icon className="h-5 w-5" />
              </div>
              <p className="text-[9px] sm:text-[10px] font-bold uppercase tracking-widest text-text-dim relative z-10">{stat.title}</p>
              <div className="mt-1 sm:mt-2 flex items-baseline gap-1.5 sm:gap-2 relative z-10">
                <span className="text-2xl sm:text-4xl font-black tracking-tight">{stat.value}</span>
                <span className="text-xs font-semibold text-text-dim">{stat.subValue}</span>
              </div>
              {stat.progress !== undefined && (
                <div className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800 relative z-10">
                  <div
                    className="h-full bg-brand shadow-[0_0_8px_var(--brand-color)] transition-all duration-500"
                    style={{ width: `${stat.progress}%` }}
                  />
                </div>
              )}
            </Card>
          ))
        )}
      </div>

      {/* Main Content Grid */}
      <div className="grid gap-4 sm:gap-6 lg:grid-cols-3">
        {/* Left Column */}
        <div className="flex flex-col gap-6 lg:col-span-2">
          {/* Quick Actions */}
          <Card padding="lg">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.quick_actions")}</h3>
            </div>
            <div className="grid grid-cols-2 gap-2 sm:gap-3 sm:grid-cols-4 stagger-children">
              {quickActions.map((action, i) => (
                <button
                  key={i}
                  onClick={() => navigate({ to: action.to as any })}
                  className={`group flex flex-col items-center gap-2 sm:gap-3 rounded-2xl border p-3 sm:p-5 transition-all duration-300 hover:-translate-y-1 hover:shadow-lg ${
                    action.primary
                      ? "border-brand/20 bg-gradient-to-b from-brand/5 to-brand/10 text-brand hover:shadow-brand/15"
                      : "border-border-subtle bg-surface text-text-dim hover:border-brand/30 hover:text-brand"
                  }`}
                >
                  <div className={`w-8 h-8 sm:w-10 sm:h-10 rounded-xl flex items-center justify-center transition-all duration-300 group-hover:scale-110 ${
                    action.primary ? "bg-brand/15" : "bg-main group-hover:bg-brand/10"
                  }`}>
                    <action.icon className="h-4 w-4 sm:h-5 sm:w-5" />
                  </div>
                  <span className="text-[10px] sm:text-[11px] font-bold text-center">{action.label}</span>
                </button>
              ))}
            </div>
          </Card>

          {/* Recent Agents */}
          <Card padding="lg">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.recent_agents")}</h3>
              <button
                onClick={() => navigate({ to: "/agents" })}
                className="text-xs font-bold text-brand hover:underline transition-colors"
              >
                {t("overview.view_all")} →
              </button>
            </div>
            {isLoading ? (
              <div className="grid gap-3 sm:grid-cols-2">
                {[1, 2].map(i => (
                  <div key={i} className="h-16 rounded-xl bg-gradient-to-r from-main via-surface-hover to-main bg-[length:200%_100%]" style={{ animation: "shimmer 1.5s ease-in-out infinite" }} />
                ))}
              </div>
            ) : snapshot?.agents && snapshot.agents.length > 0 ? (
              <div className="grid gap-3 sm:grid-cols-2">
                {snapshot.agents.filter(a => !a.name.includes("-hand") && !a.name.includes(":")).slice(0, 4).map(agent => (
                  <div
                    key={agent.id}
                    className="flex items-center gap-3 rounded-xl border border-border-subtle bg-surface p-3 shadow-sm hover:border-brand/30 transition-colors cursor-pointer"
                    onClick={() => navigate({ to: "/agents" })}
                  >
                    <div className={`flex h-10 w-10 items-center justify-center rounded-lg ${
                      agent.state === 'running' ? 'bg-success/10 text-success' : 'bg-surface-hover text-text-dim'
                    }`}>
                      <User className="h-5 w-5" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-bold">{t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}</p>
                      <p className="truncate text-[10px] text-text-dim uppercase tracking-tight font-medium">
                        {truncateId(agent.id)} · {translateStatus(agent.state)}
                      </p>
                    </div>
                    <Badge variant={getStatusVariant(agent.state)}>
                      {agent.state === 'running' ? '●' : '○'}
                    </Badge>
                  </div>
                ))}
              </div>
            ) : (
              <div className="py-8 text-center text-text-dim border border-dashed border-border-subtle rounded-xl">
                <User className="h-8 w-8 mx-auto mb-2 opacity-50" />
                <p className="text-sm font-medium">{t("overview.no_active_agents")}</p>
              </div>
            )}
          </Card>

          {/* Running Sessions */}
          <Card padding="lg">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("nav.sessions")}</h3>
              <button
                onClick={() => navigate({ to: "/sessions" })}
                className="text-xs font-bold text-brand hover:underline transition-colors"
              >
                {t("overview.view_all")} →
              </button>
            </div>
            <div className="flex flex-wrap items-center gap-4 sm:gap-6">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 sm:h-12 sm:w-12 items-center justify-center rounded-xl bg-success/10">
                  <Clock className="h-5 w-5 sm:h-6 sm:w-6 text-success" />
                </div>
                <div>
                  <p className="text-xl sm:text-2xl font-black">{sessionsCount}</p>
                  <p className="text-[10px] text-text-dim uppercase">{t("overview.active_sessions")}</p>
                </div>
              </div>
              <div className="h-10 w-px bg-border-subtle hidden sm:block" />
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 sm:h-12 sm:w-12 items-center justify-center rounded-xl bg-brand/10">
                  <Shield className="h-5 w-5 sm:h-6 sm:w-6 text-brand" />
                </div>
                <div>
                  <p className="text-xl sm:text-2xl font-black">{skillsCount}</p>
                  <p className="text-[10px] text-text-dim uppercase">{t("nav.skills")}</p>
                </div>
              </div>
            </div>
          </Card>
        </div>

        {/* Right Column */}
        <div className="flex flex-col gap-6">
          {/* System Status */}
          <Card padding="lg" hover>
            <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim mb-4">{t("overview.system_status")}</h3>
            <div className="space-y-3">
              <div className="flex items-center gap-3 p-2.5 rounded-lg bg-main/40">
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center shrink-0"><Clock className="w-4 h-4 text-success" /></div>
                <span className="text-xs text-text-dim flex-1">{t("overview.uptime")}</span>
                <span className="text-sm font-mono font-black">{formatUptime(snapshot?.status?.uptime_seconds)}</span>
              </div>
              <div className="flex items-center gap-3 p-2.5 rounded-lg bg-main/40">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center shrink-0"><HardDrive className="w-4 h-4 text-brand" /></div>
                <span className="text-xs text-text-dim flex-1">{t("overview.memory_usage")}</span>
                <span className="text-sm font-mono font-black">{snapshot?.status?.memory_used_mb ? `${snapshot.status.memory_used_mb} MB` : "-"}</span>
              </div>
              <div className="flex items-center gap-3 p-2.5 rounded-lg bg-main/40">
                <div className="w-8 h-8 rounded-lg bg-warning/10 flex items-center justify-center shrink-0"><Activity className="w-4 h-4 text-warning" /></div>
                <span className="text-xs text-text-dim flex-1">{t("overview.version")}</span>
                <span className="text-sm font-mono font-black text-brand">{versionInfo?.version || snapshot?.status?.version || "-"}</span>
              </div>
              <div className="flex items-center gap-3 p-2.5 rounded-lg bg-main/40">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center shrink-0"><Globe className="w-4 h-4 text-brand" /></div>
                <span className="text-xs text-text-dim flex-1">Hostname</span>
                <span className="text-sm font-mono font-black truncate max-w-[140px]" title={versionInfo?.hostname}>{versionInfo?.hostname || "-"}</span>
              </div>
              <div className="flex items-center gap-3 p-2.5 rounded-lg bg-main/40">
                <div className="w-8 h-8 rounded-lg bg-accent/10 flex items-center justify-center shrink-0"><User className="w-4 h-4 text-accent" /></div>
                <span className="text-xs text-text-dim flex-1">{t("overview.agent_count")}</span>
                <span className="text-sm font-mono font-black">{agentsTotal}</span>
              </div>
            </div>
          </Card>

          {/* Health Checks */}
          <Card padding="lg" hover>
            <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim mb-4">{t("overview.health_checks")}</h3>
            {snapshot?.health?.checks && snapshot.health.checks.length > 0 ? (
              <div className="space-y-2">
                {snapshot.health.checks.map((check: HealthCheck, i: number) => (
                  <div key={i} className="flex items-center gap-3 p-2 rounded-lg hover:bg-main/40 transition-colors">
                    <div className={`relative h-2.5 w-2.5 rounded-full ${check.status === "ok" ? "bg-success" : "bg-warning"}`}>
                      {check.status === "ok" && <span className="absolute inset-0 rounded-full bg-success/40 animate-pulse" />}
                    </div>
                    <span className="flex-1 text-xs font-medium">{check.name}</span>
                    <Badge variant={check.status === "ok" ? "success" : "warning"}>
                      {check.status === "ok" ? "OK" : "WARN"}
                    </Badge>
                  </div>
                ))}
              </div>
            ) : (
              <div className="flex flex-col items-center py-6">
                <div className="relative mb-3">
                  <div className="w-10 h-10 rounded-full bg-success/10 flex items-center justify-center">
                    <Shield className="w-5 h-5 text-success" />
                  </div>
                  <span className="absolute inset-0 rounded-full bg-success/10 animate-pulse" />
                </div>
                <p className="text-xs font-bold text-success">{t("common.daemon_online")}</p>
              </div>
            )}
          </Card>

        </div>
      </div>

      {/* Pro Tip */}
      <div className="hidden sm:flex items-center gap-3 rounded-xl border border-brand/10 bg-gradient-to-r from-brand/5 to-transparent px-4 py-3">
        <Sparkles className="h-4 w-4 text-brand shrink-0" />
        <span className="text-xs text-text-dim"><span className="font-bold text-brand">{t("overview.pro_tip")}</span> — {t("overview.pro_tip_shortcut")}</span>
      </div>
    </div>
  );
}
