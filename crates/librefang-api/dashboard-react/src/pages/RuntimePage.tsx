import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { loadDashboardSnapshot } from "../api";

const REFRESH_MS = 30000;

export function RuntimePage() {
  const { t } = useTranslation();
  const snapshotQuery = useQuery({ queryKey: ["dashboard", "snapshot", "runtime"], queryFn: loadDashboardSnapshot, refetchInterval: REFRESH_MS });
  const snapshot = snapshotQuery.data ?? null;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M22 12h-4l-3 9L9 3l-3 9H2" /></svg>
            {t("runtime.kernel")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("runtime.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("runtime.subtitle")}</p>
        </div>
        <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand shadow-sm" onClick={() => void snapshotQuery.refetch()}>
          {t("common.refresh")}
        </button>
      </header>

      <div className="grid gap-6 md:grid-cols-2">
        <section className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
          <h2 className="text-lg font-black tracking-tight mb-6">{t("runtime.environment")}</h2>
          <div className="space-y-4">
            <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
              <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.engine_version")}</span>
              <span className="font-mono text-sm font-black text-brand">{snapshot?.status.version || t("common.unknown")}</span>
            </div>
            <div className="flex justify-between items-center py-2 border-b border-border-subtle/30">
              <span className="text-xs font-bold text-text-dim uppercase">{t("runtime.system_uptime")}</span>
              <span className="text-sm font-black">{snapshot?.status.uptime_seconds || "-"}s</span>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
