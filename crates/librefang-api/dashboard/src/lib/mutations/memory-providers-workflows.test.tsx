import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useAddMemory } from "./memory";
import { useSetDefaultProvider } from "./providers";
import { useRunWorkflow } from "./workflows";
import { createQueryClientWrapper } from "../test/query-client";
import { providerKeys, runtimeKeys, workflowKeys } from "../queries/keys";
import * as api from "../../api";
import * as httpClient from "../http/client";

vi.mock("../../api", async () => {
  const actual = await vi.importActual<typeof import("../../api")>("../../api");
  return {
    ...actual,
    addMemoryFromText: vi.fn().mockResolvedValue({ status: "ok" }),
    setDefaultProvider: vi.fn().mockResolvedValue({ status: "ok" }),
  };
});

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>("../http/client");
  return {
    ...actual,
    runWorkflow: vi.fn().mockResolvedValue({ status: "ok" }),
  };
});

describe("useAddMemory", () => {
  it("passes selected level to addMemoryFromText", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useAddMemory(), { wrapper });

    await result.current.mutateAsync({
      content: "remember this",
      level: "semantic",
      agentId: "agent-1",
    });

    expect(api.addMemoryFromText).toHaveBeenCalledWith(
      "remember this",
      { level: "semantic", agentId: "agent-1" },
    );
  });
});

describe("useSetDefaultProvider", () => {
  it("invalidates providerKeys.all and runtimeKeys.status", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useSetDefaultProvider(), { wrapper });

    await result.current.mutateAsync({ id: "openai", model: "gpt-4.1" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: providerKeys.all });
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: runtimeKeys.status() });
  });
});

describe("useRunWorkflow", () => {
  it("invalidates lists, runs and runDetails", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useRunWorkflow(), { wrapper });

    await result.current.mutateAsync({ workflowId: "wf-1", input: "{}" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: workflowKeys.lists() });
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: workflowKeys.runs("wf-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: workflowKeys.runDetails() });
    expect(httpClient.runWorkflow).toHaveBeenCalledWith("wf-1", "{}");
  });
});
