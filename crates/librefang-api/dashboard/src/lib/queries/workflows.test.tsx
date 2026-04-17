import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import type { WorkflowRunDetail } from "../../api";
import { useWorkflowRuns, useWorkflowRunDetail, workflowQueries } from "./workflows";
import * as httpClient from "../http/client";
import { workflowKeys } from "./keys";

vi.mock("../http/client", () => ({
  listWorkflowRuns: vi.fn(),
  getWorkflowRun: vi.fn(),
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

describe("useWorkflowRuns", () => {
  it("should be disabled when workflowId is empty string", () => {
    const { result } = renderHook(() => useWorkflowRuns(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.listWorkflowRuns).not.toHaveBeenCalled();
  });

  it("should fetch when workflowId is valid", async () => {
    const mockRuns = [{ id: "run-1", status: "completed" }];
    vi.mocked(httpClient.listWorkflowRuns).mockResolvedValue(mockRuns);

    const { result } = renderHook(() => useWorkflowRuns("wf-123"), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockRuns);
    expect(httpClient.listWorkflowRuns).toHaveBeenCalledWith("wf-123");
  });

  it("should use the correct queryKey", () => {
    expect(workflowQueries.runs("wf-456").queryKey).toEqual(
      workflowKeys.runs("wf-456"),
    );
  });
});

describe("useWorkflowRunDetail", () => {
  it("should be disabled when runId is empty string", () => {
    const { result } = renderHook(() => useWorkflowRunDetail(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getWorkflowRun).not.toHaveBeenCalled();
  });

  it("should fetch when runId is valid", async () => {
    const mockRun: WorkflowRunDetail = { id: "run-1", workflow_id: "wf-1", workflow_name: "Test Workflow", input: "{}", state: "running", started_at: "2024-01-01T00:00:00Z", step_results: [] };
    vi.mocked(httpClient.getWorkflowRun).mockResolvedValue(mockRun);

    const { result } = renderHook(() => useWorkflowRunDetail("run-1"), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockRun);
    expect(httpClient.getWorkflowRun).toHaveBeenCalledWith("run-1");
  });

  it("should use the correct queryKey", () => {
    expect(workflowQueries.runDetail("run-2").queryKey).toEqual(
      workflowKeys.runDetail("run-2"),
    );
  });
});
