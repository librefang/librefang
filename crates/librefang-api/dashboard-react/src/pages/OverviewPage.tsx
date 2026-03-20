import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { DashboardSnapshot, HealthCheck } from "../api";
import { loadDashboardSnapshot } from "../api";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Home, RefreshCw, Users, Layers, Server, Network, Zap, MessageCircle, User, Clock, Shield, Sparkles, Calendar } from "lucide-react";

const REFRESH_MS = 30000;

export function OverviewPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const snapshotQuery = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "snapshot"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const snapshot = snapshotQuery.data ?? null;
  const isLoading = snapshotQuery.isLoading;

  const agentsActive = snapshot?.status?.active_agent_count ?? 0;
  const agentsTotal = snapshot?.status?.agent_count ?? 0;
  const providersReady = snapshot?.providers?.filter(p => p.auth_status === "configured").length ?? 0;
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

  const getStatusVariant = (state?: string): "success" | "warning" | "error" | "default" => {
    switch (state) {
      case "running": return "success";
      case "idle": return "warning";
      case "error": return "error";
      default: return "default";
    }
  };

  // 统计卡片数据
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

  // 快捷操作
  const quickActions = [
    { label: t("overview.new_workflow"), to: "/canvas", icon: Zap, primary: true },
    { label: t("overview.deploy_agent"), to: "/agents", icon: Users },
    { label: t("overview.open_chat"), to: "/chat", icon: MessageCircle },
    { label: t("nav.scheduler"), to: "/scheduler", icon: Calendar },
  ];

  return (
    <div className="flex flex-col gap-6 pb-12 transition-colors duration-300">
      {/* Header */}
      <header className="flex flex-col justify-between gap-4 md:flex-row md:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <Home className="h-4 w-4" />
            {t("overview.system_overview")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("overview.welcome")}</h1>
          <p className="mt-2 text-text-dim max-w-2xl font-medium">{t("overview.description")}</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 rounded-full border border-border-subtle bg-surface px-4 py-1.5 backdrop-blur-md shadow-sm">
            <div className={`h-2 w-2 rounded-full ${snapshot?.health?.status === "ok" ? "bg-success shadow-[0_0_8px_var(--success-color)]" : "bg-warning animate-pulse"}`} />
            <span className="text-xs font-semibold text-slate-600 dark:text-slate-300">
              {snapshot?.health?.status === "ok" ? t("overview.operational") : t("overview.alert")}
            </span>
          </div>
          <button
            onClick={() => void snapshotQuery.refetch()}
            className="flex h-9 w-9 items-center justify-center rounded-full border border-border-subtle bg-surface text-text-dim hover:text-brand transition-all shadow-sm"
          >
            <RefreshCw className={`h-4 w-4 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} />
          </button>
        </div>
      </header>

      {/* Stats Cards */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {isLoading ? (
          // Loading skeletons
          <>
            {[1, 2, 3, 4].map(i => (
              <Card key={i} padding="md" className="animate-pulse">
                <div className="h-4 w-24 bg-surface-hover rounded mb-3" />
                <div className="h-8 w-16 bg-surface-hover rounded" />
              </Card>
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
              <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim relative z-10">{stat.title}</p>
              <div className="mt-2 flex items-baseline gap-2 relative z-10">
                <span className="text-4xl font-black tracking-tight">{stat.value}</span>
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
      <div className="grid gap-6 lg:grid-cols-3">
        {/* Left Column */}
        <div className="flex flex-col gap-6 lg:col-span-2">
          {/* Quick Actions */}
          <Card padding="lg">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.quick_actions")}</h3>
            </div>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              {quickActions.map((action, i) => (
                <button
                  key={i}
                  onClick={() => navigate({ to: action.to as any })}
                  className={`flex flex-col items-center gap-2 rounded-xl border p-4 transition-all duration-200 ${
                    action.primary
                      ? "border-brand/30 bg-brand-muted text-brand hover:bg-brand/20 shadow-sm"
                      : "border-border-subtle bg-surface text-text-dim hover:border-brand/30 hover:bg-surface-hover hover:text-brand shadow-sm"
                  }`}
                >
                  <action.icon className="h-5 w-5" />
                  <span className="text-[11px] font-bold text-center">{action.label}</span>
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
                className="text-xs font-bold text-brand hover:underline transition-all"
              >
                {t("overview.view_all")} →
              </button>
            </div>
            {isLoading ? (
              <div className="grid gap-3 sm:grid-cols-2">
                {[1, 2].map(i => (
                  <div key={i} className="h-16 bg-surface-hover rounded-xl animate-pulse" />
                ))}
              </div>
            ) : snapshot?.agents && snapshot.agents.length > 0 ? (
              <div className="grid gap-3 sm:grid-cols-2">
                {snapshot.agents.slice(0, 4).map(agent => (
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
                      <p className="truncate text-sm font-bold">{agent.name}</p>
                      <p className="truncate text-[10px] text-text-dim uppercase tracking-tight font-medium">
                        {agent.id?.slice(0, 8)} · {translateStatus(agent.state)}
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
                className="text-xs font-bold text-brand hover:underline transition-all"
              >
                {t("overview.view_all")} →
              </button>
            </div>
            <div className="flex items-center gap-6">
              <div className="flex items-center gap-3">
                <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-success/10">
                  <Clock className="h-6 w-6 text-success" />
                </div>
                <div>
                  <p className="text-2xl font-black">{sessionsCount}</p>
                  <p className="text-[10px] text-text-dim uppercase">活跃会话</p>
                </div>
              </div>
              <div className="h-10 w-px bg-border-subtle" />
              <div className="flex items-center gap-3">
                <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-brand/10">
                  <Shield className="h-6 w-6 text-brand" />
                </div>
                <div>
                  <p className="text-2xl font-black">{skillsCount}</p>
                  <p className="text-[10px] text-text-dim uppercase">{t("nav.skills")}</p>
                </div>
              </div>
            </div>
          </Card>
        </div>

        {/* Right Column */}
        <div className="flex flex-col gap-6">
          {/* System Status */}
          <Card padding="lg">
            <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim mb-4">{t("overview.system_status")}</h3>
            <div className="space-y-4">
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-medium text-text-dim uppercase">运行时间</span>
                <span className="text-sm font-mono font-bold text-slate-700 dark:text-slate-200">
                  {formatUptime(snapshot?.status?.uptime_seconds)}
                </span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-medium text-text-dim uppercase">内存使用</span>
                <span className="text-sm font-mono font-bold text-slate-700 dark:text-slate-200">
                  {snapshot?.status?.memory_used_mb ? `${snapshot.status.memory_used_mb} MB` : "-"}
                </span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
                <span className="text-xs font-medium text-text-dim uppercase">版本</span>
                <span className="text-sm font-mono font-bold text-slate-700 dark:text-slate-200">
                  {snapshot?.status?.version || "-"}
                </span>
              </div>
              <div className="flex justify-between items-center py-2">
                <span className="text-xs font-medium text-text-dim uppercase">Agent 数量</span>
                <span className="text-sm font-mono font-bold text-slate-700 dark:text-slate-200">
                  {agentsTotal}
                </span>
              </div>
            </div>
          </Card>

          {/* Health Checks */}
          <Card padding="lg">
            <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim mb-4">健康检查</h3>
            {snapshot?.health?.checks && snapshot.health.checks.length > 0 ? (
              <div className="space-y-3">
                {snapshot.health.checks.map((check: HealthCheck, i: number) => (
                  <div key={i} className="flex items-center gap-3">
                    <div className={`h-2 w-2 rounded-full ${check.status === "ok" ? "bg-success" : "bg-warning"}`} />
                    <span className="flex-1 text-xs font-medium text-slate-600 dark:text-slate-300">{check.name}</span>
                    <Badge variant={check.status === "ok" ? "success" : "warning"}>
                      {check.status === "ok" ? "OK" : "WARN"}
                    </Badge>
                  </div>
                ))}
              </div>
            ) : (
              <p className="text-xs text-text-dim italic">{t("overview.no_telemetry")}</p>
            )}
          </Card>

          {/* Pro Tip */}
          <Card padding="lg" className="bg-gradient-to-br from-brand/5 to-transparent border-brand/10">
            <div className="flex items-start gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-brand/10 shrink-0">
                <Sparkles className="h-5 w-5 text-brand" />
              </div>
              <div>
                <h4 className="text-xs font-bold uppercase tracking-wider text-brand mb-1">使用提示</h4>
                <p className="text-xs leading-relaxed text-text-dim">
                  使用 <kbd className="px-1 py-0.5 bg-surface rounded text-[10px] font-mono">⌘K</kbd> 快速搜索
                  · 按 <kbd className="px-1 py-0.5 bg-surface rounded text-[10px] font-mono">←</kbd> 折叠侧边栏
                </p>
              </div>
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}
