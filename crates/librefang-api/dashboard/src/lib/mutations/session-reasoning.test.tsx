import { describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useSetSessionReasoning } from "./sessions";
import { setSessionReasoningOverride } from "../http/client";
import { agentKeys, sessionKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>(
    "../http/client",
  );
  return {
    ...actual,
    setSessionReasoningOverride: vi.fn().mockResolvedValue({}),
  };
});

// The session pin mirrors the session model override: one PATCH against the
// session, then every cache that renders that session is invalidated.

describe("useSetSessionReasoning", () => {
  it("patches the pinned level and invalidates the session caches", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionReasoning(), { wrapper });
    await result.current.mutateAsync({
      sessionId: "sess-1",
      reasoningEffort: "xhigh",
      agentId: "agent-1",
    });

    expect(setSessionReasoningOverride).toHaveBeenCalledWith("sess-1", "xhigh");
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: sessionKeys.detail("sess-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: sessionKeys.lists() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: agentKeys.sessions("agent-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessionSnapshots("agent-1"),
    });
  });

  it("clears the pin with null, without an agent to fan out to", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionReasoning(), { wrapper });
    await result.current.mutateAsync({
      sessionId: "sess-1",
      reasoningEffort: null,
      agentId: null,
    });

    expect(setSessionReasoningOverride).toHaveBeenCalledWith("sess-1", null);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: sessionKeys.detail("sess-1") });
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
  });
});
