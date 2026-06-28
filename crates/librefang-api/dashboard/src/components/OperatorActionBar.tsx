// Human-in-the-loop operator action bar (#4977).
//
// Renders when a workflow run is paused at a `StepMode::Operator` step:
// shows the artifact awaiting review and the action buttons the workflow
// author authorised. Posts the chosen action back through
// `useResolveOperatorStep` — the worklist + run detail caches are
// invalidated automatically.
//
// Data layer: all reads via `useWorkflowOperatorPause`, write via
// `useResolveOperatorStep`. No inline `fetch()` / `api.*` calls per the
// dashboard rule.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, XCircle, Pencil, MessageSquare, AlertCircle, Loader2 } from "lucide-react";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { useWorkflowOperatorPause } from "../lib/queries/workflows";
import { useResolveOperatorStep } from "../lib/mutations/workflows";
import { ApiError } from "../lib/http/client";
import type { OperatorActionDescriptor, OperatorActionVerb } from "../api";

interface Props {
  runId: string;
}

/** Map an `OperatorActionDescriptor` (the value the inspect endpoint
 *  returns) to a stable wire verb + display affordances. Centralised
 *  here so the action list, button labels, and resolve POST stay in
 *  lockstep — adding a verb in `OperatorAction` (Rust) requires updating
 *  exactly this function on the dashboard. */
function describeAction(
  action: OperatorActionDescriptor,
  t: (key: string, fallback: string, options?: Record<string, unknown>) => string,
): {
  verb: OperatorActionVerb;
  label: string;
  needsPayload: boolean;
  field?: string;
  variant: "primary" | "secondary" | "danger" | "ghost";
  icon: React.ComponentType<{ className?: string }>;
} {
  if (typeof action === "string") {
    switch (action) {
      case "approve":
        return { verb: "approve", label: t("approvals.approve", "Approve"), needsPayload: false, variant: "primary", icon: CheckCircle2 };
      case "reject":
        return { verb: "reject", label: t("approvals.reject", "Reject"), needsPayload: false, variant: "danger", icon: XCircle };
      case "edit":
        return { verb: "edit", label: t("common.edit", "Edit"), needsPayload: true, variant: "secondary", icon: Pencil };
      case "freeform_input":
        return { verb: "freeform_input", label: t("workflows.operator.freeform_input", "Freeform input"), needsPayload: true, variant: "secondary", icon: MessageSquare };
    }
  }
  // ProvideInput carries a `field` name — labelled on the button.
  return {
    verb: "provide_input",
    label: t("workflows.operator.provide_field", "Provide '{{field}}'", { field: action.provide_input.field }),
    needsPayload: true,
    field: action.provide_input.field,
    variant: "secondary",
    icon: MessageSquare,
  };
}

