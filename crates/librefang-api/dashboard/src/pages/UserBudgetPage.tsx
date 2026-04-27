// Per-user budget detail (RBAC M5).
//
// Shows the user's current spend vs cap across the three windows the
// metering pipeline enforces (hourly / daily / monthly), and lets an admin
// upsert or clear the cap. The page assumes Admin+ — anything below gets
// 403'd by the in-handler `require_admin_for_user_budget` gate before this
// loads.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useParams, Link } from "@tanstack/react-router";
import { Wallet, ArrowLeft, AlertTriangle, Check } from "lucide-react";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { useUserBudget } from "../lib/queries/userBudget";
import {
  useUpdateUserBudget,
  useDeleteUserBudget,
} from "../lib/mutations/userBudget";

interface FormState {
  max_hourly_usd: string;
  max_daily_usd: string;
  max_monthly_usd: string;
  alert_threshold: string;
}

const ZERO_FORM: FormState = {
  max_hourly_usd: "0",
  max_daily_usd: "0",
  max_monthly_usd: "0",
  alert_threshold: "0.8",
};

export function UserBudgetPage() {
  const { t } = useTranslation();
  const { name } = useParams({ from: "/users/$name/budget" });
  const query = useUserBudget(name);
  const updateMut = useUpdateUserBudget();
  const deleteMut = useDeleteUserBudget();

  const [form, setForm] = useState<FormState>(ZERO_FORM);
  const [error, setError] = useState<string | null>(null);

  // Seed the form from the current limits whenever they refresh. Spend
  // values are display-only; we only ever sync `limit` / `alert_threshold`.
  useEffect(() => {
    if (!query.data) return;
    setForm({
      max_hourly_usd: String(query.data.hourly.limit),
      max_daily_usd: String(query.data.daily.limit),
      max_monthly_usd: String(query.data.monthly.limit),
      alert_threshold: String(query.data.alert_threshold),
    });
  }, [query.data]);

  const isLoading = query.isLoading;
  const fetchError = query.error;

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    const payload = {
      max_hourly_usd: parseFloat(form.max_hourly_usd),
      max_daily_usd: parseFloat(form.max_daily_usd),
      max_monthly_usd: parseFloat(form.max_monthly_usd),
      alert_threshold: parseFloat(form.alert_threshold),
    };

    for (const [k, v] of Object.entries(payload)) {
      if (Number.isNaN(v) || !Number.isFinite(v) || v < 0) {
        setError(
          t(
            "userBudget.errors.non_negative",
            "{{field}} must be a finite, non-negative number",
            { field: k },
          ),
        );
        return;
      }
    }
    if (payload.alert_threshold > 1) {
      setError(
        t(
          "userBudget.errors.threshold_range",
          "alert_threshold must be in 0.0..=1.0",
        ),
      );
      return;
    }

    try {
      await updateMut.mutateAsync({ name, payload });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const onClear = async () => {
    setError(null);
    try {
      await deleteMut.mutateAsync(name);
      setForm(ZERO_FORM);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<Wallet className="h-4 w-4" />}
        title={t("user_budget.title", "User budget")}
        subtitle={name}
        badge={
          query.data?.alert_breach ? (
            <Badge variant="warning">
              {t("user_budget.alert_breach", "alert breach")}
            </Badge>
          ) : query.data?.enforced ? (
            <Badge variant="success">
              {t("user_budget.enforced", "enforced")}
            </Badge>
          ) : (
            <Badge variant="info">
              {t("user_budget.deferred", "enforcement deferred")}
            </Badge>
          )
        }
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

      {fetchError && (
        <Card padding="lg">
          <div className="flex items-start gap-3 text-sm text-error">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <div>
              <p className="font-bold">
                {t("user_budget.fetch_error", "Failed to load budget")}
              </p>
              <p className="mt-1 text-xs">{String(fetchError)}</p>
            </div>
          </div>
        </Card>
      )}

      {isLoading && (
        <Card padding="lg">
          <p className="text-sm text-text-dim">
            {t("user_budget.loading", "Loading…")}
          </p>
        </Card>
      )}

      {query.data && (
        <Card padding="lg">
          <h2 className="text-sm font-bold mb-3">
            {t("user_budget.current_spend", "Current spend (USD)")}
          </h2>
          <div className="grid grid-cols-3 gap-4">
            {(["hourly", "daily", "monthly"] as const).map((w) => {
              const win = query.data![w];
              const breached =
                win.limit > 0 && win.pct >= query.data!.alert_threshold;
              return (
                <div key={w} className="text-sm">
                  <div className="text-xs text-text-dim uppercase">
                    {t(`user_budget.window_${w}`, w)}
                  </div>
                  <div
                    className={`mt-1 font-mono ${
                      breached ? "text-error" : ""
                    }`}
                  >
                    ${win.spend.toFixed(4)}{" "}
                    <span className="text-text-dim">
                      / {win.limit > 0 ? `$${win.limit.toFixed(2)}` : "∞"}
                    </span>
                  </div>
                  {win.limit > 0 && (
                    <div className="mt-1 h-1 w-full bg-surface-2 rounded">
                      <div
                        className={`h-1 rounded ${
                          breached ? "bg-error" : "bg-brand"
                        }`}
                        style={{
                          width: `${Math.min(100, win.pct * 100).toFixed(1)}%`,
                        }}
                      />
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </Card>
      )}

      <Card padding="lg">
        <h2 className="text-sm font-bold mb-3">
          {t("user_budget.set_limits", "Set spend limits")}
        </h2>
        <p className="text-xs text-text-dim mb-4">
          {t(
            "user_budget.zero_means_unlimited",
            "Set any window to 0 for unlimited on that window. Threshold is the fraction of any limit at which a BudgetExceeded audit fires.",
          )}
        </p>
        <form onSubmit={onSubmit} className="grid grid-cols-2 gap-4">
          {(
            [
              [
                "max_hourly_usd",
                t("userBudget.fields.max_hourly", "Max hourly USD"),
              ],
              [
                "max_daily_usd",
                t("userBudget.fields.max_daily", "Max daily USD"),
              ],
              [
                "max_monthly_usd",
                t("userBudget.fields.max_monthly", "Max monthly USD"),
              ],
              [
                "alert_threshold",
                t(
                  "userBudget.fields.alert_threshold",
                  "Alert threshold (0–1)",
                ),
              ],
            ] as const
          ).map(([key, label]) => (
            <label key={key} className="flex flex-col gap-1 text-xs">
              <span className="text-text-dim">{label}</span>
              <input
                type="number"
                step="0.01"
                min="0"
                value={form[key]}
                onChange={(e) =>
                  setForm((f) => ({ ...f, [key]: e.target.value }))
                }
                className="rounded border border-border bg-surface-2 px-2 py-1 font-mono"
              />
            </label>
          ))}
          {error && (
            <div className="col-span-2 text-xs text-error flex items-center gap-2">
              <AlertTriangle className="h-3.5 w-3.5" />
              {error}
            </div>
          )}
          <div className="col-span-2 flex items-center gap-2 mt-2">
            <Button
              type="submit"
              disabled={updateMut.isPending}
              leftIcon={<Check className="h-3.5 w-3.5" />}
            >
              {updateMut.isPending
                ? t("user_budget.saving", "Saving…")
                : t("user_budget.save", "Save")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={onClear}
              disabled={deleteMut.isPending}
            >
              {deleteMut.isPending
                ? t("user_budget.clearing", "Clearing…")
                : t("user_budget.clear", "Clear cap")}
            </Button>
          </div>
        </form>
      </Card>
    </div>
  );
}
