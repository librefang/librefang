import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import type { DashboardSnapshot } from "../api";
import { loadDashboardSnapshot } from "../api";

const REFRESH_MS = 30000;

function formatUptime(seconds?: number): string {
  if (!seconds || seconds < 0) return "-";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

export function OverviewPage() {
  const navigate = useNavigate();
  const snapshotQuery = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "snapshot"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const snapshot = snapshotQuery.data ?? null;
  const loading = snapshotQuery.isLoading;
  const error = snapshotQuery.error instanceof Error ? snapshotQuery.error.message : "";

  const providersReady = snapshot?.providers?.filter((p) => p.auth_status === "configured").length ?? 0;
  const channelsReady = snapshot?.channels?.filter((c) => c.configured && c.has_token).length ?? 0;
  const agentsActive = snapshot?.status?.active_agent_count ?? 0;

  const statCardClass = "group relative overflow-hidden rounded-2xl border border-border-subtle bg-surface p-5 transition-all duration-300 hover:border-brand/50 hover:shadow-lg dark:hover:shadow-[0_0_30px_rgba(14,165,233,0.1)]";

  return (
    <div className="flex flex-col gap-8 pb-12 transition-colors duration-300">
      {/* Header / Hero */}
      <header className="flex flex-col justify-between gap-6 md:flex-row md:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
            </svg>
            System Overview
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Welcome to LibreFang</h1>
          <p className="mt-2 text-text-dim max-w-2xl font-medium">Manage your autonomous agent infrastructure and multi-channel workflows from a single unified control center.</p>
        </div>
        
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 rounded-full border border-border-subtle bg-surface px-4 py-1.5 backdrop-blur-md shadow-sm">
            <div className={`h-2 w-2 rounded-full ${snapshot?.health?.status === "ok" ? "bg-success shadow-[0_0_8px_var(--success-color)]" : "bg-warning animate-pulse"}`} />
            <span className="text-xs font-semibold text-slate-600 dark:text-slate-300">
              {snapshot?.health?.status === "ok" ? "Operational" : "System Alert"}
            </span>
          </div>
          <button
            onClick={() => void snapshotQuery.refetch()}
            disabled={snapshotQuery.isFetching}
            className="flex h-9 w-9 items-center justify-center rounded-full border border-border-subtle bg-surface text-text-dim hover:text-brand transition-all shadow-sm disabled:opacity-50"
          >
            <svg className={`h-4 w-4 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
              <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          </button>
        </div>
      </header>

      {loading && !snapshot && (
        <div className="flex h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-border-subtle bg-surface">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-brand border-t-transparent" />
          <p className="mt-4 text-sm text-text-dim">Loading system status...</p>
        </div>
      )}

      {error && (
        <div className="rounded-xl border border-error/20 bg-error/5 p-4 text-sm text-error ring-1 ring-error/10">
          <div className="flex items-center gap-2 font-bold">
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 17c-.77 1.333.192 3 1.732 3z" /></svg>
            Error Loading Dashboard
          </div>
          <p className="mt-1 opacity-80">{error}</p>
        </div>
      )}

      {snapshot && (
        <>
          {/* Quick Stats Grid */}
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <button onClick={() => navigate({ to: "/agents" })} className={statCardClass}>
              <div className="absolute -right-4 -top-4 text-brand/5 transition-transform group-hover:scale-110 group-hover:text-brand/10">
                <svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg>
              </div>
              <div className="relative">
                <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">Active Agents</p>
                <div className="mt-2 flex items-baseline gap-2">
                  <span className="text-4xl font-black tracking-tight">{snapshot.status?.agent_count ?? 0}</span>
                  <span className="text-xs font-semibold text-success">{agentsActive} online</span>
                </div>
                <div className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-surface-hover shadow-inner">
                  <div className="h-full bg-brand shadow-[0_0_8px_var(--brand-color)]" style={{ width: `${Math.min(100, (agentsActive / (snapshot.status?.agent_count || 1)) * 100)}%` }} />
                </div>
              </div>
            </button>

            <button onClick={() => navigate({ to: "/canvas" })} className={statCardClass}>
              <div className="absolute -right-4 -top-4 text-accent/5 transition-transform group-hover:scale-110 group-hover:text-accent/10">
                <svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"/></svg>
              </div>
              <div className="relative">
                <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">Workflows</p>
                <div className="mt-2 flex items-baseline gap-2">
                  <span className="text-4xl font-black tracking-tight">n8n</span>
                  <span className="text-xs font-semibold text-accent">Active</span>
                </div>
                <p className="mt-4 text-[10px] text-text-dim group-hover:text-brand transition-colors italic font-medium">Design with visual canvas →</p>
              </div>
            </button>

            <button onClick={() => navigate({ to: "/providers" })} className={statCardClass}>
              <div className="absolute -right-4 -top-4 text-success/5 transition-transform group-hover:scale-110 group-hover:text-success/10">
                <svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/></svg>
              </div>
              <div className="relative">
                <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">Providers</p>
                <div className="mt-2 flex items-baseline gap-2">
                  <span className="text-4xl font-black tracking-tight">{providersReady}</span>
                  <span className="text-xs font-semibold text-text-dim">/ {snapshot.providers?.length ?? 0}</span>
                </div>
                <div className="mt-4 flex gap-1">
                  {(snapshot.providers || []).slice(0, 5).map((p, i) => (
                    <div key={i} title={p.name} className={`h-1.5 flex-1 rounded-full ${p.auth_status === 'configured' ? 'bg-success/50' : 'bg-surface-hover'}`} />
                  ))}
                </div>
              </div>
            </button>

            <button onClick={() => navigate({ to: "/channels" })} className={statCardClass}>
              <div className="absolute -right-4 -top-4 text-warning/5 transition-transform group-hover:scale-110 group-hover:text-warning/10">
                <svg className="h-24 w-24" fill="currentColor" viewBox="0 0 24 24"><circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/><line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/></svg>
              </div>
              <div className="relative">
                <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">Channels</p>
                <div className="mt-2 flex items-baseline gap-2">
                  <span className="text-4xl font-black tracking-tight">{channelsReady}</span>
                  <span className="text-xs font-semibold text-warning">Configured</span>
                </div>
                <p className="mt-4 text-[10px] text-text-dim font-medium">{snapshot.channels?.length ?? 0} adapters available</p>
              </div>
            </button>
          </div>

          <div className="grid gap-6 lg:grid-cols-3">
            {/* Quick Actions & System Info */}
            <div className="flex flex-col gap-6 lg:col-span-2">
              <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm">
                <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">Quick Actions</h3>
                <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
                  {[
                    { label: "New Workflow", to: "/canvas", icon: <path d="M12 5v14M5 12h14" />, primary: true },
                    { label: "Deploy Agent", to: "/agents", icon: <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" /> },
                    { label: "Open Chat", to: "/chat", icon: <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" /> },
                    { label: "Settings", to: "/settings", icon: <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33" /></> },
                  ].map((action, i) => (
                    <button
                      key={i}
                      onClick={() => navigate({ to: action.to as any })}
                      className={`flex flex-col items-center gap-2 rounded-xl border p-4 transition-all duration-200 ${
                        action.primary 
                          ? "border-brand/30 bg-brand-muted text-brand hover:bg-brand/20 shadow-sm" 
                          : "border-border-subtle bg-surface text-text-dim hover:border-brand/30 hover:bg-surface-hover hover:text-brand shadow-sm"
                      }`}
                    >
                      <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">{action.icon}</svg>
                      <span className="text-[11px] font-bold text-center">{action.label}</span>
                    </button>
                  ))}
                </div>
              </div>

              <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm">
                <div className="flex items-center justify-between">
                  <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">Recent Agents</h3>
                  <button onClick={() => navigate({ to: "/agents" })} className="text-xs font-bold text-brand hover:underline transition-all">View All →</button>
                </div>
                <div className="mt-4 grid gap-3 sm:grid-cols-2">
                  {(snapshot.agents || []).slice(0, 4).map((agent) => (
                    <div key={agent.id} className="flex items-center gap-3 rounded-xl border border-border-subtle bg-surface p-3 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
                      <div className={`flex h-10 w-10 items-center justify-center rounded-lg ${agent.status === 'running' ? 'bg-success/10 text-success' : 'bg-surface-hover text-text-dim'}`}>
                        <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" /></svg>
                      </div>
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-bold">{agent.name}</p>
                        <p className="truncate text-[10px] text-text-dim uppercase tracking-tight font-medium">{agent.id?.slice(0, 8)} • {agent.status}</p>
                      </div>
                      <div className={`h-1.5 w-1.5 rounded-full ${agent.status === 'running' ? 'bg-success animate-pulse shadow-[0_0_8px_var(--success-color)]' : 'bg-text-dim/30'}`} />
                    </div>
                  ))}
                  {(!snapshot.agents || snapshot.agents?.length === 0) && (
                    <div className="col-span-2 py-8 text-center text-text-dim border border-dashed border-border-subtle rounded-xl font-medium">No active agents configured</div>
                  )}
                </div>
              </div>
            </div>

            {/* Sidebar Stats: System & Health */}
            <div className="flex flex-col gap-6">
              <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm">
                <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">System Status</h3>
                <div className="mt-4 space-y-4">
                  <div className="flex justify-between border-b border-border-subtle pb-2">
                    <span className="text-xs text-text-dim font-medium">Uptime</span>
                    <span className="text-xs font-mono font-bold text-slate-700 dark:text-slate-200">{formatUptime(snapshot.status?.uptime_seconds)}</span>
                  </div>
                  <div className="flex justify-between border-b border-border-subtle pb-2">
                    <span className="text-xs text-text-dim font-medium">Memory</span>
                    <span className="text-xs font-mono font-bold text-slate-700 dark:text-slate-200">{snapshot.status?.memory_used_mb ? `${snapshot.status.memory_used_mb} MB` : "-"}</span>
                  </div>
                  <div className="flex justify-between border-b border-border-subtle pb-2">
                    <span className="text-xs text-text-dim font-medium">Version</span>
                    <span className="text-xs font-mono font-bold text-brand">{snapshot.status?.version ?? "-"}</span>
                  </div>
                </div>
              </div>

              <div className="rounded-2xl border border-border-subtle bg-surface p-6 backdrop-blur-sm shadow-sm">
                <h3 className="text-xs font-bold uppercase tracking-wider text-text-dim">Service Telemetry</h3>
                <div className="mt-4 space-y-3">
                  {snapshot.health?.checks?.map((check, i) => (
                    <div key={i} className="flex items-center gap-3">
                      <div className={`h-1.5 w-1.5 rounded-full ${check.status === "ok" ? "bg-success" : "bg-warning"}`} />
                      <span className="flex-1 text-xs font-medium text-slate-600 dark:text-slate-300">{check.name}</span>
                      <span className={`text-[10px] font-bold uppercase tracking-widest ${check.status === "ok" ? "text-success" : "text-warning"}`}>
                        {check.status === "ok" ? "OK" : "Alert"}
                      </span>
                    </div>
                  )) ?? <p className="text-xs text-text-dim italic">No telemetry data available</p>}
                </div>
              </div>

              {/* Pro Tip Card */}
              <div className="rounded-2xl bg-brand-muted border border-brand/5 p-6 shadow-sm relative overflow-hidden">
                <div className="absolute -right-4 -bottom-4 text-brand/10">
                  <svg className="h-16 w-16" fill="currentColor" viewBox="0 0 24 24"><path d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                </div>
                <div className="relative">
                  <div className="flex items-center gap-2 text-brand">
                    <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                    <span className="text-[10px] font-bold uppercase tracking-widest">Pro Tip</span>
                  </div>
                  <p className="mt-2 text-xs leading-relaxed text-slate-600 dark:text-slate-400 font-medium">
                    Use the <strong className="text-slate-900 dark:text-white">Canvas</strong> to orchestrate agents across multiple channels using visual flow builders.
                  </p>
                </div>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
