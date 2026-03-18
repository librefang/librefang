import { useQuery } from "@tanstack/react-query";
import { listChannels, loadDashboardSnapshot } from "../api";

const REFRESH_MS = 30000;

export function CommsPage() {
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

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <polyline points="22 12 18 12 15 21 9 3 6 12 2 12" />
            </svg>
            Communication Bus
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Comms</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Monitor real-time message traffic and adapter connectivity health.</p>
        </div>
        <button
          className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm"
          onClick={() => { void channelsQuery.refetch(); void snapshotQuery.refetch(); }}
        >
          <svg className={`h-3.5 w-3.5 ${channelsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
            <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
          Refresh
        </button>
      </header>

      <div className="grid gap-6 lg:grid-cols-2">
        <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight mb-1">Active Channels</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">Connectivity status of external messaging adapters.</p>
          
          <div className="space-y-3">
            {channels.map((c) => (
              <div key={c.id} className="flex items-center justify-between p-3 rounded-xl bg-main/40 border border-border-subtle/50">
                <div className="flex items-center gap-3">
                  <div className={`h-2 w-2 rounded-full ${c.configured ? 'bg-success shadow-[0_0_8px_var(--success-color)]' : 'bg-text-dim/30'}`} />
                  <span className="text-sm font-bold">{c.display_name || c.id}</span>
                </div>
                <span className={`text-[10px] font-black uppercase tracking-widest ${c.configured ? 'text-success' : 'text-text-dim'}`}>
                  {c.configured ? "Online" : "Unconfigured"}
                </span>
              </div>
            ))}
            {channels.length === 0 && <p className="text-xs text-text-dim italic py-4 text-center">No channels discovered.</p>}
          </div>
        </section>

        <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight mb-1">System Health</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">Telemetric health checks for critical sub-systems.</p>
          
          <div className="grid gap-3">
            {snapshot?.health.checks?.map((check, i) => (
              <div key={i} className="flex items-center justify-between p-3 rounded-xl bg-main/40 border border-border-subtle/50">
                <span className="text-sm font-bold">{check.name}</span>
                <div className={`px-2 py-0.5 rounded-lg border text-[10px] font-black uppercase tracking-widest ${check.status === 'ok' ? 'border-success/20 bg-success/10 text-success' : 'border-error/20 bg-error/10 text-error'}`}>
                  {check.status === 'ok' ? 'Nominal' : 'Check'}
                </div>
              </div>
            )) ?? <p className="text-xs text-text-dim italic py-4 text-center">Awaiting system telemetry...</p>}
          </div>
        </section>
      </div>

      <div className="rounded-2xl bg-brand-muted border border-brand/5 p-8 shadow-sm">
        <h3 className="text-sm font-black uppercase tracking-widest text-brand mb-2">Network Topology</h3>
        <p className="text-xs text-text-dim leading-relaxed max-w-xl">
          LibreFang uses a high-performance event bus to coordinate messages between multiple agents and external channels. All data is end-to-end encrypted when transiting unverified networks.
        </p>
      </div>
    </div>
  );
}
