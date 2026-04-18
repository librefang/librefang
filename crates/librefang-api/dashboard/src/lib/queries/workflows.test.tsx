import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import type { WorkflowRunDetail } from "../../api";
import { useWorkflowRuns, useWorkflowRunDetail } from "./workflows";
import * as httpClient from "../http/client";
import { workflowKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  listWorkflowRuns: vi.fn(),
  getWorkflowRun: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useWorkflowRuns", () => {
  it("should be disabled when workflowId is empty string", () => {
    const { result } = renderHook(() => useWorkflowRuns(""), {
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockRuns);
    expect(httpClient.listWorkflowRuns).toHaveBeenCalledWith("wf-123");
  });

  it("should use the correct queryKey", () => {
    const mockRuns = [{ id: "run-2", status: "queued" }];
    vi.mocked(httpClient.listWorkflowRuns).mockResolvedValue(mockRuns);

    const { queryClient, wrapper } = createQueryClientWrapper();
    renderHook(() => useWorkflowRuns("wf-456"), { wrapper });

    return waitFor(() => {
      expect(queryClient.getQueryData(workflowKeys.runs("wf-456"))).toEqual(mockRuns);
    });
  });
});

describe("useWorkflowRunDetail", () => {
  it("should be disabled when runId is empty string", () => {
    const { result } = renderHook(() => useWorkflowRunDetail(""), {
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockRun);
    expect(httpClient.getWorkflowRun).toHaveBeenCalledWith("run-1");
  });

  it("should use the correct queryKey", () => {
    const mockRun: WorkflowRunDetail = {
      id: "run-2",
      workflow_id: "wf-2",
      workflow_name: "Queued Workflow",
      input: "{}",
      state: "queued",
      started_at: "2024-01-01T00:00:00Z",
      step_results: [],
    };
    vi.mocked(httpClient.getWorkflowRun).mockResolvedValue(mockRun);

    const { queryClient, wrapper } = createQueryClientWrapper();
    renderHook(() => useWorkflowRunDetail("run-2"), { wrapper });

    return waitFor(() => {
      expect(queryClient.getQueryData(workflowKeys.runDetail("run-2"))).toEqual(mockRun);
    });
  });
});
