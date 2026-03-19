import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { DashboardSnapshot, HealthCheck } from "../api";
import { loadDashboardSnapshot } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Home, RefreshCw, Users, Layers, Server, Network, Zap, MessageCircle, Settings, User, Info } from "lucide-react";

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
  const agentsActive = snapshot?.status?.active_agent_count ?? 0;
  const providersReady = snapshot?.providers?.filter(p => p.auth_status === "configured").length ?? 0;
  const channelsReady = snapshot?.channels?.filter(c => c.configured).length ?? 0;

  const formatUptimeTranslated = (seconds?: number): string => {
    if (seconds === undefined || seconds < 0) return t("common.symbols.none");
    const d = Math.floor(seconds / 86400);
    const h = Math.floor((seconds % 86400) / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);

    if (d > 0) return `${d}${t("common.units.days_short")} ${h}${t("common.units.hours_short")}`;
    if (h > 0) return `${h}${t("common.units.hours_short")} ${m}${t("common.units.minutes_short")}`;
    if (m > 0) return `${m}${t("common.units.minutes_short")} ${s}${t("common.units.seconds_short")}`;
    return `${s}${t("common.units.seconds_short")}`;
  };

  const translateStatus = (s?: string) => {
    if (!s) return t("status.unknown");
    const key = `status.${s.toLowerCase()}`;
    return t(key, { defaultValue: s });
  };

  return (
    <div className="flex flex-col gap-8 pb-12 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-6 md:flex-row md:items-end">
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
            <span className="text-xs font-semibold text-slate-600 dark:text-slate-300">{snapshot?.health?.status === "ok" ? t("overview.operational") : t("overview.alert")}</span>
          </div>
          <Button variant="secondary" size="sm" onClick={() => void snapshotQuery.refetch()}>
            <RefreshCw className={`h-4 w-4 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </header>

      {snapshot && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <Card hover padding="md" className="cursor-pointer relative overflow-hidden" onClick={() => navigate({ to: "/agents" })}>
            <div className="absolute -right-4 -top-4 text-brand/5 transition-transform group-hover:scale-110 group-hover:text-brand/10"><Users className="h-24 w-24" /></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.active_agents")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{snapshot.status?.agent_count ?? 0}</span><span className="text-xs font-semibold text-success">{agentsActive} {t("overview.active")}</span></div>
            <div className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800"><div className="h-full bg-brand shadow-[0_0_8px_var(--brand-color)]" style={{ width: `${Math.min(100, (agentsActive / (snapshot.status?.agent_count || 1)) * 100)}%` }} /></div>
          </Card>

          <Card hover padding="md" className="cursor-pointer relative overflow-hidden" onClick={() => navigate({ to: "/canvas" })}>
            <div className="absolute -right-4 -top-4 text-accent/5 transition-transform group-hover:scale-110 group-hover:text-accent/10"><Layers className="h-24 w-24" /></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.workflows")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{snapshot?.workflowCount ?? 0}</span><span className="text-xs font-semibold text-accent">{t("common.active")}</span></div>
            <p className="mt-4 text-[10px] text-text-dim group-hover:text-brand transition-colors italic font-medium">{t("overview.design_with_canvas")} {t("common.symbols.arrow")}</p>
          </Card>

          <Card hover padding="md" className="cursor-pointer relative overflow-hidden" onClick={() => navigate({ to: "/providers" })}>
            <div className="absolute -right-4 -top-4 text-success/5 transition-transform group-hover:scale-110 group-hover:text-success/10"><Server className="h-24 w-24" /></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.providers")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{providersReady}</span><span className="text-xs font-semibold text-text-dim">{t("common.symbols.slash")} {snapshot.providers?.length ?? 0}</span></div>
            <div className="mt-4 flex gap-1">{(snapshot.providers || []).slice(0, 5).map((p, i) => (<div key={i} title={p.display_name || p.id} className={`h-1.5 flex-1 rounded-full ${p.auth_status === 'configured' ? 'bg-success/50' : 'bg-slate-100 dark:bg-slate-800'}`} />))}</div>
          </Card>

          <Card hover padding="md" className="cursor-pointer relative overflow-hidden" onClick={() => navigate({ to: "/channels" })}>
            <div className="absolute -right-4 -top-4 text-warning/5 transition-transform group-hover:scale-110 group-hover:text-warning/10"><Network className="h-24 w-24" /></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.channels")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{channelsReady}</span><span className="text-xs font-semibold text-warning">{t("status.configured")}</span></div>
            <p className="mt-4 text-[10px] text-text-dim font-medium">{snapshot.channels?.length ?? 0} {t("overview.adapters_available")}</p>
          </Card>
        </div>
      )}

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="flex flex-col gap-6 lg:col-span-2">
          <Card padding="lg">
            <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.quick_actions")}</h3>
            <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
              {[{ label: t("overview.new_workflow"), to: "/canvas" as const, icon: <Zap className="h-5 w-5" />, primary: true }, { label: t("overview.deploy_agent"), to: "/agents" as const, icon: <Users className="h-5 w-5" /> }, { label: t("overview.open_chat"), to: "/chat" as const, icon: <MessageCircle className="h-5 w-5" /> }, { label: t("nav.settings"), to: "/settings" as const, icon: <Settings className="h-5 w-5" /> }].map((a, i) => (
                <button key={i} onClick={() => navigate({ to: a.to })} className={`flex flex-col items-center gap-2 rounded-xl border p-4 transition-all duration-200 ${a.primary ? "border-brand/30 bg-brand-muted text-brand hover:bg-brand/20 shadow-sm" : "border-border-subtle bg-surface text-text-dim hover:border-brand/30 hover:bg-surface-hover hover:text-brand shadow-sm"}`}>{a.icon}<span className="text-[11px] font-bold text-center">{a.label}</span></button>
              ))}
            </div>
          </Card>
          <Card padding="lg">
            <div className="flex items-center justify-between"><h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.recent_agents")}</h3><button onClick={() => navigate({ to: "/agents" })} className="text-xs font-bold text-brand hover:underline transition-all">{t("overview.view_all")} {t("common.symbols.arrow")}</button></div>
            <div className="mt-4 grid gap-3 sm:grid-cols-2">{snapshot?.agents?.slice(0, 4).map(a => (<div key={a.id} className="flex items-center gap-3 rounded-xl border border-border-subtle bg-surface p-3 shadow-sm ring-1 ring-black/5 dark:ring-white/5"><div className={`flex h-10 w-10 items-center justify-center rounded-lg ${a.state === 'running' ? 'bg-success/10 text-success' : 'bg-surface-hover text-text-dim'}`}><User className="h-5 w-5" /></div><div className="min-w-0 flex-1"><p className="truncate text-sm font-bold">{a.name}</p><p className="truncate text-[10px] text-text-dim uppercase tracking-tight font-medium">{a.id?.slice(0, 8)} {t("common.symbols.separator")} {translateStatus(a.state)}</p></div><div className={`h-1.5 w-1.5 rounded-full ${a.state === 'running' ? 'bg-success animate-pulse shadow-[0_0_8px_var(--success-color)]' : 'bg-text-dim/30'}`} /></div>)) || <div className="col-span-2 py-8 text-center text-text-dim border border-dashed border-border-subtle rounded-xl font-medium">{t("overview.no_active_agents")}</div>}</div>
          </Card>
        </div>
        <div className="flex flex-col gap-6">
          <Card padding="lg"><h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.system_status")}</h3><div className="mt-4 space-y-4">{[{ l: t("overview.uptime"), v: formatUptimeTranslated(snapshot?.status?.uptime_seconds) }, { l: t("overview.memory_usage"), v: snapshot?.status?.memory_used_mb ? `${snapshot.status.memory_used_mb} ${t("common.units.mb")}` : t("common.symbols.none") }, { l: t("overview.version"), v: snapshot?.status?.version || t("common.symbols.none") }].map((s, i) => (<div key={i} className="flex justify-between border-b border-border-subtle pb-2"><span className="text-xs text-text-dim font-medium">{s.l}</span><span className="text-xs font-mono font-bold text-slate-700 dark:text-slate-200">{s.v}</span></div>))}</div></Card>
          <Card padding="lg"><h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.telemetry")}</h3><div className="mt-4 space-y-3">{snapshot?.health?.checks?.map((c: HealthCheck, i: number) => (<div key={i} className="flex items-center gap-3"><div className={`h-1.5 w-1.5 rounded-full ${c.status === "ok" ? "bg-success" : "bg-warning"}`} /><span className="flex-1 text-xs font-medium text-slate-600 dark:text-slate-300">{c.name}</span><span className={`text-[10px] font-bold uppercase tracking-widest ${c.status === "ok" ? "text-success" : "text-warning"}`}>{c.status === "ok" ? t("common.ok") : t("common.check")}</span></div>)) || <p className="text-xs text-text-dim italic">{t("overview.no_telemetry")}</p>}</div></Card>
          <Card padding="lg" className="bg-brand-muted border-brand/5"><div className="absolute -right-4 -bottom-4 text-brand/10"><Info className="h-16 w-16" /></div><div className="relative"><div className="flex items-center gap-2 text-brand"><Info className="h-4 w-4" /><span className="text-[10px] font-bold uppercase tracking-widest">{t("overview.pro_tip")}</span></div><p className="mt-2 text-xs leading-relaxed text-slate-600 dark:text-slate-400 font-medium">{t("overview.pro_tip_text")}</p></div></Card>
        </div>
      </div>
    </div>
  );
}
