import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useCronJobs } from "./runtime";
import * as api from "../../api";
import { cronKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../../api", () => ({
  listCronJobs: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useCronJobs", () => {
  it("should be disabled when agentId is undefined", () => {
    const { result } = renderHook(() => useCronJobs(), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(api.listCronJobs).not.toHaveBeenCalled();
  });

  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => useCronJobs(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(api.listCronJobs).not.toHaveBeenCalled();
  });

  it("should be enabled when agentId is valid string, fetches data", async () => {
    const mockJobs = [
      { id: "job-1", enabled: true, name: "Test Job", schedule: "0 * * * *" },
    ];
    vi.mocked(api.listCronJobs).mockResolvedValue(mockJobs);

    const { result } = renderHook(() => useCronJobs("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockJobs);
    expect(api.listCronJobs).toHaveBeenCalledWith("agent-1");
  });

  it("should use the correct queryKey", async () => {
    const mockJobs: Array<{ id: string; enabled: boolean; name: string; schedule: string }> = [];
    vi.mocked(api.listCronJobs).mockResolvedValue(mockJobs);
    const { queryClient, wrapper } = createQueryClientWrapper();
    renderHook(() => useCronJobs("test-agent"), { wrapper });
    await waitFor(() => {
      expect(queryClient.getQueryData(cronKeys.jobs("test-agent"))).toEqual(mockJobs);
    });
  });
});
