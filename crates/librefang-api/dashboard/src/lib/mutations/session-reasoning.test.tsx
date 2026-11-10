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

// A pin changes one scalar on the session record. The record also carries the
// full transcript, so refetching it after every pin/unpin would re-download
// the conversation to learn a value the caller already sent. These pin the
// cached-scalar policy that replaced the invalidation.

const DETAIL = {
  session_id: "sess-1",
  messages: [
    { role: "user", content: "hello" },
    { role: "assistant", content: "hi" },
  ],
  reasoning_effort: null as string | null,
};

function seeded() {
  const { queryClient, wrapper } = createQueryClientWrapper();
  queryClient.setQueryData(sessionKeys.detail("sess-1"), structuredClone(DETAIL));
  return { queryClient, wrapper };
}

const cachedEffort = (queryClient: ReturnType<typeof seeded>["queryClient"]) =>
  (queryClient.getQueryData(sessionKeys.detail("sess-1")) as { reasoning_effort?: string | null })
    ?.reasoning_effort;

describe("useSetSessionReasoning", () => {
  it("patches the pinned level and updates the cached scalar in place", async () => {
    const { queryClient, wrapper } = seeded();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionReasoning(), { wrapper });
    await result.current.mutateAsync({
      sessionId: "sess-1",
      reasoningEffort: "xhigh",
      agentId: "agent-1",
    });

    expect(setSessionReasoningOverride).toHaveBeenCalledWith("sess-1", "xhigh");
    expect(cachedEffort(queryClient)).toBe("xhigh");

    // The cheap list caches still refresh…
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: sessionKeys.lists() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: agentKeys.sessions("agent-1") });
    // …but neither full transcript read is invalidated: the scalar above is
    // the only thing the PATCH changed.
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: sessionKeys.detail("sess-1"),
    });
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: agentKeys.sessionSnapshots("agent-1"),
    });
  });

  it("clears the cached pin with null, without an agent to fan out to", async () => {
    const { queryClient, wrapper } = seeded();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionReasoning(), { wrapper });
    await result.current.mutateAsync({
      sessionId: "sess-1",
      reasoningEffort: null,
      agentId: null,
    });

    expect(setSessionReasoningOverride).toHaveBeenCalledWith("sess-1", null);
    expect(cachedEffort(queryClient)).toBeNull();
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: sessionKeys.detail("sess-1"),
    });
  });

  // The route validates and normalizes the token (trim/case); it does not
  // rewrite the level to one the model declares. So the only echo worth
  // reconciling against is the normalized spelling of what was sent.
  it("writes the server's normalized level back when it trims the request", async () => {
    const { queryClient, wrapper } = seeded();
    vi.mocked(setSessionReasoningOverride).mockResolvedValueOnce({
      status: "ok",
      reasoning_effort: "high",
    });

    const { result } = renderHook(() => useSetSessionReasoning(), { wrapper });
    await result.current.mutateAsync({
      sessionId: "sess-1",
      reasoningEffort: " High ",
      agentId: null,
    });

    expect(cachedEffort(queryClient)).toBe("high");
  });

  it("rolls the cached scalar back when the PATCH fails", async () => {
    const { queryClient, wrapper } = seeded();
    queryClient.setQueryData(sessionKeys.detail("sess-1"), {
      ...structuredClone(DETAIL),
      reasoning_effort: "high",
    });
    vi.mocked(setSessionReasoningOverride).mockRejectedValueOnce(new Error("nope"));

    const { result } = renderHook(() => useSetSessionReasoning(), { wrapper });
    await expect(
      result.current.mutateAsync({
        sessionId: "sess-1",
        reasoningEffort: "max",
        agentId: null,
      }),
    ).rejects.toThrow("nope");

    expect(cachedEffort(queryClient)).toBe("high");
  });
});
