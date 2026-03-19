import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { approveApproval, listApprovals } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { CheckCircle } from "lucide-react";

const REFRESH_MS = 15000;

export function ApprovalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  const approvalsQuery = useQuery({ queryKey: ["approvals", "list"], queryFn: listApprovals, refetchInterval: REFRESH_MS });
  const approveMutation = useMutation({ mutationFn: ({ id }: any) => approveApproval(id) });

  const approvals = approvalsQuery.data ?? [];

  async function handleDecision(id: string, decision: "approve" | "reject") {
    setPendingId(id);
    try {
      await approveMutation.mutateAsync({ id, decision });
      await queryClient.invalidateQueries({ queryKey: ["approvals"] });
      addToast(t("common.success"), "success");
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
      />

      {approvalsQuery.isLoading ? (
        <ListSkeleton rows={3} />
      ) : approvals.length === 0 ? (
        <EmptyState
          title={t("approvals.queue_clear")}
          description={t("approvals.queue_clear_desc")}
          icon={<CheckCircle className="h-6 w-6" />}
        />
      ) : (
        <div className="grid gap-4">
          {approvals.map((a) => (
            <Card key={a.id} hover padding="lg">
              <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
                <div className="min-w-0 flex-1">
                  <Badge variant="warning">{t("approvals.pending_review")}</Badge>
                  <p className="mt-4 text-sm font-medium leading-relaxed">{a.action_summary || a.description || t("common.actions")}</p>
                </div>
                <div className="flex gap-3 shrink-0">
                  <Button variant="danger" onClick={() => handleDecision(a.id, "reject")} disabled={pendingId === a.id}>{t("approvals.reject")}</Button>
                  <Button variant="success" onClick={() => handleDecision(a.id, "approve")} disabled={pendingId === a.id}>{t("approvals.approve")}</Button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
