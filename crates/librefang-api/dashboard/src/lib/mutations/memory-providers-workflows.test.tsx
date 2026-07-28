import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useAddMemory } from "./memory";
import { useConnectEveryApi, useSetDefaultProvider } from "./providers";
import { useRunWorkflow } from "./workflows";
import { createQueryClientWrapper } from "../test/query-client";
import { modelKeys, providerKeys, runtimeKeys, workflowKeys } from "../queries/keys";
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
    runWorkflow: vi.fn().mockResolvedValue({ status: "ok", run_id: "run-1" }),
    createRegistryContent: vi.fn().mockResolvedValue({ status: "ok" }),
    setProviderKey: vi.fn().mockResolvedValue({ status: "ok" }),
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
  it("invalidates providerKeys.all, modelKeys.lists, and runtimeKeys.status", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useSetDefaultProvider(), { wrapper });

    await result.current.mutateAsync({ id: "openai", model: "gpt-4.1" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: providerKeys.all });
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: modelKeys.lists() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: runtimeKeys.status() });
  });
});

describe("useConnectEveryApi", () => {
  // The two writes are not atomic: the registry entry lands first, then the key.
  // When the key write fails the daemon is already holding a keyless `everyapi` entry, so the caches that decide whether the Providers page still offers "Connect EveryAPI gateway" have to be invalidated on the failure path too — hence `onSettled` rather than `onSuccess`.
  it("invalidates provider and model caches when the key write fails after the entry was created", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    vi.mocked(httpClient.setProviderKey).mockRejectedValueOnce(new Error("key write failed"));
    const { result } = renderHook(() => useConnectEveryApi(), { wrapper });

    await expect(
      result.current.mutateAsync({ relayKey: "sk-relay-test" }),
    ).rejects.toThrow("key write failed");

    expect(httpClient.createRegistryContent).toHaveBeenCalled();
    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: providerKeys.all });
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: modelKeys.lists() });
  });

  it("invalidates provider and model caches on the success path", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useConnectEveryApi(), { wrapper });

    await result.current.mutateAsync({ relayKey: "sk-relay-test" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: providerKeys.all });
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: modelKeys.lists() });
  });
});

describe("useRunWorkflow", () => {
  it("invalidates lists, runs and returned run detail", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useRunWorkflow(), { wrapper });

    await result.current.mutateAsync({ workflowId: "wf-1", input: "{}" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: workflowKeys.lists() });
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: workflowKeys.runs("wf-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: workflowKeys.runDetail("run-1") });
    expect(httpClient.runWorkflow).toHaveBeenCalledWith("wf-1", "{}");
  });
});
