// Per-user budget detail (RBAC M5).
//
// Shows the user's current spend vs cap across the three windows the
// metering pipeline enforces (hourly / daily / monthly), and lets an admin
// upsert or clear the cap. The page assumes Admin+ — anything below gets
// 403'd by the in-handler `require_admin_for_user_budget` gate before this
// loads.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useParams, Link } from "@tanstack/react-router";
import {
  Wallet,
  ArrowLeft,
  AlertTriangle,
  Check,
  Clock,
  Calendar,
  CalendarDays,
  Bell,
  Trash2,
} from "lucide-react";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { useUserBudget } from "../lib/queries/userBudget";
import {
  useUpdateUserBudget,
  useDeleteUserBudget,
} from "../lib/mutations/userBudget";
import { useUIStore } from "../lib/store";

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
  const addToast = useUIStore((s) => s.addToast);

  // One-shot seed guard. React Query refetches on window focus and on
  // mutation invalidation; without this, the form would clobber any
  // in-progress edits every time `query.data` swapped reference.
  const hasSeeded = useRef(false);
  // Bumped after a successful save / clear so the next `query.data`
  // delivery is allowed to reseed (so the form reflects the normalized
  // server state, not the raw input we just sent).
  const [lastSavedAt, setLastSavedAt] = useState(0);
  const lastSeededSavedAt = useRef(0);

  // Seed once on first successful load; reseed only after a save we
  // initiated (tracked via `lastSavedAt`). All other refetches —
  // window-focus revalidations, sibling-mutation invalidations — leave
  // the form alone so the operator's edits survive.
  useEffect(() => {
    if (!query.data) return;
    const justSaved =
      lastSavedAt > 0 && lastSavedAt !== lastSeededSavedAt.current;
    if (hasSeeded.current && !justSaved) return;
    setForm({
      max_hourly_usd: String(query.data.hourly.limit),
      max_daily_usd: String(query.data.daily.limit),
      max_monthly_usd: String(query.data.monthly.limit),
      alert_threshold: String(query.data.alert_threshold),
    });
    hasSeeded.current = true;
    lastSeededSavedAt.current = lastSavedAt;
  }, [query.data, lastSavedAt]);

  // Server-truth snapshot for the dirty flag. Stringified so a refetch
  // that returns identical numbers doesn't flicker the Save button.
  const serverForm: FormState | null = query.data
    ? {
        max_hourly_usd: String(query.data.hourly.limit),
        max_daily_usd: String(query.data.daily.limit),
        max_monthly_usd: String(query.data.monthly.limit),
        alert_threshold: String(query.data.alert_threshold),
      }
    : null;
  const dirty =
    serverForm !== null &&
    (form.max_hourly_usd !== serverForm.max_hourly_usd ||
      form.max_daily_usd !== serverForm.max_daily_usd ||
      form.max_monthly_usd !== serverForm.max_monthly_usd ||
      form.alert_threshold !== serverForm.alert_threshold);

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
      // Allow the next `query.data` delivery to reseed the form.
      setLastSavedAt(Date.now());
      addToast(t("userBudget.toast.saved", "Spend cap saved"), "success");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const onClear = async () => {
    setError(null);
    try {
      await deleteMut.mutateAsync(name);
      setForm(ZERO_FORM);
      // Clearing is also a save; trip the reseed gate so refetched
      // (now-zeroed) limits replace ZERO_FORM cleanly.
      setLastSavedAt(Date.now());
      addToast(t("userBudget.toast.saved", "Spend cap saved"), "success");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const onDiscard = () => {
    if (!serverForm) return;
    setForm(serverForm);
    setError(null);
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<Wallet className="h-4 w-4" />}
        title={t("user_budget.title", "User budget")}
        subtitle={name}
        helpText={t("user_budget.help")}
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
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3 sm:gap-4">
          {(
            [
              ["hourly", Clock] as const,
              ["daily", Calendar] as const,
              ["monthly", CalendarDays] as const,
            ]
          ).map(([w, WIcon]) => {
            const win = query.data![w];
            const unlimited = win.limit <= 0;
            const breached =
              !unlimited && win.pct >= query.data!.alert_threshold;
            const widthPct = unlimited
              ? 0
              : Math.min(100, win.pct * 100);
            return (
              <Card key={w} padding="md" className="relative overflow-hidden">
                <div
                  className={`absolute inset-x-0 top-0 h-1 bg-linear-to-r ${
                    breached
                      ? "from-error via-error/60 to-error/30"
                      : "from-brand via-brand/60 to-brand/30"
                  }`}
                />
                <div className="flex items-start justify-between mb-3">
                  <div>
                    <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/70">
                      {t(`user_budget.window_${w}`, w)}
                    </p>
                    <div className="mt-1 flex items-baseline gap-1.5">
                      <span
                        className={`text-2xl sm:text-3xl font-black tracking-tight font-mono ${
                          breached ? "text-error" : "text-text-main"
                        }`}
                      >
                        ${win.spend.toFixed(4)}
                      </span>
                    </div>
                    <p className="mt-0.5 text-[11px] text-text-dim font-mono">
                      {t("user_budget.of_cap", "of {{cap}}", {
                        cap: unlimited ? "∞" : `$${win.limit.toFixed(2)}`,
                      })}
                    </p>
                  </div>
                  <div
                    className={`w-9 h-9 rounded-xl flex items-center justify-center shrink-0 ${
                      breached
                        ? "bg-error/10 text-error"
                        : "bg-brand/10 text-brand"
                    }`}
                  >
                    <WIcon className="w-4 h-4" />
                  </div>
                </div>
                {!unlimited ? (
                  <>
                    <div className="h-2 w-full overflow-hidden rounded-full bg-main/60">
                      <div
                        className={`h-full rounded-full transition-all duration-500 ${
                          breached
                            ? "bg-error shadow-[0_0_8px_rgba(239,68,68,0.45)]"
                            : "bg-brand shadow-[0_0_8px_var(--brand-color)]"
                        }`}
                        style={{ width: `${widthPct.toFixed(1)}%` }}
                      />
                    </div>
                    <p className="mt-1.5 text-[10px] font-mono text-text-dim">
                      {widthPct.toFixed(1)}%
                      {breached && (
                        <span className="ml-1.5 text-error font-bold uppercase tracking-wider">
                          {t("user_budget.over_threshold", "over threshold")}
                        </span>
                      )}
                    </p>
                  </>
                ) : (
                  <p className="text-[10px] text-text-dim/60 italic">
                    {t("user_budget.no_cap", "no cap on this window")}
                  </p>
                )}
              </Card>
            );
          })}
        </div>
      )}

      <Card padding="lg">
        <div className="flex items-center gap-2 mb-1.5">
          <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center">
            <Wallet className="w-4 h-4 text-brand" />
          </div>
          <h2 className="text-sm font-black tracking-tight uppercase">
            {t("user_budget.set_limits", "Set spend limits")}
          </h2>
        </div>
        <p className="text-xs text-text-dim mb-5 leading-relaxed">
          {t(
            "user_budget.zero_means_unlimited",
            "Set any window to 0 for unlimited on that window. Threshold is the fraction of any limit at which a BudgetExceeded audit fires.",
          )}
        </p>
        <form onSubmit={onSubmit} className="space-y-4">
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
            {(
              [
                [
                  "max_hourly_usd",
                  t("userBudget.fields.max_hourly", "Max hourly USD"),
                  Clock,
                ],
                [
                  "max_daily_usd",
                  t("userBudget.fields.max_daily", "Max daily USD"),
                  Calendar,
                ],
                [
                  "max_monthly_usd",
                  t("userBudget.fields.max_monthly", "Max monthly USD"),
                  CalendarDays,
                ],
                [
                  "alert_threshold",
                  t(
                    "userBudget.fields.alert_threshold",
                    "Alert threshold (0–1)",
                  ),
                  Bell,
                ],
              ] as const
            ).map(([key, label, FieldIcon]) => (
              <Input
                key={key}
                label={label}
                type="number"
                step="0.01"
                min="0"
                value={form[key]}
                onChange={(e) =>
                  setForm((f) => ({ ...f, [key]: e.target.value }))
                }
                leftIcon={<FieldIcon className="h-4 w-4" />}
                className="font-mono"
              />
            ))}
          </div>
          {error && (
            <div className="flex items-center gap-2 p-3 rounded-xl bg-error/5 border border-error/20 text-xs text-error">
              <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
              {error}
            </div>
          )}
          <div className="flex flex-wrap items-center gap-2 pt-2 border-t border-border-subtle/50">
            <Button
              type="submit"
              variant="primary"
              disabled={updateMut.isPending || !dirty}
              leftIcon={<Check className="h-3.5 w-3.5" />}
            >
              {updateMut.isPending
                ? t("user_budget.saving", "Saving…")
                : t("user_budget.save", "Save")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={onDiscard}
              disabled={!dirty}
            >
              {t("user_budget.discard", "Discard")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={onClear}
              disabled={deleteMut.isPending}
              leftIcon={<Trash2 className="h-3.5 w-3.5" />}
              className="ml-auto !text-error hover:!bg-error/10"
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
