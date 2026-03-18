import { useQuery } from "@tanstack/react-query";
import { loadDashboardSnapshot } from "../api";

const REFRESH_MS = 5000;

export function LogsPage() {
  const snapshotQuery = useQuery({
    queryKey: ["dashboard", "snapshot", "logs"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const logs = [
    { level: "info", time: "2026-03-18 10:07:22", module: "api", message: "Daemon started on port 4545" },
    { level: "info", time: "2026-03-18 10:07:23", module: "kernel", message: "Loaded 5 skill providers" },
    { level: "warn", time: "2026-03-18 10:08:05", module: "wire", message: "Retrying connection to Discord" },
    { level: "info", time: "2026-03-18 10:09:12", module: "runtime", message: "Agent 'Research-1' session created" },
    { level: "error", time: "2026-03-18 10:10:45", module: "openai", message: "API quota exceeded for model gpt-4o" },
  ];

  const levelColor = (level: string) => {
    switch (level) {
      case "error": return "text-error border-error/20 bg-error/5";
      case "warn": return "text-warning border-warning/20 bg-warning/5";
      default: return "text-success border-success/20 bg-success/5";
    }
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />
            </svg>
            System Telemetry
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Logs</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Real-time execution stream and diagnostic events from the kernel.</p>
        </div>
        <div className="flex gap-2">
          <button className="rounded-xl border border-border-subtle bg-surface px-4 py-2 text-xs font-bold text-text-dim hover:text-brand transition-all shadow-sm">
            Export JSON
          </button>
          <button
            className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm"
            onClick={() => void snapshotQuery.refetch()}
          >
            <svg className={`h-3.5 w-3.5 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
              <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            Refresh
          </button>
        </div>
      </header>

      <section className="flex-1 rounded-2xl border border-border-subtle bg-surface shadow-sm ring-1 ring-black/5 dark:ring-white/5 overflow-hidden">
        <div className="bg-main border-b border-border-subtle px-6 py-3 flex items-center justify-between">
          <div className="flex gap-4 text-[10px] font-black uppercase tracking-widest text-text-dim/60">
            <span>Timestamp</span>
            <span>Module</span>
            <span>Message</span>
          </div>
          <div className="h-2 w-2 rounded-full bg-brand animate-pulse" />
        </div>
        
        <div className="p-2 overflow-y-auto max-h-[600px] font-mono text-xs">
          {logs.map((log, i) => (
            <div key={i} className="group flex items-start gap-4 p-2 hover:bg-surface-hover rounded-lg transition-colors border border-transparent hover:border-border-subtle/30">
              <span className="text-text-dim/40 whitespace-nowrap">{log.time.split(' ')[1]}</span>
              <span className={`px-1.5 py-0.5 rounded border text-[10px] font-black uppercase tracking-tighter ${levelColor(log.level)}`}>
                {log.level}
              </span>
              <span className="text-brand font-bold whitespace-nowrap">[{log.module}]</span>
              <span className="text-slate-700 dark:text-slate-300 break-all">{log.message}</span>
            </div>
          ))}
        </div>
      </section>

      <div className="flex justify-center">
        <p className="text-[10px] font-bold text-text-dim/40 uppercase tracking-[0.2em]">End of buffer — Viewing last 100 events</p>
      </div>
    </div>
  );
}
