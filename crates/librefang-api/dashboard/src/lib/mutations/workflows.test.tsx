import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  useRunWorkflow,
  useDryRunWorkflow,
  useDeleteWorkflow,
  useCreateWorkflow,
  useUpdateWorkflow,
  useInstantiateTemplate,
  useSaveWorkflowAsTemplate,
} from "./workflows";
import * as httpClient from "../http/client";
import { workflowKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  runWorkflow: vi.fn().mockResolvedValue({ status: "ok" }),
  dryRunWorkflow: vi.fn().mockResolvedValue({ valid: true, steps: [] }),
  deleteWorkflow: vi.fn().mockResolvedValue({ status: "ok" }),
  createWorkflow: vi.fn().mockResolvedValue({ id: "wf-1" }),
  updateWorkflow: vi.fn().mockResolvedValue({ id: "wf-1", name: "Updated workflow", description: "" }),
  instantiateTemplate: vi.fn().mockResolvedValue({ workflow_id: "wf-1" }),
  saveWorkflowAsTemplate: vi.fn().mockResolvedValue({ status: "ok" }),
}));

describe("useRunWorkflow", () => {
  it("invalidates workflow runs, lists, and returned run detail", async () => {
    vi.mocked(httpClient.runWorkflow).mockResolvedValueOnce({ status: "ok", run_id: "run-1" });

    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useRunWorkflow(), { wrapper });

    await result.current.mutateAsync({ workflowId: "wf-1", input: "hello" });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: workflowKeys.runs("wf-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: workflowKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: workflowKeys.runDetail("run-1"),
    });
  });

  it("does not invalidate run detail queries when response has no run id", async () => {
    vi.mocked(httpClient.runWorkflow).mockResolvedValueOnce({ status: "ok" });

    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useRunWorkflow(), { wrapper });

    await result.current.mutateAsync({ workflowId: "wf-1", input: "hello" });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: workflowKeys.runs("wf-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: workflowKeys.lists(),
    });
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: workflowKeys.runDetails(),
    });
    expect(invalidateSpy.mock.calls).toHaveLength(2);
  });
});

describe("useDryRunWorkflow", () => {
  it("does not invalidate cached workflow queries", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDryRunWorkflow(), { wrapper });

    await result.current.mutateAsync({ workflowId: "wf-1", input: "hello" });

    expect(invalidateSpy).not.toHaveBeenCalled();
  });
});

describe.each([
  {
    name: "useDeleteWorkflow",
    hook: useDeleteWorkflow,
    arg: "wf-1",
    // detail(workflowId) is evicted via removeQueries (see separate test
    // below) — the deleted workflow has no row to refetch, so an
    // invalidate would just trigger a 404 on next read. Only lists()
    // and runs() go through invalidateQueries here.
    expectedKeys: [workflowKeys.lists(), workflowKeys.runs("wf-1")],
  },
  {
    name: "useCreateWorkflow",
    hook: useCreateWorkflow,
    arg: { name: "New workflow", steps: [] },
    expectedKeys: [workflowKeys.lists()],
  },
  {
    name: "useUpdateWorkflow",
    hook: useUpdateWorkflow,
    arg: { workflowId: "wf-1", payload: { name: "Updated workflow" } },
    expectedKeys: [workflowKeys.lists(), workflowKeys.detail("wf-1"), workflowKeys.runs("wf-1")],
  },
  {
    name: "useInstantiateTemplate",
    hook: useInstantiateTemplate,
    arg: { id: "tmpl-1", params: {} },
    expectedKeys: [workflowKeys.lists()],
  },
  {
    name: "useSaveWorkflowAsTemplate",
    hook: useSaveWorkflowAsTemplate,
    arg: "wf-1",
    expectedKeys: [workflowKeys.templates()],
  },
])("$name", ({ hook, arg, expectedKeys }) => {
  it("invalidates the expected workflow keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => hook(), { wrapper });

    await result.current.mutateAsync(arg as never);

    for (const queryKey of expectedKeys) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey });
    }
  });
});

describe("useDeleteWorkflow detail eviction", () => {
  it("removes the deleted workflow's detail query instead of refetching", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const removeSpy = vi.spyOn(queryClient, "removeQueries");

    const { result } = renderHook(() => useDeleteWorkflow(), { wrapper });

    await result.current.mutateAsync("wf-1");

    expect(removeSpy).toHaveBeenCalledWith({
      queryKey: workflowKeys.detail("wf-1"),
    });
  });
});
