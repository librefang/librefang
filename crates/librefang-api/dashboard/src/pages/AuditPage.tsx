// Audit query viewer (RBAC M6 — stub).
//
// The dashboard data layer (`useAuditQuery` in `lib/queries/audit.ts`) is
// already wired against `/api/audit/query` so when M5 / #3203 ships the
// daemon endpoint, only this placeholder body needs to be replaced with
// the table. The hook is intentionally NOT enabled here so the page
// doesn't 404-spam the daemon while M5 is in flight.

import { useTranslation } from "react-i18next";
import { ScrollText, ExternalLink } from "lucide-react";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { useAuditQuery } from "../lib/queries/audit";

export function AuditPage() {
  const { t } = useTranslation();

  // Wire the hook in `enabled: false` mode so:
  //   1. The query layer is exercised by typecheck + tests (factory key
  //      stays anchored, types match the M5 endpoint shape we're committing
  //      to).
  //   2. We don't actually fire the request against a daemon that hasn't
  //      shipped #3203 yet, which would just return 404 noise.
  // The very moment M5 lands, drop `enabled: false` and render `data`.
  void useAuditQuery({ limit: 50 }, { enabled: false });

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<ScrollText className="h-4 w-4" />}
        title={t("audit.title", "Audit trail")}
        subtitle={t(
          "audit.subtitle",
          "Searchable, filterable audit log across users / actions / agents.",
        )}
        badge={t("audit.badge_pending", "Pending M5")}
      />

      <Card padding="lg">
        <div className="flex items-start gap-3">
          <ScrollText className="h-5 w-5 text-text-dim shrink-0" />
          <div>
            <p className="text-sm font-bold">
              {t(
                "audit.stub_title",
                "Audit query / export will activate when M5 (#3203) merges.",
              )}
            </p>
            <p className="mt-1 text-xs text-text-dim">
              {t(
                "audit.stub_body",
                "The dashboard query layer is wired and ready. The page consumes `useAuditQuery({...})` from `lib/queries/audit.ts`, keyed via the `auditKeys.query()` factory. When M5 ships `/api/audit/query`, drop the `enabled: false` guard in this component and render the table.",
              )}
            </p>
            <div className="mt-3 flex items-center gap-2 text-[11px]">
              <Badge variant="info">depends on librefang/librefang#3203</Badge>
              <a
                href="https://github.com/librefang/librefang/pull/3203"
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1 text-text-dim hover:text-brand"
              >
                <ExternalLink className="h-3 w-3" />
                {t("audit.view_pr", "View PR")}
              </a>
            </div>
          </div>
        </div>
      </Card>
    </div>
  );
}
