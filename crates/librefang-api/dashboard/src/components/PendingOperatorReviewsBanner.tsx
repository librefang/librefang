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
import { Badge } from "./ui/Badge";
import { usePendingOperatorRuns } from "../lib/queries/workflows";

interface Props {
  /** Callback when the operator clicks a pending row — typically scrolls
   *  the run into the detail panel and selects it. Optional; when
   *  omitted the rows render as static badges. */
  onSelectRun?: (runId: string, workflowId: string) => void;
}

export function PendingOperatorReviewsBanner({ onSelectRun }: Props) {
  const query = usePendingOperatorRuns();

  // Quiet when there's nothing to surface — don't push other content
  // down with an empty banner.
  if (query.isLoading) {
    return null;
  }
  if (query.isError || !query.data || query.data.length === 0) {
    return null;
  }

  const rows = query.data;
  return (
    <div className="rounded-xl border border-warning/30 bg-warning/5 p-3 space-y-2">
      <div className="flex items-center gap-2">
        <AlertCircle className="h-3.5 w-3.5 text-warning" />
        <span className="text-xs font-bold text-warning">
          {rows.length === 1
            ? "1 workflow run awaiting operator review"
            : `${rows.length} workflow runs awaiting operator review`}
        </span>
        {query.isFetching && <Loader2 className="h-3 w-3 animate-spin text-warning/60" />}
      </div>
      <ul className="space-y-1">
        {rows.map((row) => {
          const allowedCount = row.actions.length;
          return (
            <li key={row.run_id}>
              <button
                type="button"
                className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg bg-main border border-border-subtle hover:bg-surface text-left transition-colors"
                onClick={() => onSelectRun?.(row.run_id, row.workflow_id)}
              >
                <div className="flex-1 min-w-0">
                  <p className="text-[11px] font-semibold truncate">
                    {row.workflow_name}
                    <span className="ml-2 text-text-dim/60 font-mono text-[10px]">
                      {row.step_name}
                    </span>
                  </p>
                  <p className="text-[9px] text-text-dim/60 truncate">
                    {row.artifact || "(empty artifact)"}
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
    </div>
  );
}
