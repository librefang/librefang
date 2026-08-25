import { useMutation, useQueryClient, type QueryKey } from "@tanstack/react-query";
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

function useInvalidatingMutation<TVariables, TResult>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
  queryKey: QueryKey,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: () => qc.invalidateQueries({ queryKey }),
  });
}

export class BatchApprovalError extends Error {
  readonly failures: Array<{ id: string; status: string; message?: string }>;

  constructor(failures: Array<{ id: string; status: string; message?: string }>) {
    super(`${failures.length} approval${failures.length === 1 ? "" : "s"} failed`);
    this.name = "BatchApprovalError";
    this.failures = failures;
    Object.setPrototypeOf(this, BatchApprovalError.prototype);
  }
}

export function useApproveApproval() {
  return useInvalidatingMutation(
    ({ id, totpCode }: { id: string; totpCode?: string }) => approveApproval(id, totpCode),
    approvalKeys.all,
  );
}

export function useRejectApproval() {
  return useInvalidatingMutation(rejectApproval, approvalKeys.all);
}

export function useBatchResolveApprovals() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ ids, decision }: { ids: string[]; decision: "approve" | "reject" }) =>
      batchResolveApprovals(ids, decision).then((data) => {
        const failures = data.results.filter((result) => result.status === "error");
        if (failures.length > 0) throw new BatchApprovalError(failures);
        return data;
      }),
    // A partial failure may still have resolved other approvals.
    onSettled: () => qc.invalidateQueries({ queryKey: approvalKeys.all }),
  });
}

export function useModifyAndRetryApproval() {
  return useInvalidatingMutation(
    ({ id, feedback }: { id: string; feedback: string }) =>
      modifyAndRetryApproval(id, feedback),
    approvalKeys.all,
  );
}

export function useTotpSetup() {
  return useInvalidatingMutation(totpSetup, totpKeys.all);
}

export function useTotpConfirm() {
  return useInvalidatingMutation(totpConfirm, totpKeys.all);
}

export function useTotpRevoke() {
  return useInvalidatingMutation(totpRevoke, totpKeys.all);
}
