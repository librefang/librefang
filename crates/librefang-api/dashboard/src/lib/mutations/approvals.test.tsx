import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import * as api from "../../api";
import { approvalKeys, totpKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import {
  BatchApprovalError,
  useApproveApproval,
  useBatchResolveApprovals,
  useTotpRevoke,
} from "./approvals";

vi.mock("../../api", () => ({
  approveApproval: vi.fn().mockResolvedValue({}),
  rejectApproval: vi.fn().mockResolvedValue({}),
  batchResolveApprovals: vi.fn(),
  modifyAndRetryApproval: vi.fn().mockResolvedValue({}),
  totpSetup: vi.fn().mockResolvedValue({}),
  totpConfirm: vi.fn().mockResolvedValue({}),
  totpRevoke: vi.fn().mockResolvedValue({}),
}));

describe("approval mutations", () => {
  it("shares approval invalidation for individual decisions", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useApproveApproval(), { wrapper });

    await result.current.mutateAsync({ id: "approval-1" });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: approvalKeys.all });
  });

  it("shares TOTP invalidation for security mutations", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useTotpRevoke(), { wrapper });

    await result.current.mutateAsync("123456");

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: totpKeys.all });
  });

  it("rejects partial batch failures and still refreshes approvals", async () => {
    vi.mocked(api.batchResolveApprovals).mockResolvedValueOnce({
      results: [
        { id: "ok", status: "ok" },
        { id: "bad", status: "error", message: "invalid UUID" },
      ],
    });
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useBatchResolveApprovals(), { wrapper });

    const promise = result.current.mutateAsync({ ids: ["ok", "bad"], decision: "approve" });

    const error: unknown = await promise.catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(BatchApprovalError);
    expect(error).toMatchObject({
      failures: [{ id: "bad", status: "error", message: "invalid UUID" }],
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: approvalKeys.all });
  });
});
