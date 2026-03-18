import { useMutation, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { listAuditRecent, verifyAuditChain, type AuditEntry } from "../api";

const REFRESH_MS = 3000;

type LevelFilter = "" | "info" | "warn" | "error";
type LogsTab = "live" | "audit";

function classifyLevel(action?: string): Exclude<LevelFilter, ""> {
  if (!action) return "info";
  const value = action.toLowerCase();
  if (value.includes("error") || value.includes("fail") || value.includes("crash")) return "error";
  if (value.includes("warn") || value.includes("deny") || value.includes("block")) return "warn";
  return "info";
}

function levelClass(level: Exclude<LevelFilter, "">): string {
  if (level === "error") return "border-rose-700 bg-rose-700/20 text-rose-200";
  if (level === "warn") return "border-amber-700 bg-amber-700/20 text-amber-200";
  return "border-sky-700 bg-sky-700/20 text-sky-200";
}

function dateText(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function exportEntries(entries: AuditEntry[]) {
  const lines = entries.map((entry) => {
    const time = entry.timestamp ? dateText(entry.timestamp) : "-";
    const action = entry.action ?? "Unknown";
    const detail = entry.detail ?? "";
    const agent = entry.agent_id ?? "-";
    return `${time} [${action}] (${agent}) ${detail}`.trim();
  });

  const blob = new Blob([lines.join("\n")], { type: "text/plain" });
  const href = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.download = `librefang-logs-${new Date().toISOString().slice(0, 10)}.txt`;
  anchor.click();
  URL.revokeObjectURL(href);
}

export function LogsPage() {
  const [tab, setTab] = useState<LogsTab>("live");
  const [levelFilter, setLevelFilter] = useState<LevelFilter>("");
  const [textFilter, setTextFilter] = useState("");
  const [actionFilter, setActionFilter] = useState("");
  const [autoRefresh, setAutoRefresh] = useState(true);

  const logsQuery = useQuery({
    queryKey: ["audit", "recent", 200],
    queryFn: () => listAuditRecent(200),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const verifyMutation = useMutation({
    mutationFn: verifyAuditChain
  });

  const entries = logsQuery.data?.entries ?? [];
  const tipHash = logsQuery.data?.tip_hash ?? "";

  const filteredLiveEntries = useMemo(() => {
    const text = textFilter.trim().toLowerCase();
    return entries.filter((entry) => {
      if (levelFilter && classifyLevel(entry.action) !== levelFilter) return false;
      if (!text) return true;
      const haystack = `${entry.action ?? ""} ${entry.detail ?? ""} ${entry.agent_id ?? ""}`.toLowerCase();
      return haystack.includes(text);
    });
  }, [entries, levelFilter, textFilter]);

  const auditActions = useMemo(() => {
    return Array.from(new Set(entries.map((entry) => entry.action ?? "Unknown"))).sort((a, b) =>
      a.localeCompare(b)
    );
  }, [entries]);

  const filteredAuditEntries = useMemo(() => {
    if (!actionFilter) return entries;
    return entries.filter((entry) => (entry.action ?? "Unknown") === actionFilter);
  }, [entries, actionFilter]);

  const error = logsQuery.error instanceof Error ? logsQuery.error.message : "";
  const verifyError = verifyMutation.error instanceof Error ? verifyMutation.error.message : "";
  const verifyData = verifyMutation.data;

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Logs</h1>
          <p className="text-sm text-slate-400">Live audit feed, filtering, export, and chain integrity checks.</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void logsQuery.refetch()}
            disabled={logsQuery.isFetching}
          >
            Refresh
          </button>
          <button
            className={`rounded-lg border px-3 py-2 text-sm font-medium transition ${
              autoRefresh
                ? "border-emerald-600 bg-emerald-700/20 text-emerald-200 hover:bg-emerald-700/30"
                : "border-slate-600 bg-slate-800 text-slate-100 hover:border-sky-500 hover:bg-slate-700"
            }`}
            type="button"
            onClick={() => setAutoRefresh((current) => !current)}
          >
            {autoRefresh ? "Auto: On" : "Auto: Off"}
          </button>
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => exportEntries(filteredLiveEntries)}
            disabled={filteredLiveEntries.length === 0}
          >
            Export
          </button>
        </div>
      </header>

      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      <div className="flex items-center gap-2">
        <button
          className={`rounded-lg border px-3 py-2 text-sm ${
            tab === "live"
              ? "border-sky-500 bg-sky-600/20 text-sky-100"
              : "border-slate-700 bg-slate-900/70 text-slate-300 hover:border-slate-500"
          }`}
          onClick={() => setTab("live")}
        >
          Live
        </button>
        <button
          className={`rounded-lg border px-3 py-2 text-sm ${
            tab === "audit"
              ? "border-sky-500 bg-sky-600/20 text-sky-100"
              : "border-slate-700 bg-slate-900/70 text-slate-300 hover:border-slate-500"
          }`}
          onClick={() => setTab("audit")}
        >
          Audit
        </button>
      </div>

      {tab === "live" ? (
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <div className="grid gap-2 md:grid-cols-[180px_1fr]">
            <select
              value={levelFilter}
              onChange={(event) => setLevelFilter(event.target.value as LevelFilter)}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            >
              <option value="">All levels</option>
              <option value="info">Info</option>
              <option value="warn">Warn</option>
              <option value="error">Error</option>
            </select>
            <input
              value={textFilter}
              onChange={(event) => setTextFilter(event.target.value)}
              placeholder="Filter by action/detail/agent"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            />
          </div>

          {logsQuery.isLoading ? (
            <p className="mt-3 text-sm text-slate-400">Loading logs...</p>
          ) : filteredLiveEntries.length === 0 ? (
            <p className="mt-3 text-sm text-slate-400">No log entries match current filters.</p>
          ) : (
            <ul className="mt-3 flex max-h-[460px] list-none flex-col gap-2 overflow-y-auto p-0">
              {filteredLiveEntries.map((entry) => {
                const level = classifyLevel(entry.action);
                return (
                  <li
                    key={`${entry.seq ?? 0}-${entry.hash ?? ""}-${entry.timestamp ?? ""}`}
                    className="grid grid-cols-[auto_auto_1fr] items-start gap-2 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2"
                  >
                    <span className={`rounded-full border px-2 py-0.5 text-xs ${levelClass(level)}`}>{level}</span>
                    <span className="pt-0.5 text-xs text-slate-400">{dateText(entry.timestamp)}</span>
                    <div className="min-w-0">
                      <p className="m-0 truncate text-sm font-medium">{entry.action ?? "Unknown"}</p>
                      <p className="m-0 mt-1 break-words text-xs text-slate-300">{entry.detail ?? "-"}</p>
                      <p className="m-0 mt-1 text-xs text-slate-500">
                        agent {entry.agent_id ?? "-"} · seq {entry.seq ?? "-"}
                      </p>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </article>
      ) : (
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="text-sm text-slate-400">
              {filteredAuditEntries.length}/{entries.length} entries · tip{" "}
              <span className="font-mono text-xs">{tipHash ? `${tipHash.slice(0, 16)}...` : "-"}</span>
            </div>
            <button
              className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
              onClick={() => void verifyMutation.mutateAsync()}
              disabled={verifyMutation.isPending}
            >
              Verify Chain
            </button>
          </div>

          {verifyData ? (
            <div
              className={`mt-3 rounded-lg border p-3 text-sm ${
                verifyData.valid
                  ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
                  : "border-rose-700 bg-rose-700/10 text-rose-200"
              }`}
            >
              {verifyData.valid ? "Audit chain valid." : "Audit chain invalid."}
              {typeof verifyData.entries === "number" ? ` entries: ${verifyData.entries}.` : ""}
              {verifyData.warning ? ` ${verifyData.warning}` : ""}
              {verifyData.error ? ` ${verifyData.error}` : ""}
            </div>
          ) : null}
          {verifyError ? (
            <div className="mt-3 rounded-lg border border-rose-700 bg-rose-700/10 p-3 text-sm text-rose-200">
              {verifyError}
            </div>
          ) : null}

          <div className="mt-3">
            <select
              value={actionFilter}
              onChange={(event) => setActionFilter(event.target.value)}
              className="w-full rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            >
              <option value="">All actions</option>
              {auditActions.map((action) => (
                <option key={action} value={action}>
                  {action}
                </option>
              ))}
            </select>
          </div>

          {filteredAuditEntries.length === 0 ? (
            <p className="mt-3 text-sm text-slate-400">No audit entries.</p>
          ) : (
            <ul className="mt-3 flex max-h-[420px] list-none flex-col gap-2 overflow-y-auto p-0">
              {filteredAuditEntries.map((entry) => (
                <li
                  key={`${entry.seq ?? 0}-${entry.hash ?? ""}-${entry.timestamp ?? ""}`}
                  className="grid grid-cols-[auto_1fr] gap-3 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2 text-sm"
                >
                  <span className="text-xs text-slate-500">{entry.seq ?? "-"}</span>
                  <div className="min-w-0">
                    <p className="m-0 truncate font-medium">{entry.action ?? "Unknown"}</p>
                    <p className="m-0 mt-1 text-xs text-slate-400">{dateText(entry.timestamp)}</p>
                    <p className="m-0 mt-1 break-words text-xs text-slate-300">{entry.detail ?? "-"}</p>
                    <p className="m-0 mt-1 font-mono text-[11px] text-slate-500">{entry.hash ?? "-"}</p>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </article>
      )}
    </section>
  );
}
