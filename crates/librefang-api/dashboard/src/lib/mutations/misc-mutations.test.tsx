import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import * as httpClient from "../http/client";
import { useCompleteExperiment } from "./agents";
import { useSetSessionLabel } from "./sessions";
import { useInstallSkill } from "./skills";
import { agentKeys, sessionKeys, skillKeys, fanghubKeys } from "../queries/keys";

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>(
    "../http/client",
  );
  return {
    ...actual,
    completeExperiment: vi.fn().mockResolvedValue({}),
    setSessionLabel: vi.fn().mockResolvedValue({}),
    installSkill: vi.fn().mockResolvedValue({}),
  };
});

describe("useCompleteExperiment", () => {
  it("invalidates experiments and experimentMetrics keys", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const { result } = renderHook(() => useCompleteExperiment(), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    });

    const variables = { experimentId: "exp-1", agentId: "agent-1" };
    await result.current.mutateAsync(variables);

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experiments("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experimentMetrics("exp-1"),
    });
  });
});

describe("useSetSessionLabel", () => {
  it("with agentId invalidates session lists and agent sessions", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionLabel(), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    });

    await result.current.mutateAsync({
      sessionId: "sess-1",
      label: "test label",
      agentId: "agent-1",
    });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
  });

  it("without agentId invalidates only session lists", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionLabel(), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    });

    await result.current.mutateAsync({ sessionId: "sess-1", label: "test label" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(1);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });
});

describe("useInstallSkill", () => {
  it("invalidates skillKeys.all and fanghubKeys.all", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const { result } = renderHook(() => useInstallSkill(), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    });

    await result.current.mutateAsync({ name: "test-skill" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: fanghubKeys.all,
    });
  });

  it("invalidates skillKeys.all and fanghubKeys.all with hand parameter", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const { result } = renderHook(() => useInstallSkill(), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    });

    await result.current.mutateAsync({ name: "test-skill", hand: "test-hand" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: fanghubKeys.all,
    });
  });
});
