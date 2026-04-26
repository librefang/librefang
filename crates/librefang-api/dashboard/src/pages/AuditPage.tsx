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
import type { AuditQueryFilters } from "../lib/http/client";

function buildExportUrl(
  filters: AuditQueryFilters,
  format: "csv" | "json",
): string {
  const params = new URLSearchParams({ format });
  for (const [k, v] of Object.entries(filters)) {
    if (v === undefined || v === null || v === "") continue;
    params.set(k, String(v));
  }
  return `/api/audit/export?${params.toString()}`;
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
  const query = useAuditQuery(active);

  const onApply = (e: React.FormEvent) => {
    e.preventDefault();
    setActive(draft);
  };

  const isForbidden =
    query.error instanceof Error && /403|admin/i.test(query.error.message);

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
          <a
            href={buildExportUrl(active, "csv")}
            className="inline-flex items-center gap-1.5 rounded border border-border px-2 py-1 text-xs hover:bg-surface-2"
            download
          >
            <Download className="h-3.5 w-3.5" />
            {t("audit.export_csv", "Export CSV")}
          </a>
        }
      />

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
            <Button type="submit" icon={<Search className="h-3.5 w-3.5" />}>
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
