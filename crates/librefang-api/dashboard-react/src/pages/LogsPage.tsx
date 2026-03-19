import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { FileText } from "lucide-react";

const REFRESH_MS = 5000;

const LOG_LEVELS = {
  info: { color: "text-brand", bg: "bg-brand/10" },
  warn: { color: "text-warning", bg: "bg-warning/10" },
  error: { color: "text-error", bg: "bg-error/10" },
  debug: { color: "text-text-dim", bg: "bg-text-dim/10" },
};

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
      <PageHeader
        badge={t("common.status")}
        title={t("logs.title")}
        subtitle={t("logs.subtitle")}
        isFetching={snapshotQuery.isFetching}
        onRefresh={() => void snapshotQuery.refetch()}
        icon={<FileText className="h-4 w-4" />}
        actions={
          <button className="rounded-xl border border-border-subtle bg-surface px-4 py-2 text-xs font-bold text-text-dim hover:text-brand transition-all shadow-sm">
            {t("logs.export_json")}
          </button>
        }
      />

      <section className="flex-1 rounded-2xl border border-border-subtle bg-surface shadow-sm overflow-hidden">
        <div className="bg-main border-b border-border-subtle px-6 py-3 flex items-center justify-between text-[10px] font-black uppercase tracking-widest text-text-dim/60">
          <div className="flex gap-12"><span>{t("logs.timestamp")}</span><span>{t("logs.module")}</span><span>{t("logs.message")}</span></div>
        </div>
        <div className="p-4 font-mono text-xs space-y-1 max-h-[60vh] overflow-y-auto">
          {logs.map((l, i) => {
            const levelStyle = LOG_LEVELS[l.level as keyof typeof LOG_LEVELS] || LOG_LEVELS.info;
            return (
              <div key={i} className="flex gap-4 p-2 hover:bg-surface-hover rounded transition-colors items-center">
                <span className="text-text-dim/40 shrink-0 w-16">{l.time.split(' ')[1]}</span>
                <span className={`px-1.5 py-0.5 rounded text-[10px] font-black uppercase shrink-0 ${levelStyle.bg} ${levelStyle.color}`}>{l.level}</span>
                <span className="text-brand font-bold shrink-0 w-20">[{l.module}]</span>
                <span className="text-slate-700 dark:text-slate-300 truncate">{l.message}</span>
              </div>
            );
          })}
        </div>
      </section>
    </div>
  );
}
