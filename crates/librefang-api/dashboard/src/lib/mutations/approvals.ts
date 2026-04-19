import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  approveApproval,
  rejectApproval,
  batchResolveApprovals,
  modifyAndRetryApproval,
  totpSetup,
  totpConfirm,
  totpRevoke,
} from "../../api";
import { approvalKeys, totpKeys } from "../queries/keys";

export function useApproveApproval() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, totpCode }: { id: string; totpCode?: string }) =>
      approveApproval(id, totpCode),
    onSuccess: () => qc.invalidateQueries({ queryKey: approvalKeys.all }),
  });
}

export function useRejectApproval() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: rejectApproval,
    onSuccess: () => qc.invalidateQueries({ queryKey: approvalKeys.all }),
  });
}

export function useBatchResolveApprovals() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ ids, decision }: { ids: string[]; decision: "approve" | "reject" }) =>
      batchResolveApprovals(ids, decision),
    onSuccess: () => qc.invalidateQueries({ queryKey: approvalKeys.all }),
  });
}

export function useModifyAndRetryApproval() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, feedback }: { id: string; feedback: string }) =>
      modifyAndRetryApproval(id, feedback),
    onSuccess: () => qc.invalidateQueries({ queryKey: approvalKeys.all }),
  });
}

export function useTotpSetup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: totpSetup,
    onSuccess: () => qc.invalidateQueries({ queryKey: totpKeys.all }),
  });
}

export function useTotpConfirm() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: totpConfirm,
    onSuccess: () => qc.invalidateQueries({ queryKey: totpKeys.all }),
  });
}

export function useTotpRevoke() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: totpRevoke,
    onSuccess: () => qc.invalidateQueries({ queryKey: totpKeys.all }),
  });
}
