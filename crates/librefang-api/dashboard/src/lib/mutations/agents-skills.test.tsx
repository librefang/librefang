import { describe, it, expect, vi } from "vitest";
import * as http from "../http/client";
import { renderHook } from "@testing-library/react";
import { useSetAgentSkills } from "./agents";
import { agentKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

// Issue #4917 — inline skill assignment mutation. A PUT must invalidate the
// per-agent skill read, the agent detail (skills / skills_mode are echoed on
// it), and the agent list (row-level skill chips).

vi.mock("../http/client", () => ({
  setAgentSkills: vi.fn().mockResolvedValue({ status: "ok", skills: [] }),
}));

describe("useSetAgentSkills", () => {
  it("PUTs the new allowlist and invalidates skills, detail, and lists", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetAgentSkills(), { wrapper });

    await result.current.mutateAsync({
      agentId: "agent-1",
      skills: ["web-search"],
    });

    expect(http.setAgentSkills).toHaveBeenCalledWith("agent-1", [
      "web-search",
    ]);
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.skills("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
  });
});
