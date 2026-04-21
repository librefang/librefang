import { formatTime } from "../lib/datetime";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { InlineEmpty } from "../components/ui/InlineEmpty";
import { FileText, Search, Download, Loader2 } from "lucide-react";
import { truncateId } from "../lib/string";
import { useAuditRecent } from "../lib/queries/runtime";
import type { AuditEntry } from "../api";

type LogEntry = AuditEntry & {
  source?: string;
  message?: string;
  event_type?: string;
  id?: string;
};

const REFRESH_MS = 5000;

const LOG_LEVELS = {
  info: { color: "text-brand", bg: "bg-brand/10" },
  warn: { color: "text-warning", bg: "bg-warning/10" },
  error: { color: "text-error", bg: "bg-error/10" },
  debug: { color: "text-text-dim", bg: "bg-text-dim/10" },
};

export function LogsPage() {
  const { t } = useTranslation();
  const LIMIT = 100;
  const auditQuery = useAuditRecent(LIMIT, {
    refetchInterval: REFRESH_MS, // Logs page polls faster so the live tail stays responsive.
  });

  const logs = auditQuery.data?.entries ?? [];
  const modules = useMemo(
    () => Array.from(new Set(logs.map((l: LogEntry) => l.action || l.source).filter(Boolean))) as string[],
    [logs],
  );
  const [search, setSearch] = useState("");
  const [moduleFilter, setModuleFilter] = useState<string | null>(null);

  const searchLower = useMemo(() => search.toLowerCase(), [search]);

  const filteredLogs = useMemo(
    () => logs.filter((l: LogEntry) => {
      const matchesSearch = !search || (l.detail || l.outcome || l.message || "").toLowerCase().includes(searchLower);
      const matchesModule = !moduleFilter || (l.action || l.source) === moduleFilter;
      return matchesSearch && matchesModule;
    }),
    [logs, searchLower, moduleFilter],
  );

  const handleExport = () => {
    const blob = new Blob([JSON.stringify(logs, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `audit-log-${new Date().toISOString().split("T")[0]}.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 100);
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.status")}
        title={t("logs.title")}
        subtitle={t("logs.subtitle")}
        isFetching={auditQuery.isFetching}
        onRefresh={() => void auditQuery.refetch()}
        icon={<FileText className="h-4 w-4" />}
        helpText={t("logs.help")}
        actions={
          <Button variant="secondary" size="sm" onClick={handleExport}>
            <Download className="h-3.5 w-3.5 mr-1" />
            {t("logs.export_json")}
          </Button>
        }
      />

      <Card padding="none" className="flex-1 overflow-hidden">
        <div className="bg-main border-b border-border-subtle px-3 sm:px-6 py-3 flex flex-col sm:flex-row items-stretch sm:items-center gap-2 sm:gap-4">
          <div className="flex-1">
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("common.search")}
              leftIcon={<Search className="h-4 w-4" />}
              className="py-1.5!"
              data-shortcut-search
            />
          </div>
          <select
            value={moduleFilter || ""}
            onChange={(e) => setModuleFilter(e.target.value || null)}
            className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-xs font-medium focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
          >
            <option value="">{t("logs.all_modules")}</option>
            {modules.map(m => <option key={m} value={m}>{m}</option>)}
          </select>
        </div>

        <div className="bg-main border-b border-border-subtle px-4 py-3 hidden sm:flex gap-4 items-center text-[10px] font-black uppercase tracking-widest text-text-dim/60">
          <span className="shrink-0 w-16">{t("logs.timestamp")}</span>
          <span className="shrink-0 w-14">{t("common.type")}</span>
          <span className="shrink-0 w-28">{t("logs.module")}</span>
          <span className="shrink-0 w-16">{t("logs.agent")}</span>
          <span className="flex-1">{t("logs.message")}</span>
        </div>
        <div className="p-2 sm:p-4 font-mono text-xs space-y-1 max-h-[60vh] overflow-y-auto scrollbar-thin">
          {auditQuery.isError ? (
            <InlineEmpty
              icon={<FileText className="w-5 h-5" />}
              message={t("common.error")}
            />
          ) : auditQuery.isLoading ? (
            <InlineEmpty
              icon={<Loader2 className="w-5 h-5 animate-spin" />}
              message={t("common.loading")}
            />
          ) : filteredLogs.length === 0 ? (
            <InlineEmpty
              icon={<FileText className="w-5 h-5" />}
              message={t("common.no_data")}
            />
          ) : (
            filteredLogs.map((l: LogEntry, i: number) => {
              const outcome = l.outcome || "";
              const isError = outcome.startsWith("error");
              const level = isError ? "error" : (l.event_type || "info").toLowerCase();
              const levelStyle = LOG_LEVELS[level as keyof typeof LOG_LEVELS] || LOG_LEVELS.info;
              const time = formatTime(l.timestamp);
              const detail = l.detail || l.message || "-";
              const reason = l.outcome && l.outcome !== detail ? l.outcome : "";
              const agentId = l.agent_id ? truncateId(l.agent_id) : "";
              return (
                <div key={l.seq || l.id || i} className="flex flex-col sm:flex-row gap-1 sm:gap-4 p-2 hover:bg-surface-hover rounded transition-colors items-start">
                  <div className="flex items-center gap-2 sm:contents">
                    <span className="text-text-dim/40 shrink-0 sm:w-16 text-[10px]">{time}</span>
                    <span className="shrink-0 sm:w-14"><span className={`px-1.5 py-0.5 rounded text-[10px] font-black uppercase ${levelStyle.bg} ${levelStyle.color}`}>{level}</span></span>
                    <span className="text-brand font-bold shrink-0 sm:w-28 truncate text-[10px]">{l.action || l.source || "-"}</span>
                    <span className="text-text-dim/40 font-mono shrink-0 sm:w-16 text-[9px] hidden sm:inline">{agentId || "-"}</span>
                  </div>
                  <div className="min-w-0 flex-1">
                    <span className="text-text-main text-[11px] break-all">{detail}</span>
                    {reason && (
                      <p className={`text-[10px] mt-0.5 break-all ${isError ? "text-error/70" : "text-text-dim/50"}`}>{reason}</p>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>
      </Card>
    </div>
  );
}
