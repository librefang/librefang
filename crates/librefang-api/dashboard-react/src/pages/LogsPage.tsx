import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot } from "../api";

const REFRESH_MS = 5000;

export function LogsPage() {
  const { t } = useTranslation();
  const snapshotQuery = useQuery({ queryKey: ["dashboard", "snapshot", "logs"], queryFn: loadDashboardSnapshot, refetchInterval: REFRESH_MS });

  const logs = [
    { level: "info", time: "2026-03-18 10:07:22", module: "api", message: "Daemon started on port 4545" },
    { level: "info", time: "2026-03-18 10:07:23", module: "kernel", message: "Loaded 5 skill providers" },
    { level: "warn", time: "2026-03-18 10:08:05", module: "wire", message: "Retrying connection to Discord" },
    { level: "info", time: "2026-03-18 10:09:12", module: "runtime", message: "Agent 'Research-1' session created" },
    { level: "error", time: "2026-03-18 10:10:45", module: "openai", message: "API quota exceeded for model gpt-4o" },
  ];

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M4 6h16M4 12h16M4 18h16" /></svg>
            {t("common.status")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("logs.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("logs.subtitle")}</p>
        </div>
        <div className="flex gap-2">
          <button className="rounded-xl border border-border-subtle bg-surface px-4 py-2 text-xs font-bold text-text-dim hover:text-brand transition-all shadow-sm">{t("logs.export_json")}</button>
          <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void snapshotQuery.refetch()}>
            {t("common.refresh")}
          </button>
        </div>
      </header>

      <section className="flex-1 rounded-2xl border border-border-subtle bg-surface shadow-sm overflow-hidden">
        <div className="bg-main border-b border-border-subtle px-6 py-3 flex items-center justify-between text-[10px] font-black uppercase tracking-widest text-text-dim/60">
          <div className="flex gap-12"><span>{t("logs.timestamp")}</span><span>{t("logs.module")}</span><span>{t("logs.message")}</span></div>
        </div>
        <div className="p-4 font-mono text-xs space-y-2">
          {logs.map((l, i) => (
            <div key={i} className="flex gap-4 p-1 hover:bg-surface-hover rounded transition-colors">
              <span className="text-text-dim/40">{l.time.split(' ')[1]}</span>
              <span className="text-brand font-bold">[{l.module}]</span>
              <span className="text-slate-700 dark:text-slate-300">{l.message}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