export function OperatorActionBar({ runId }: Props) {
  const { t } = useTranslation();
  const pauseQuery = useWorkflowOperatorPause(runId);
  const resolve = useResolveOperatorStep();
  // Per-action local state: when the user clicks a `needs_payload` button
  // we open an inline textarea instead of submitting immediately. Keyed
  // by `verb + (field ?? "")` so multiple payload actions on the same
  // step don't share a draft.
  const [openVerb, setOpenVerb] = useState<string | null>(null);
  const [payload, setPayload] = useState("");
  const [error, setError] = useState<string | null>(null);

  if (pauseQuery.isLoading) {
    return (
      <div className="flex items-center gap-1.5 p-2 text-[10px] text-text-dim/50">
        <Loader2 className="w-3 h-3 animate-spin" /> {t("workflows.operator.loading", "Loading operator review…")}
      </div>
    );
  }
  // 404 / 409 from the inspect endpoint mean "this run isn't currently
  // paused at an operator step" — render nothing so the parent panel
  // (run detail) is unchanged for non-operator runs.
  if (pauseQuery.error instanceof ApiError) {
    if (pauseQuery.error.status === 404 || pauseQuery.error.status === 409) {
      return null;
    }
    return (
      <div className="flex items-start gap-1.5 p-2 rounded-lg bg-error/5 border border-error/20">
        <AlertCircle className="w-3 h-3 text-error shrink-0 mt-0.5" />
        <p className="text-[10px] text-error">
          {t("workflows.operator.unavailable", "Operator pause unavailable:")} {pauseQuery.error.message}
        </p>
      </div>
    );
  }
  if (!pauseQuery.data) return null;

  const pause = pauseQuery.data;
  const onSubmit = async (verb: OperatorActionVerb, body: { payload?: string; field?: string }) => {
    setError(null);
    try {
      await resolve.mutateAsync({ runId, action: verb, ...body });
      setOpenVerb(null);
      setPayload("");
    } catch (e) {
      setError(e instanceof Error ? e.message : t("workflows.operator.resolve_failed", "Failed to resolve operator step"));
    }
  };

  return (
    <div className="space-y-2 p-2.5 rounded-lg bg-warning/5 border border-warning/30">
      <div className="flex items-start gap-1.5">
        <AlertCircle className="w-3 h-3 text-warning shrink-0 mt-0.5" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-[10px] font-bold text-warning">{t("workflows.operator.review_required", "Operator review required")}</span>
            <Badge variant="default" className="text-[9px]">{pause.step_name}</Badge>
          </div>
          <p className="mt-0.5 text-[9px] text-text-dim/70">
            {t("workflows.operator.paused", "Paused")} {pause.paused_at ? new Date(pause.paused_at).toLocaleString() : t("common.just_now", "just now")} ·
            {t("workflows.operator.step", "step")} {pause.operator_step_index + 1}
          </p>
        </div>
      </div>

      {/* Artifact preview — what the operator must review. */}
      <div className="rounded-md bg-main border border-border-subtle p-2">
        <p className="text-[9px] uppercase tracking-wide text-text-dim/60 mb-1">{t("workflows.operator.artifact", "Artifact")}</p>
        <pre className="text-[10px] whitespace-pre-wrap break-words font-mono leading-relaxed max-h-48 overflow-y-auto">
          {pause.artifact || "(empty)"}
        </pre>
      </div>

      {/* Action bar — one button per authorised action. Payload-bearing
          verbs expand into an inline textarea on click. */}
      <div className="flex flex-wrap gap-1.5">
        {pause.actions.map((action, idx) => {
          const desc = describeAction(action, t);
          const key = `${desc.verb}:${desc.field ?? ""}`;
          const Icon = desc.icon;
          const isOpen = openVerb === key;
          if (desc.needsPayload && isOpen) {
            return (
              <div key={`${idx}-${key}`} className="w-full space-y-1.5">
                <textarea
                  value={payload}
                  onChange={(e) => setPayload(e.target.value)}
                  placeholder={
                    desc.field
                      ? `Value for '${desc.field}'`
                      : t("workflows.operator.response_placeholder", "Your response (text)")
                  }
                  className="w-full text-[10px] font-mono rounded-md border border-border-subtle bg-main p-2 min-h-[80px] focus:outline-none focus:border-brand"
                  autoFocus
                />
                <div className="flex gap-1.5">
                  <Button
                    variant={desc.variant === "danger" ? "danger" : "primary"}
                    onClick={() =>
                      onSubmit(desc.verb, {
                        payload: payload.trim(),
                        ...(desc.field ? { field: desc.field } : {}),
                      })
                    }
                    disabled={!payload.trim() || resolve.isPending}
                  >
                    {resolve.isPending ? (
                      <>
                        <Loader2 className="w-3 h-3 animate-spin" /> {t("workflows.operator.submitting", "Submitting…")}
                      </>
                    ) : (
                      <>
                        <Icon className="w-3 h-3" /> {t("workflows.operator.submit_action", "Submit {{action}}", { action: desc.label })}
                      </>
                    )}
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setOpenVerb(null);
                      setPayload("");
                    }}
                    disabled={resolve.isPending}
                  >
                    {t("common.cancel", "Cancel")}
                  </Button>
                </div>
              </div>
            );
          }
          return (
            <Button
              key={`${idx}-${key}`}
              variant={desc.variant}
              onClick={() => {
                if (desc.needsPayload) {
                  setOpenVerb(key);
                  setPayload("");
                } else {
                  void onSubmit(desc.verb, {});
                }
              }}
              disabled={resolve.isPending || (openVerb !== null && !desc.needsPayload)}
            >
              <Icon className="w-3 h-3" /> {desc.label}
            </Button>
          );
        })}
      </div>

      {error && (
        <div className="flex items-start gap-1.5 p-1.5 rounded-md bg-error/5 border border-error/20">
          <AlertCircle className="w-3 h-3 text-error shrink-0 mt-0.5" />
          <p className="text-[10px] text-error">{error}</p>
        </div>
      )}
    </div>
  );
}
