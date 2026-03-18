import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { DashboardSnapshot, HealthCheck } from "../api";
import { loadDashboardSnapshot } from "../api";

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

  const statCardClass = "group relative overflow-hidden rounded-2xl border border-border-subtle bg-surface p-5 transition-all duration-300 hover:border-brand/50 hover:shadow-lg dark:hover:shadow-[0_0_30px_rgba(14,165,233,0.1)]";

  return (
    <div className="flex flex-col gap-8 pb-12 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-6 md:flex-row md:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></svg>
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
          <button onClick={() => void snapshotQuery.refetch()} className="flex h-9 w-9 items-center justify-center rounded-full border border-border-subtle bg-surface text-text-dim hover:text-brand transition-all shadow-sm"><svg className={`h-4 w-4 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" /></svg></button>
        </div>
      </header>

      {snapshot && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <button onClick={() => navigate({ to: "/agents" })} className={statCardClass}>
            <div className="absolute -right-4 -top-4 text-brand/5 transition-transform group-hover:scale-110 group-hover:text-brand/10"><svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /></svg></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.active_agents")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{snapshot.status?.agent_count ?? 0}</span><span className="text-xs font-semibold text-success">{agentsActive} {t("overview.active")}</span></div>
            <div className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800"><div className="h-full bg-brand shadow-[0_0_8px_var(--brand-color)]" style={{ width: `${Math.min(100, (agentsActive / (snapshot.status?.agent_count || 1)) * 100)}%` }} /></div>
          </button>

          <button onClick={() => navigate({ to: "/canvas" })} className={statCardClass}>
            <div className="absolute -right-4 -top-4 text-accent/5 transition-transform group-hover:scale-110 group-hover:text-accent/10"><svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" /></svg></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.workflows")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{t("common.brands.n8n")}</span><span className="text-xs font-semibold text-accent">{t("common.active")}</span></div>
            <p className="mt-4 text-[10px] text-text-dim group-hover:text-brand transition-colors italic font-medium">{t("overview.design_with_canvas")} {t("common.symbols.arrow")}</p>
          </button>

          <button onClick={() => navigate({ to: "/providers" })} className={statCardClass}>
            <div className="absolute -right-4 -top-4 text-success/5 transition-transform group-hover:scale-110 group-hover:text-success/10"><svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" /></svg></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.providers")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{providersReady}</span><span className="text-xs font-semibold text-text-dim">{t("common.symbols.slash")} {snapshot.providers?.length ?? 0}</span></div>
            <div className="mt-4 flex gap-1">{(snapshot.providers || []).slice(0, 5).map((p, i) => (<div key={i} title={p.display_name || p.id} className={`h-1.5 flex-1 rounded-full ${p.auth_status === 'configured' ? 'bg-success/50' : 'bg-slate-100 dark:bg-slate-800'}`} />))}</div>
          </button>

          <button onClick={() => navigate({ to: "/channels" })} className={statCardClass}>
            <div className="absolute -right-4 -top-4 text-warning/5 transition-transform group-hover:scale-110 group-hover:text-warning/10"><svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><circle cx="18" cy="5" r="3" /><circle cx="6" cy="12" r="3" /><circle cx="18" cy="19" r="3" /><line x1="8.59" y1="13.51" x2="15.42" y2="17.49" /><line x1="15.41" y1="6.51" x2="8.59" y2="10.49" /></svg></div>
            <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("overview.channels")}</p>
            <div className="mt-2 flex items-baseline gap-2"><span className="text-4xl font-black tracking-tight">{channelsReady}</span><span className="text-xs font-semibold text-warning">{t("status.configured")}</span></div>
            <p className="mt-4 text-[10px] text-text-dim font-medium">{snapshot.channels?.length ?? 0} {t("overview.adapters_available")}</p>
          </button>
        </div>
      )}

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="flex flex-col gap-6 lg:col-span-2">
          <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm">
            <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.quick_actions")}</h3>
            <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
              {[{ label: t("overview.new_workflow"), to: "/canvas" as const, icon: <path d="M12 5v14M5 12h14" />, primary: true }, { label: t("overview.deploy_agent"), to: "/agents" as const, icon: <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" /> }, { label: t("overview.open_chat"), to: "/chat" as const, icon: <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" /> }, { label: t("nav.settings"), to: "/settings" as const, icon: <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33" /></> }].map((a, i) => (
                <button key={i} onClick={() => navigate({ to: a.to })} className={`flex flex-col items-center gap-2 rounded-xl border p-4 transition-all duration-200 ${a.primary ? "border-brand/30 bg-brand-muted text-brand hover:bg-brand/20 shadow-sm" : "border-border-subtle bg-surface text-text-dim hover:border-brand/30 hover:bg-surface-hover hover:text-brand shadow-sm"}`}><svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">{a.icon}</svg><span className="text-[11px] font-bold text-center">{a.label}</span></button>
              ))}
            </div>
          </div>
          <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm">
            <div className="flex items-center justify-between"><h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.recent_agents")}</h3><button onClick={() => navigate({ to: "/agents" })} className="text-xs font-bold text-brand hover:underline transition-all">{t("overview.view_all")} {t("common.symbols.arrow")}</button></div>
            <div className="mt-4 grid gap-3 sm:grid-cols-2">{snapshot?.agents?.slice(0, 4).map(a => (<div key={a.id} className="flex items-center gap-3 rounded-xl border border-border-subtle bg-surface p-3 shadow-sm ring-1 ring-black/5 dark:ring-white/5"><div className={`flex h-10 w-10 items-center justify-center rounded-lg ${a.state === 'running' ? 'bg-success/10 text-success' : 'bg-surface-hover text-text-dim'}`}><svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" /></svg></div><div className="min-w-0 flex-1"><p className="truncate text-sm font-bold">{a.name}</p><p className="truncate text-[10px] text-text-dim uppercase tracking-tight font-medium">{a.id?.slice(0, 8)} {t("common.symbols.separator")} {translateStatus(a.state)}</p></div><div className={`h-1.5 w-1.5 rounded-full ${a.state === 'running' ? 'bg-success animate-pulse shadow-[0_0_8px_var(--success-color)]' : 'bg-text-dim/30'}`} /></div>)) || <div className="col-span-2 py-8 text-center text-text-dim border border-dashed border-border-subtle rounded-xl font-medium">{t("overview.no_active_agents")}</div>}</div>
          </div>
        </div>
        <div className="flex flex-col gap-6">
          <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm"><h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.system_status")}</h3><div className="mt-4 space-y-4">{[{ l: t("overview.uptime"), v: formatUptimeTranslated(snapshot?.status?.uptime_seconds) }, { l: t("overview.memory_usage"), v: snapshot?.status?.memory_used_mb ? `${snapshot.status.memory_used_mb} ${t("common.units.mb")}` : t("common.symbols.none") }, { l: t("overview.version"), v: snapshot?.status?.version || t("common.symbols.none") }].map((s, i) => (<div key={i} className="flex justify-between border-b border-border-subtle pb-2"><span className="text-xs text-text-dim font-medium">{s.l}</span><span className="text-xs font-mono font-bold text-slate-700 dark:text-slate-200">{s.v}</span></div>))}</div></div>
          <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm"><h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">{t("overview.telemetry")}</h3><div className="mt-4 space-y-3">{snapshot?.health?.checks?.map((c: HealthCheck, i: number) => (<div key={i} className="flex items-center gap-3"><div className={`h-1.5 w-1.5 rounded-full ${c.status === "ok" ? "bg-success" : "bg-warning"}`} /><span className="flex-1 text-xs font-medium text-slate-600 dark:text-slate-300">{c.name}</span><span className={`text-[10px] font-bold uppercase tracking-widest ${c.status === "ok" ? "text-success" : "text-warning"}`}>{c.status === "ok" ? t("common.ok") : t("common.check")}</span></div>)) || <p className="text-xs text-text-dim italic">{t("overview.no_telemetry")}</p>}</div></div>
          <div className="rounded-2xl bg-brand-muted border border-brand/5 p-6 shadow-sm relative overflow-hidden"><div className="absolute -right-4 -bottom-4 text-brand/10"><svg className="h-16 w-16" fill="currentColor" viewBox="0 0 24 24"><path d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" /></svg></div><div className="relative"><div className="flex items-center gap-2 text-brand"><svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" /></svg><span className="text-[10px] font-bold uppercase tracking-widest">{t("overview.pro_tip")}</span></div><p className="mt-2 text-xs leading-relaxed text-slate-600 dark:text-slate-400 font-medium">{t("overview.pro_tip_text")}</p></div></div>
        </div>
      </div>
    </div>
  );
}
