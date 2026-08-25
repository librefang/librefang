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
  it("fetches all jobs when agentId is undefined", async () => {
    vi.mocked(api.listCronJobs).mockResolvedValue([]);
    const { result } = renderHook(() => useCronJobs(), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(api.listCronJobs).toHaveBeenCalledWith(undefined);
  });

  it("treats an empty agentId as the API's unfiltered form", async () => {
    vi.mocked(api.listCronJobs).mockResolvedValue([]);
    const { result } = renderHook(() => useCronJobs(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(api.listCronJobs).toHaveBeenCalledWith("");
  });

  it("allows callers to disable an unfiltered query explicitly", () => {
    const { result } = renderHook(() => useCronJobs(undefined, { enabled: false }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

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
