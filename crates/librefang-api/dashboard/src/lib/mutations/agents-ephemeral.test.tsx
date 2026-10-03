import { describe, it, expect, vi } from "vitest";
import * as http from "../http/client";
import { renderHook } from "@testing-library/react";
import { useSpawnEphemeral } from "./agents";
import { agentKeys, budgetKeys, usageKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  spawnEphemeral: vi.fn().mockResolvedValue({
    name: "worker-1",
    response: "",
    iterations: 0,
    tools: [],
  }),
}));

/**
 * The worker registers no agent, opens no session and leaves no workspace, so
 * the agent list is legitimately untouched. What it does leave is spend on the
 * parent's ledger, and the three invalidations below are the whole of the
 * hook's contract — the Agents page renders the parent's `Cost · 24h` tile
 * beside the control that starts the run, so a missing `agentKeys.stats` shows
 * the pre-run figure right next to the button that changed it.
 */
describe("useSpawnEphemeral", () => {
  it("bills the parent's stats, and the usage and budget domains", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSpawnEphemeral(), { wrapper });
    await result.current.mutateAsync({ parent: "agent-1", message: "hi" });

    expect(http.spawnEphemeral).toHaveBeenCalledWith({
      parent: "agent-1",
      message: "hi",
    });

    const invalidated = invalidateSpy.mock.calls.map(
      ([arg]) => (arg as { queryKey: unknown }).queryKey,
    );
    expect(invalidated).toEqual([
      agentKeys.stats("agent-1"),
      usageKeys.all,
      budgetKeys.all,
    ]);
  });

  // `onSettled`, not `onSuccess`: a run that failed halfway still spent
  // whatever it spent before it failed, so the ledger has to be re-read either
  // way.
  it("re-reads the ledger after a failed run too", async () => {
    vi.mocked(http.spawnEphemeral).mockRejectedValueOnce(new Error("boom"));

    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSpawnEphemeral(), { wrapper });
    await expect(
      result.current.mutateAsync({ parent: "agent-2", message: "hi" }),
    ).rejects.toThrow("boom");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.stats("agent-2"),
    });
  });
});
