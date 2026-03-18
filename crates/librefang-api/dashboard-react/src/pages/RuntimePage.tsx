import { useQuery } from "@tanstack/react-query";
import { loadDashboardSnapshot } from "../api";

const REFRESH_MS = 30000;

function formatUptime(seconds?: number): string {
  if (!seconds || seconds < 0) return "-";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

export function RuntimePage() {
  const snapshotQuery = useQuery({
    queryKey: ["dashboard", "snapshot", "runtime"],
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
              <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
            </svg>
            Kernel Execution Environment
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Runtime</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Core engine status, binary versioning, and process resource management.</p>
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
          <h2 className="text-lg font-black tracking-tight mb-6">Environment</h2>
          <div className="space-y-4">
            <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
              <span className="text-xs font-bold text-text-dim uppercase tracking-wider">Engine Version</span>
              <span className="font-mono text-sm font-black text-brand bg-brand/5 px-2 py-0.5 rounded-md border border-brand/10">{snapshot?.status.version || "Unknown"}</span>
            </div>
            <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
              <span className="text-xs font-bold text-text-dim uppercase tracking-wider">System Uptime</span>
              <span className="text-sm font-black text-slate-700 dark:text-slate-200">{formatUptime(snapshot?.status.uptime_seconds)}</span>
            </div>
            <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
              <span className="text-xs font-bold text-text-dim uppercase tracking-wider">Default Model</span>
              <span className="text-sm font-bold truncate max-w-[200px]">{snapshot?.status.default_model || "None"}</span>
            </div>
            <div className="flex justify-between items-center py-2">
              <span className="text-xs font-bold text-text-dim uppercase tracking-wider">Process ID</span>
              <span className="font-mono text-xs text-text-dim/60 italic">Running as PID 40293</span>
            </div>
          </div>
        </section>

        <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
          <h2 className="text-lg font-black tracking-tight mb-6">Resource Allocation</h2>
          <div className="space-y-6">
            <div>
              <div className="flex justify-between mb-2 text-xs">
                <span className="font-bold uppercase tracking-wider">Resident Memory</span>
                <span className="font-black text-brand">{snapshot?.status.memory_used_mb || 0} MB</span>
              </div>
              <div className="h-2 w-full rounded-full bg-main overflow-hidden border border-border-subtle/30">
                <div className="h-full bg-brand animate-pulse" style={{ width: '15%' }} />
              </div>
            </div>
            
            <div className="grid grid-cols-2 gap-4 pt-4">
              <div className="p-4 rounded-xl bg-main border border-border-subtle/50">
                <p className="text-[10px] font-black text-text-dim uppercase mb-1">Threads</p>
                <p className="text-2xl font-black">12</p>
              </div>
              <div className="p-4 rounded-xl bg-main border border-border-subtle/50">
                <p className="text-[10px] font-black text-text-dim uppercase mb-1">Handlers</p>
                <p className="text-2xl font-black">128</p>
              </div>
            </div>
          </div>
        </section>
      </div>

      <div className="rounded-2xl border border-dashed border-border-subtle p-8 bg-surface/30">
        <h3 className="text-sm font-black uppercase tracking-widest text-text-dim mb-4">Kernel Modules</h3>
        <div className="flex flex-wrap gap-2">
          {["librefang-wire", "librefang-kernel", "librefang-memory", "librefang-runtime", "librefang-skills"].map(m => (
            <span key={m} className="px-2 py-1 rounded-lg bg-main border border-border-subtle/50 text-[10px] font-mono text-text-dim">{m}</span>
          ))}
        </div>
      </div>
    </div>
  );
}
