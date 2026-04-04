import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Bell, Check, X, ExternalLink } from "lucide-react";
import { fetchApprovalCount, listApprovals, approveApproval, rejectApproval } from "../api";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../lib/store";
import { useNavigate } from "@tanstack/react-router";

export function NotificationCenter() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const navigate = useNavigate();

  const countQuery = useQuery({
    queryKey: ["approvals", "count"],
    queryFn: fetchApprovalCount,
    refetchInterval: 5000,
  });

  const listQuery = useQuery({
    queryKey: ["approvals", "list-bell"],
    queryFn: () => listApprovals(),
    enabled: open,
    refetchInterval: open ? 5000 : false,
  });

  const pendingCount = countQuery.data ?? 0;
  const pendingItems = (listQuery.data ?? []).filter(
    (a) => !a.status || a.status === "pending"
  );

  const handleAction = async (id: string, action: "approve" | "reject") => {
    try {
      if (action === "approve") await approveApproval(id);
      else await rejectApproval(id);
      addToast(
        t(`approvals.${action === "approve" ? "approvedToast" : "rejectedToast"}`),
        "success"
      );
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
    } catch {
      addToast(t("common.error", "Action failed"), "error");
    }
  };

  const goToAgent = (agentId: string) => {
    setOpen(false);
    navigate({ to: "/chat", search: { agentId } });
  };

  return (
    <div className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="relative flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-colors duration-200"
        aria-label={t("approvals.pending_review", "Notifications")}
      >
        <Bell className="h-4 w-4" />
        {countQuery.isError ? (
          <span className="absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full bg-error/60 ring-2 ring-surface" title={t("common.error", "Connection error")} />
        ) : pendingCount > 0 ? (
          <span className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-error px-1 text-[10px] font-bold text-white">
            {pendingCount > 99 ? "99+" : pendingCount}
          </span>
        ) : null}
      </button>

      {open && (
        <>
          <div
            className="fixed inset-0 z-40"
            onClick={() => setOpen(false)}
          />
          <div className="absolute right-0 top-full mt-1 z-50 w-96 rounded-xl border border-border-subtle bg-surface shadow-xl">
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
                  className="text-xs text-brand hover:underline"
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
                pendingItems.slice(0, 10).map((item) => (
                  <div
                    key={item.id}
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
                        <button
                          onClick={() => goToAgent(item.agent_id)}
                          className="flex items-center gap-1 text-xs text-brand hover:underline mt-0.5"
                          title={t("approvals.goToAgent", "Open agent chat")}
                        >
                          <span className="truncate">{item.agent_name ?? item.agent_id}</span>
                          <ExternalLink className="w-3 h-3 shrink-0" />
                        </button>
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
                        >
                          <Check className="w-4 h-4" />
                        </button>
                        <button
                          onClick={() => handleAction(item.id, "reject")}
                          className="p-1 rounded hover:bg-error/10 text-error transition-colors"
                          title={t("approvals.reject")}
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
