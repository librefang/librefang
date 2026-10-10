import { describe, it, expect, vi } from "vitest";
import * as http from "../http/client";
import { renderHook } from "@testing-library/react";
import { useSetAgentSkills } from "./agents";
import { agentKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

// Issue #4917 — inline skill assignment mutation. A PUT must invalidate the
// per-agent skill read, the agent detail (skills / skills_mode are echoed on
// it), the agent list (row-level skill chips), and the manifest read (#8424:
// the PUT rotates the ETag the manifest editor echoes).

vi.mock("../http/client", () => ({
  setAgentSkills: vi.fn().mockResolvedValue({ status: "ok", skills: [] }),
}));

describe("useSetAgentSkills", () => {
  it("PUTs the new allowlist and invalidates skills, detail, lists, and the manifest", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetAgentSkills(), { wrapper });

    await result.current.mutateAsync({
      agentId: "agent-1",
      skills: ["web-search"],
    });

    // No editor token is in play on this call, hence the explicit `undefined`
    // third argument (#8424).
    expect(http.setAgentSkills).toHaveBeenCalledWith("agent-1", [
      "web-search",
    ], undefined);
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.skills("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.manifest("agent-1"),
    });
  });
});
