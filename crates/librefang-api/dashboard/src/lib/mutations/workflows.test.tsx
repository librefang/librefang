import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  useRunWorkflow,
  useDryRunWorkflow,
  useDeleteWorkflow,
  useCreateWorkflow,
  useUpdateWorkflow,
  useInstantiateTemplate,
} from "./workflows";
import { workflowKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  runWorkflow: vi.fn().mockResolvedValue({ status: "ok" }),
  dryRunWorkflow: vi.fn().mockResolvedValue({ valid: true, steps: [] }),
  deleteWorkflow: vi.fn().mockResolvedValue({ status: "ok" }),
  createWorkflow: vi.fn().mockResolvedValue({ id: "wf-1" }),
  updateWorkflow: vi.fn().mockResolvedValue({ status: "ok" }),
  instantiateTemplate: vi.fn().mockResolvedValue({ workflow_id: "wf-1" }),
  saveWorkflowAsTemplate: vi.fn().mockResolvedValue({ status: "ok" }),
}));

describe("useRunWorkflow", () => {
  it("invalidates workflow runs, lists, and detail for the workflow", async () => {
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
      queryKey: workflowKeys.detail("wf-1"),
    });
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
  { name: "useDeleteWorkflow", hook: useDeleteWorkflow, arg: "wf-1" },
  {
    name: "useCreateWorkflow",
    hook: useCreateWorkflow,
    arg: { name: "New workflow", steps: [] },
  },
  {
    name: "useUpdateWorkflow",
    hook: useUpdateWorkflow,
    arg: { workflowId: "wf-1", payload: { name: "Updated workflow" } },
  },
  {
    name: "useInstantiateTemplate",
    hook: useInstantiateTemplate,
    arg: { id: "tmpl-1", params: {} },
  },
])("$name", ({ hook, arg }) => {
  it("invalidates workflowKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => hook(), { wrapper });

    await result.current.mutateAsync(arg as never);

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: workflowKeys.all });
  });
});
