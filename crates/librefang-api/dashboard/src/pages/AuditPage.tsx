// Audit-trail viewer (RBAC M5 + M6).
//
// Admin-only. Filters narrow the in-memory window (server hard cap 5000
// rows, default 200) — for deeper history use the export button which hits
// /api/audit/export with the same filter set.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ScrollText, Download, AlertTriangle, Search } from "lucide-react";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { useAuditQuery } from "../lib/queries/audit";
import { ApiError } from "../lib/http/errors";
import type { AuditQueryFilters } from "../lib/http/client";

// `<input type="datetime-local">` produces "YYYY-MM-DDTHH:MM" with no
// timezone. The server parses `from` / `to` as RFC-3339 (offset
// required), so we must normalise to ISO-8601 with `Z` before sending
// — otherwise the server returns 400 and the filter silently fails.
// Treats the input as the user's local time (matches what the picker
// displays) and converts to UTC.
function toRfc3339(local: string | undefined): string | undefined {
  if (!local) return undefined;
  const d = new Date(local);
  if (Number.isNaN(d.getTime())) return undefined;
  return d.toISOString();
}

function normaliseFilters(filters: AuditQueryFilters): AuditQueryFilters {
  return {
    ...filters,
    from: toRfc3339(filters.from),
    to: toRfc3339(filters.to),
  };
}

function buildExportUrl(
  filters: AuditQueryFilters,
  format: "csv" | "json",
): string {
  const normalised = normaliseFilters(filters);
  const params = new URLSearchParams({ format });
  for (const [k, v] of Object.entries(normalised)) {
    if (v === undefined || v === null || v === "") continue;
    params.set(k, String(v));
  }
  return `/api/audit/export?${params.toString()}`;
}

// Authenticated download: dashboard auth is Bearer-in-header, but
// `<a download>` triggers a navigation that drops custom headers, so
// the browser would download the daemon's 401 / login HTML as
// `audit.csv`. Fetch with the Bearer header, materialise the body as
// a Blob, then programmatically click an object-URL anchor.
async function downloadExport(
  filters: AuditQueryFilters,
  format: "csv" | "json",
): Promise<void> {
  const url = buildExportUrl(filters, format);
  const token = localStorage.getItem("librefang-api-key") || "";
  const headers: Record<string, string> = {};
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const resp = await fetch(url, { headers });
  if (!resp.ok) {
    throw await ApiError.fromResponse(resp);
  }
  const blob = await resp.blob();
  const objectUrl = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = objectUrl;
  a.download = `audit.${format}`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Defer revoke so the browser has a chance to start the save dialog.
  setTimeout(() => URL.revokeObjectURL(objectUrl), 1000);
}

const ACTION_OPTIONS = [
  "",
  "ToolInvoke",
  "ShellExec",
  "UserLogin",
  "RoleChange",
  "PermissionDenied",
  "BudgetExceeded",
  "ConfigChange",
];

