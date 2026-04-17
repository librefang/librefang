import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { useCronJobs } from "./runtime";
import * as api from "../../api";
import { cronKeys } from "./keys";

vi.mock("../../api", () => ({
  listCronJobs: vi.fn(),
}));

function createWrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useCronJobs", () => {
  it("should be disabled when agentId is undefined", () => {
    const { result } = renderHook(() => useCronJobs(), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(api.listCronJobs).not.toHaveBeenCalled();
  });

  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => useCronJobs(""), {
      wrapper: createWrapper(),
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
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockJobs);
    expect(api.listCronJobs).toHaveBeenCalledWith("agent-1");
  });

  it("should use the correct queryKey", async () => {
    vi.mocked(api.listCronJobs).mockResolvedValue([]);
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    renderHook(() => useCronJobs("test-agent"), { wrapper });
    await waitFor(() => {
      expect(qc.getQueryCache().find({ queryKey: cronKeys.jobs("test-agent") })).toBeDefined();
    });
    expect(
      qc.getQueryCache().find({ queryKey: cronKeys.jobs("test-agent") })?.queryKey,
    ).toEqual(cronKeys.jobs("test-agent"));
  });
});
