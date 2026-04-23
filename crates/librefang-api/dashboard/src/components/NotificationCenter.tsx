import { useState, useMemo, useCallback } from "react";
import { Bell, Check, X, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../lib/store";
import { useNavigate } from "@tanstack/react-router";
import { useApprovalCount, useApprovals, useTotpStatus } from "../lib/queries/approvals";
import { useApproveApproval, useRejectApproval } from "../lib/mutations/approvals";

const POLL_INTERVAL_MS = 5_000;
const MAX_VISIBLE_ITEMS = 10;
const MAX_BADGE_COUNT = 99;

export function NotificationCenter() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const addToast = useUIStore((s) => s.addToast);
  const navigate = useNavigate();

  const countQuery = useApprovalCount({ refetchInterval: POLL_INTERVAL_MS });
  const listQuery = useApprovals({ enabled: open });
  const totpQuery = useTotpStatus();
  const approveMutation = useApproveApproval();
  const rejectMutation = useRejectApproval();

  const totpEnforced = totpQuery.data?.enforced ?? false;

  const pendingCount = countQuery.data ?? 0;
  const pendingItems = useMemo(
    () => (listQuery.data ?? []).filter((a) => !a.status || a.status === "pending"),
    [listQuery.data]
  );

  const handleAction = useCallback(async (id: string, action: "approve" | "reject") => {
    // When TOTP is enforced, redirect to Approvals page for approve
    if (action === "approve" && totpEnforced) {
      setOpen(false);
      navigate({ to: "/approvals" });
      addToast(t("approvals.totpRequired", "TOTP code required. Use the Approvals page."), "info");
      return;
    }
    try {
      if (action === "approve") await approveMutation.mutateAsync({ id });
      else await rejectMutation.mutateAsync(id);
      addToast(
        t(`approvals.${action === "approve" ? "approvedToast" : "rejectedToast"}`),
        "success"
      );
    } catch {
      addToast(t("common.error", "Action failed"), "error");
    }
  }, [totpEnforced, approveMutation, rejectMutation, addToast, navigate, t]);

  const goToAgent = useCallback((agentId: string) => {
    setOpen(false);
    navigate({ to: "/chat", search: { agentId } });
  }, [navigate]);

  return (
    <div className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="relative flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-primary hover:bg-surface-hover transition-colors duration-200"
        aria-label={pendingCount > 0 ? `${t("approvals.pending_review", "Notifications")} (${pendingCount})` : t("approvals.pending_review", "Notifications")}
        aria-expanded={open}
        aria-haspopup="menu"
      >
        <Bell className="h-4 w-4" />
        {countQuery.isError ? (
          <span className="absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full bg-error/60 ring-2 ring-surface" title={t("common.error", "Connection error")} />
        ) : pendingCount > 0 ? (
          <span className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-error px-1 text-[10px] font-bold text-white">
            {pendingCount > MAX_BADGE_COUNT ? `${MAX_BADGE_COUNT}+` : pendingCount}
          </span>
        ) : null}
      </button>

      {open && (
        <>
          <div
            className="fixed inset-0 z-40"
            onClick={() => setOpen(false)}
          />
          <div className="absolute right-0 top-full mt-1 z-50 w-96 rounded-xl border border-border-subtle bg-surface shadow-xl" role="menu">
            <div className="px-4 py-3 border-b border-border-subtle flex items-center justify-between">
              <h3 className="text-sm font-bold text-text-main">
                {t("approvals.pending_review", "Pending Approvals")}
              </h3>
              {pendingItems.length > 0 && (
                <button
                  onClick={() => {
                    setOpen(false);
                    navigate({ to: "/approvals" });
                  }}
                  className="text-xs text-primary hover:underline"
                >
                  {t("common.viewAll", "View all")}
                </button>
              )}
            </div>
            <div className="max-h-96 overflow-y-auto">
              {pendingItems.length === 0 ? (
                <div className="px-4 py-6 text-center text-sm text-text-dim">
                  {t("approvals.queue_clear_desc", "All clear")}
                </div>
              ) : (
                pendingItems.slice(0, MAX_VISIBLE_ITEMS).map((item) => (
                  <div
                    key={item.id}
                    role="menuitem"
                    className="px-4 py-3 border-b last:border-0 border-border-subtle hover:bg-surface-hover transition-colors"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5">
                          <p className="text-sm font-medium text-text-main truncate">
                            {item.tool_name}
                          </p>
                          {item.risk_level && (
                            <span className={`text-[10px] px-1.5 py-0.5 rounded font-bold uppercase ${
                              item.risk_level === "critical" ? "bg-error/10 text-error" :
                              item.risk_level === "high" ? "bg-warning/10 text-warning" :
                              "bg-surface-hover text-text-dim"
                            }`}>
                              {item.risk_level}
                            </span>
                          )}
                        </div>
                        {item.agent_id && (
                          <button
                            onClick={() => goToAgent(item.agent_id!)}
                            className="flex items-center gap-1 text-xs text-primary hover:underline mt-0.5"
                            title={t("approvals.goToAgent", "Open agent chat")}
                          >
                            <span className="truncate">{item.agent_name ?? item.agent_id}</span>
                            <ExternalLink className="w-3 h-3 shrink-0" />
                          </button>
                        )}
                        {(item.action_summary || item.description) && (
                          <p className="text-xs text-text-dim mt-1 line-clamp-2">
                            {item.action_summary || item.description}
                          </p>
                        )}
                      </div>
                      <div className="flex gap-1 shrink-0">
                        <button
                          onClick={() => handleAction(item.id, "approve")}
                          className="p-1 rounded hover:bg-success/10 text-success transition-colors"
                          title={t("approvals.approve")}
                          aria-label={t("approvals.approve")}
                        >
                          <Check className="w-4 h-4" />
                        </button>
                        <button
                          onClick={() => handleAction(item.id, "reject")}
                          className="p-1 rounded hover:bg-error/10 text-error transition-colors"
                          title={t("approvals.reject")}
                          aria-label={t("approvals.reject")}
                        >
                          <X className="w-4 h-4" />
                        </button>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
