import { useQuery } from "@tanstack/react-query";
import { loadDashboardSnapshot } from "../api";
import { useUIStore } from "../lib/store";

const REFRESH_MS = 30000;

export function SettingsPage() {
  const { theme, toggleTheme } = useUIStore();
  const snapshotQuery = useQuery({
    queryKey: ["dashboard", "snapshot", "settings"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const snapshot = snapshotQuery.data ?? null;

  const sectionClass = "rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5";
  const labelClass = "text-[10px] font-black uppercase tracking-widest text-text-dim mb-2 block";
  const infoRowClass = "flex justify-between items-center py-3 border-b border-border-subtle/30 last:border-0";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
            System Configuration
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Settings</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Manage your experience, API endpoints, and global kernel parameters.</p>
        </div>
      </header>

      <div className="grid gap-6 lg:grid-cols-2">
        <section className={sectionClass}>
          <h2 className="text-lg font-black tracking-tight mb-6">Appearance</h2>
          <div>
            <span className={labelClass}>Interface Theme</span>
            <div className="flex p-1 rounded-xl bg-main border border-border-subtle w-fit">
              <button 
                onClick={() => theme !== 'light' && toggleTheme()}
                className={`px-4 py-2 rounded-lg text-xs font-bold transition-all ${theme === 'light' ? 'bg-surface text-brand shadow-sm border border-border-subtle' : 'text-text-dim hover:text-slate-900 dark:hover:text-white'}`}
              >
                Light
              </button>
              <button 
                onClick={() => theme !== 'dark' && toggleTheme()}
                className={`px-4 py-2 rounded-lg text-xs font-bold transition-all ${theme === 'dark' ? 'bg-surface text-brand shadow-sm border border-border-subtle' : 'text-text-dim hover:text-slate-900 dark:hover:text-white'}`}
              >
                Dark
              </button>
            </div>
            <p className="mt-3 text-[10px] text-text-dim italic">System will remember your preference across sessions.</p>
          </div>
        </section>

        <section className={sectionClass}>
          <h2 className="text-lg font-black tracking-tight mb-6">Kernel Information</h2>
          <div className="space-y-1">
            <div className={infoRowClass}>
              <span className="text-xs font-bold text-text-dim">Daemon Version</span>
              <span className="font-mono text-xs font-black text-brand">{snapshot?.status.version || "0.6.0-stable"}</span>
            </div>
            <div className={infoRowClass}>
              <span className="text-xs font-bold text-text-dim">API Endpoint</span>
              <span className="font-mono text-xs text-slate-700 dark:text-slate-300">http://127.0.0.1:4545/api</span>
            </div>
            <div className={infoRowClass}>
              <span className="text-xs font-bold text-text-dim">Environment</span>
              <span className="px-2 py-0.5 rounded-lg bg-success/10 border border-success/20 text-[10px] font-black text-success uppercase">Production</span>
            </div>
          </div>
        </section>

        <section className={sectionClass}>
          <h2 className="text-lg font-black tracking-tight mb-6">Security</h2>
          <div className="space-y-4">
            <div>
              <span className={labelClass}>Default Provider</span>
              <div className="p-3 rounded-xl bg-main border border-border-subtle text-sm font-bold truncate">
                {snapshot?.status.default_provider || "No provider configured"}
              </div>
            </div>
            <div className="pt-2">
              <button className="text-xs font-bold text-error hover:underline transition-all">Clear local cache and tokens</button>
            </div>
          </div>
        </section>

        <section className="rounded-2xl bg-brand-muted border border-brand/5 p-6 flex flex-col justify-center">
          <h3 className="text-sm font-black uppercase tracking-widest text-brand mb-2">LibreFang Cloud</h3>
          <p className="text-xs text-text-dim leading-relaxed mb-4">
            Sync your agents, skills, and memory across multiple devices with our upcoming secure cloud backend.
          </p>
          <button className="w-fit rounded-xl bg-brand px-6 py-2 text-xs font-bold text-white shadow-lg shadow-brand/20 hover:opacity-90 transition-all">
            Join Waitlist
          </button>
        </section>
      </div>
    </div>
  );
}
