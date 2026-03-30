import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { approveApproval, rejectApproval, listApprovals } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { CheckCircle, XCircle, Clock } from "lucide-react";

const REFRESH_MS = 15000;

function statusBadge(status: string | undefined, t: (key: string) => string) {
  switch (status) {
    case "approved":
      return <Badge variant="success">{t("approvals.status.approved")}</Badge>;
    case "rejected":
      return <Badge variant="danger">{t("approvals.status.rejected")}</Badge>;
    case "expired":
      return <Badge variant="neutral">{t("approvals.status.expired")}</Badge>;
    default:
      return <Badge variant="warning">{t("approvals.pending_review")}</Badge>;
  }
}

function statusIcon(status: string | undefined) {
  switch (status) {
    case "approved":
      return <CheckCircle className="w-5 h-5 text-success" />;
    case "rejected":
      return <XCircle className="w-5 h-5 text-danger" />;
    case "expired":
      return <Clock className="w-5 h-5 text-text-dim" />;
    default:
      return <CheckCircle className="w-5 h-5 text-warning" />;
  }
}

function statusIconBg(status: string | undefined) {
  switch (status) {
    case "approved":
      return "bg-success/10";
    case "rejected":
      return "bg-danger/10";
    case "expired":
      return "bg-surface-2";
    default:
      return "bg-warning/10";
  }
}

export function ApprovalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  const approvalsQuery = useQuery({ queryKey: ["approvals", "list"], queryFn: listApprovals, refetchInterval: REFRESH_MS });

  const approveMutation = useMutation({
    mutationFn: (id: string) => approveApproval(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["approvals"] }),
  });
  const rejectMutation = useMutation({
    mutationFn: (id: string) => rejectApproval(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["approvals"] }),
  });

  const approvals = approvalsQuery.data ?? [];

  async function handleDecision(id: string, decision: "approve" | "reject") {
    setPendingId(id);
    try {
      if (decision === "approve") {
        await approveMutation.mutateAsync(id);
        addToast(t("approvals.approvedToast"), "success");
      } else {
        await rejectMutation.mutateAsync(id);
        addToast(t("approvals.rejectedToast"), "success");
      }
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setPendingId(null);
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.approvals")}
        title={t("approvals.title")}
        subtitle={t("approvals.subtitle")}
        isFetching={approvalsQuery.isFetching}
        onRefresh={() => void approvalsQuery.refetch()}
        icon={<CheckCircle className="h-4 w-4" />}
        helpText={t("approvals.help")}
      />

      {approvalsQuery.isLoading ? (
        <ListSkeleton rows={3} />
      ) : approvals.length === 0 ? (
        <div className="flex flex-col items-center py-20">
          <div className="relative mb-6">
            <div className="w-20 h-20 rounded-3xl bg-success/10 flex items-center justify-center">
              <CheckCircle className="h-10 w-10 text-success" />
            </div>
            <span className="absolute inset-0 rounded-3xl bg-success/5 animate-pulse" style={{ animationDuration: "3s" }} />
          </div>
          <h3 className="text-xl font-black tracking-tight">{t("approvals.queue_clear")}</h3>
          <p className="text-sm text-text-dim mt-2 max-w-xs text-center">{t("approvals.queue_clear_desc")}</p>
        </div>
      ) : (
        <div className="grid gap-4">
          {approvals.map((a) => {
            const isPending = !a.status || a.status === "pending";
            return (
              <Card key={a.id} hover padding="lg">
                <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-3">
                      <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 ${statusIconBg(a.status)}`}>
                        {statusIcon(a.status)}
                      </div>
                      <div>
                        {statusBadge(a.status, t)}
                        <p className="mt-1 text-sm font-medium leading-relaxed">{a.action_summary || a.description || t("common.actions")}</p>
                      </div>
                    </div>
                  </div>
                  {isPending ? (
                    <div className="flex gap-3 shrink-0">
                      <Button variant="danger" onClick={() => handleDecision(a.id, "reject")} disabled={pendingId === a.id}>{t("approvals.reject")}</Button>
                      <Button variant="success" onClick={() => handleDecision(a.id, "approve")} disabled={pendingId === a.id}>{t("approvals.approve")}</Button>
                    </div>
                  ) : (
                    <div className="text-sm text-text-dim shrink-0">
                      {t(`approvals.status.${a.status}`)}
                    </div>
                  )}
                </div>
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
