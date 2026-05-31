import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useAgentSkills } from "./agents";
import * as httpClient from "../http/client";
import { agentKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

// Issue #4917 — per-agent skill assignment query hook. The Skills tab on the
// agent detail page reads { assigned, available, mode, disabled } from
// GET /api/agents/{id}/skills through this hook.

vi.mock("../http/client", () => ({
  getAgentSkills: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useAgentSkills", () => {
  it("is disabled (no fetch) when agentId is empty", () => {
    const { result } = renderHook(() => useAgentSkills(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getAgentSkills).not.toHaveBeenCalled();
  });

  it("fetches and caches under agentKeys.skills(id)", async () => {
    const payload = {
      assigned: ["web-search"],
      available: ["web-search", "writing-coach"],
      mode: "allowlist" as const,
      disabled: false,
    };
    vi.mocked(httpClient.getAgentSkills).mockResolvedValue(payload);
    const { queryClient, wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useAgentSkills("agent-1"), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(payload);
    expect(httpClient.getAgentSkills).toHaveBeenCalledWith("agent-1");
    expect(queryClient.getQueryData(agentKeys.skills("agent-1"))).toEqual(
      payload,
    );
  });
});
