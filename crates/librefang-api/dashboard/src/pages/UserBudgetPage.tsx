// Per-user budget detail (RBAC M6 — stub for M5 / #3203).
//
// Reads the route param `name` and pre-wires `useUserBudget` against
// `/api/budget/users/{name}`. Activates after M5 merges; until then the
// component renders a placeholder so visitors don't see a perpetual
// loading spinner.

import { useTranslation } from "react-i18next";
import { useParams, Link } from "@tanstack/react-router";
import { Wallet, ExternalLink, ArrowLeft } from "lucide-react";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { useUserBudget } from "../lib/queries/userBudget";

export function UserBudgetPage() {
  const { t } = useTranslation();
  const { name } = useParams({ from: "/users/$name/budget" });

  // Disabled until M5 lands — see AuditPage for the same pattern.
  void useUserBudget(name, { enabled: false });

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<Wallet className="h-4 w-4" />}
        title={t("user_budget.title", "User budget")}
        subtitle={name}
        badge={t("user_budget.badge_pending", "Pending M5")}
        actions={
          <Link
            to="/users"
            className="inline-flex items-center gap-1.5 text-xs text-text-dim hover:text-brand"
          >
            <ArrowLeft className="h-3.5 w-3.5" />
            {t("user_budget.back", "Back to users")}
          </Link>
        }
      />

      <Card padding="lg">
        <div className="flex items-start gap-3">
          <Wallet className="h-5 w-5 text-text-dim shrink-0" />
          <div>
            <p className="text-sm font-bold">
              {t(
                "user_budget.stub_title",
                "Per-user budget charts will activate when M5 merges.",
              )}
            </p>
            <p className="mt-1 text-xs text-text-dim">
              {t(
                "user_budget.stub_body",
                "Hook ready: `useUserBudget(name)` in `lib/queries/userBudget.ts`, keyed via `userBudgetKeys.detail(name)`. Once M5 ships `/api/budget/users/{name}`, drop `enabled: false` and render the daily-spend chart + alert thresholds.",
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
                {t("user_budget.view_pr", "View PR")}
              </a>
            </div>
          </div>
        </div>
      </Card>
    </div>
  );
}