export function AuditPage() {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<AuditQueryFilters>({ limit: 200 });
  const [active, setActive] = useState<AuditQueryFilters>({ limit: 200 });
  const [exportError, setExportError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  // Normalise from/to so the server's RFC-3339 parser doesn't 400 on
  // the bare datetime-local format. Same for export URL.
  const query = useAuditQuery(normaliseFilters(active));

  const onApply = (e: React.FormEvent) => {
    e.preventDefault();
    setActive(draft);
  };

  const onExport = async () => {
    setExportError(null);
    setExporting(true);
    try {
      await downloadExport(active, "csv");
    } catch (err) {
      setExportError(
        err instanceof ApiError
          ? `${err.status}: ${err.message}`
          : err instanceof Error
            ? err.message
            : String(err),
      );
    } finally {
      setExporting(false);
    }
  };

  // Status-code check, not text-matching the message: the server's
  // forbidden body is "Admin role required for audit access" today
  // but a future copy edit shouldn't silently regress this banner.
  const isForbidden = query.error instanceof ApiError && query.error.status === 403;

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<ScrollText className="h-4 w-4" />}
        title={t("audit.title", "Audit trail")}
        subtitle={t(
          "audit.subtitle",
          "Searchable, filterable audit log across users / actions / agents.",
        )}
        actions={
          <button
            type="button"
            onClick={onExport}
            disabled={exporting}
            className="inline-flex items-center gap-1.5 rounded border border-border px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-50"
          >
            <Download className="h-3.5 w-3.5" />
            {exporting
              ? t("audit.exporting", "Exporting…")
              : t("audit.export_csv", "Export CSV")}
          </button>
        }
      />

      {exportError && (
        <Card padding="lg">
          <div className="flex items-start gap-3 text-sm text-error">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <div>
              <p className="font-bold">
                {t("audit.export_error_title", "Export failed")}
              </p>
              <p className="mt-1 text-xs">{exportError}</p>
            </div>
          </div>
        </Card>
      )}

      <Card padding="lg">
        <form onSubmit={onApply} className="grid grid-cols-3 gap-3 text-xs">
          <label className="flex flex-col gap-1">
            <span className="text-text-dim">{t("audit.f_user", "User")}</span>
            <input
              value={draft.user ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, user: e.target.value || undefined }))
              }
              placeholder="UUID or name"
              className="rounded border border-border bg-surface-2 px-2 py-1"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-text-dim">
              {t("audit.f_action", "Action")}
            </span>
            <select
              value={draft.action ?? ""}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  action: e.target.value || undefined,
                }))
              }
              className="rounded border border-border bg-surface-2 px-2 py-1"
            >
              {ACTION_OPTIONS.map((a) => (
                <option key={a} value={a}>
                  {a || t("audit.f_any", "(any)")}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-text-dim">{t("audit.f_agent", "Agent")}</span>
            <input
              value={draft.agent ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, agent: e.target.value || undefined }))
              }
              className="rounded border border-border bg-surface-2 px-2 py-1"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-text-dim">
              {t("audit.f_channel", "Channel")}
            </span>
            <input
              value={draft.channel ?? ""}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  channel: e.target.value || undefined,
                }))
              }
              placeholder="api / telegram / ..."
              className="rounded border border-border bg-surface-2 px-2 py-1"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-text-dim">
              {t("audit.f_from", "From (ISO-8601)")}
            </span>
            <input
              type="datetime-local"
              value={draft.from ?? ""}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  from: e.target.value || undefined,
                }))
              }
              className="rounded border border-border bg-surface-2 px-2 py-1"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-text-dim">
              {t("audit.f_to", "To (ISO-8601)")}
            </span>
            <input
              type="datetime-local"
              value={draft.to ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, to: e.target.value || undefined }))
              }
              className="rounded border border-border bg-surface-2 px-2 py-1"
            />
          </label>
          <div className="col-span-3 flex justify-end">
            <Button type="submit" leftIcon={<Search className="h-3.5 w-3.5" />}>
              {t("audit.apply", "Apply filters")}
            </Button>
          </div>
        </form>
      </Card>

      {isForbidden && (
        <Card padding="lg">
          <div className="flex items-start gap-3 text-sm text-error">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <div>
              <p className="font-bold">
                {t("audit.forbidden_title", "Admin role required")}
              </p>
              <p className="mt-1 text-xs">
                {t(
                  "audit.forbidden_body",
                  "/api/audit/query is admin-only. Sign in with an Admin or Owner api_key.",
                )}
              </p>
            </div>
          </div>
        </Card>
      )}

      {!isForbidden && query.error && (
        <Card padding="lg">
          <div className="flex items-start gap-3 text-sm text-error">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <div>
              <p className="font-bold">
                {t("audit.error_title", "Failed to load audit log")}
              </p>
              <p className="mt-1 text-xs">{String(query.error)}</p>
            </div>
          </div>
        </Card>
      )}

      <Card padding="lg">
        {query.isLoading && (
          <p className="text-sm text-text-dim">
            {t("audit.loading", "Loading…")}
          </p>
        )}
        {query.data && (
          <>
            <p className="text-xs text-text-dim mb-3">
              {t("audit.count_label", "Showing {{count}} of up to {{limit}}", {
                count: query.data.count,
                limit: query.data.limit,
              })}
            </p>
            <div className="overflow-x-auto">
              <table className="w-full text-xs">
                <thead>
                  <tr className="text-left text-text-dim">
                    <th className="px-2 py-1">seq</th>
                    <th className="px-2 py-1">timestamp</th>
                    <th className="px-2 py-1">action</th>
                    <th className="px-2 py-1">agent</th>
                    <th className="px-2 py-1">user</th>
                    <th className="px-2 py-1">channel</th>
                    <th className="px-2 py-1">outcome</th>
                    <th className="px-2 py-1">detail</th>
                  </tr>
                </thead>
                <tbody>
                  {query.data.entries.map((e) => (
                    <tr key={`${e.seq}-${e.hash}`} className="border-t border-border/50">
                      <td className="px-2 py-1 font-mono">{e.seq}</td>
                      <td className="px-2 py-1 font-mono">{e.timestamp}</td>
                      <td className="px-2 py-1">{e.action}</td>
                      <td className="px-2 py-1 font-mono">{e.agent_id}</td>
                      <td className="px-2 py-1 font-mono">{e.user_id ?? "—"}</td>
                      <td className="px-2 py-1">{e.channel ?? "—"}</td>
                      <td
                        className={`px-2 py-1 ${
                          e.outcome === "denied" || e.outcome === "error"
                            ? "text-error"
                            : ""
                        }`}
                      >
                        {e.outcome}
                      </td>
                      <td className="px-2 py-1 text-text-dim">{e.detail}</td>
                    </tr>
                  ))}
                  {query.data.entries.length === 0 && (
                    <tr>
                      <td
                        colSpan={8}
                        className="px-2 py-4 text-center text-text-dim"
                      >
                        {t("audit.empty", "No matching audit entries")}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </>
        )}
      </Card>
    </div>
  );
}
