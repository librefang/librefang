// Human-in-the-loop operator-step worklist banner (#4977).
//
// Surfaces every workflow run currently paused at an `Operator` step so a
// human operator can land on the Workflows page and see "you have N runs
// waiting for review" without scanning the run list per-workflow. The
// per-row action bar lives in `OperatorActionBar` (rendered inside the
// run detail panel) — this banner is the discovery surface, not the
// resolution surface; clicking a row scrolls the matching run into the
// detail panel via `onSelectRun(runId)`.
//
// Data layer: `usePendingOperatorRuns` (lib/queries/workflows.ts). No
// inline `fetch()` / `api.*` calls per the dashboard rule.

import { AlertCircle, ChevronRight, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "./ui/Badge";
import { usePendingOperatorRuns } from "../lib/queries/workflows";

interface Props {
  /** Callback when the operator clicks a pending row — typically scrolls
   *  the run into the detail panel and selects it. Optional; when
   *  omitted the rows render as static badges. */
  onSelectRun?: (runId: string, workflowId: string) => void;
}

const MAX_VISIBLE_PENDING_RUNS = 5;

export function PendingOperatorReviewsBanner({ onSelectRun }: Props) {
  const { t } = useTranslation();
  const query = usePendingOperatorRuns();

  // Quiet when there's nothing to surface — don't push other content
  // down with an empty banner.
  if (query.isLoading) {
    return null;
  }
  if (query.isError) {
    return (
      <div
        role="alert"
        className="flex items-center justify-between gap-3 rounded-xl border border-error/30 bg-error/5 p-3"
      >
        <span className="text-xs font-semibold text-error">
          {t("workflows.operator.load_failed")}
        </span>
        <button
          type="button"
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
          className="shrink-0 rounded-md border border-error/30 px-2 py-1 text-[10px] font-bold text-error hover:bg-error/10 disabled:opacity-50"
        >
          {t("common.retry")}
        </button>
      </div>
    );
  }
  if (!query.data || query.data.length === 0) {
    return null;
  }

  const rows = query.data;
  const visibleRows = rows.slice(0, MAX_VISIBLE_PENDING_RUNS);
  const hiddenCount = rows.length - visibleRows.length;
  return (
    <div className="rounded-xl border border-warning/30 bg-warning/5 p-3 space-y-2">
      <div className="flex items-center gap-2">
        <AlertCircle className="h-3.5 w-3.5 text-warning" />
        <span className="text-xs font-bold text-warning">
          {t("workflows.operator.pending_review", {
            defaultValue: "{{count}} workflow runs awaiting operator review",
            count: rows.length,
          })}
        </span>
        {query.isFetching && <Loader2 className="h-3 w-3 animate-spin text-warning/60" />}
      </div>
      <ul className="space-y-1">
        {visibleRows.map((row) => {
          const allowedCount = row.actions?.length ?? 0;
          return (
            <li key={row.run_id}>
              <button
                type="button"
                className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg bg-main border border-border-subtle hover:bg-surface text-left transition-colors"
                onClick={() => onSelectRun?.(row.run_id, row.workflow_id)}
                aria-label={t("workflows.operator.review_run", {
                  workflow: row.workflow_name,
                  step: row.step_name,
                })}
              >
                <div className="flex-1 min-w-0">
                  <p className="text-[11px] font-semibold truncate">
                    {row.workflow_name}
                    <span className="ml-2 text-text-dim/60 font-mono text-[10px]">
                      {row.step_name}
                    </span>
                  </p>
                  <p className="text-[9px] text-text-dim/60 truncate">
                    {row.artifact ||
                      t("workflows.operator.empty_artifact", {
                        defaultValue: "(empty artifact)",
                      })}
                  </p>
                </div>
                <Badge variant="default" className="text-[9px]">
                  {allowedCount} action{allowedCount === 1 ? "" : "s"}
                </Badge>
                <ChevronRight className="h-3 w-3 text-text-dim/40 shrink-0" />
              </button>
            </li>
          );
        })}
      </ul>
      {hiddenCount > 0 && (
        <p className="text-[10px] font-medium text-warning/80">
          {t("workflows.operator.more_pending", { count: hiddenCount })}
        </p>
      )}
    </div>
  );
}
