import { useQuery } from "@tanstack/react-query";
import { loadDashboardSnapshot } from "../api";

const REFRESH_MS = 30000;

export function AnalyticsPage() {
  const snapshotQuery = useQuery({
    queryKey: ["dashboard", "snapshot", "analytics"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const snapshot = snapshotQuery.data ?? null;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 20V10" /><path d="M12 20V4" /><path d="M6 20V14" />
            </svg>
            System Intelligence
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Analytics</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Usage statistics, token consumption, and performance telemetry.</p>
        </div>
        <button
          className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm"
          onClick={() => void snapshotQuery.refetch()}
        >
          <svg className={`h-3.5 w-3.5 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
            <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
          Refresh
        </button>
      </header>

      <div className="grid gap-6 md:grid-cols-2">
        <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight mb-1">Compute Usage</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">Resources consumed by inference providers.</p>
          
          <div className="space-y-4">
            {snapshot?.providers.map((p) => (
              <div key={p.id} className="group">
                <div className="flex justify-between mb-1 text-xs">
                  <span className="font-bold group-hover:text-brand transition-colors">{p.display_name || p.id}</span>
                  <span className="text-text-dim">{p.latency_ms ? `${p.latency_ms}ms avg` : "N/A"}</span>
                </div>
                <div className="h-2 w-full rounded-full bg-main overflow-hidden border border-border-subtle/30">
                  <div 
                    className="h-full bg-brand transition-all duration-1000" 
                    style={{ width: `${Math.min(100, (p.model_count || 0) * 10)}%` }} 
                  />
                </div>
              </div>
            ))}
            {!snapshot && <p className="text-xs text-text-dim italic">Awaiting telemetry data...</p>}
          </div>
        </section>

        <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight mb-1">Runtime Status</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">Core system execution metrics.</p>
          
          <div className="grid grid-cols-2 gap-4">
            <div className="p-4 rounded-xl bg-main border border-border-subtle/50">
              <p className="text-[10px] font-black text-text-dim uppercase tracking-wider mb-1">Active Agents</p>
              <p className="text-3xl font-black">{snapshot?.status.agent_count || 0}</p>
            </div>
            <div className="p-4 rounded-xl bg-main border border-border-subtle/50">
              <p className="text-[10px] font-black text-text-dim uppercase tracking-wider mb-1">Configured Channels</p>
              <p className="text-3xl font-black">{snapshot?.channels.length || 0}</p>
            </div>
            <div className="p-4 rounded-xl bg-main border border-border-subtle/50">
              <p className="text-[10px] font-black text-text-dim uppercase tracking-wider mb-1">Available Skills</p>
              <p className="text-3xl font-black text-accent">{snapshot?.skillCount || 0}</p>
            </div>
            <div className="p-4 rounded-xl bg-main border border-border-subtle/50">
              <p className="text-[10px] font-black text-text-dim uppercase tracking-wider mb-1">Health Checks</p>
              <p className="text-3xl font-black text-success">{snapshot?.health.checks?.length || 0}</p>
            </div>
          </div>
        </section>
      </div>

      <div className="rounded-2xl border border-dashed border-border-subtle p-12 text-center bg-surface/30">
        <div className="mx-auto h-12 w-12 rounded-full bg-brand/5 flex items-center justify-center text-brand mb-4">
          <svg className="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
            <path strokeLinecap="round" strokeLinejoin="round" d="M11 3.055A9.001 9.001 0 1020.945 13H11V3.055z" />
            <path strokeLinecap="round" strokeLinejoin="round" d="M20.488 9H15V3.512A9.025 9.001 0 0120.488 9z" />
          </svg>
        </div>
        <h3 className="text-lg font-black tracking-tight">Advanced Metrics</h3>
        <p className="text-sm text-text-dim mt-1">Detailed time-series analysis and cost tracking coming soon.</p>
      </div>
    </div>
  );
}
