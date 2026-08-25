import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useAgentMcpServers } from "./agents";
import * as httpClient from "../http/client";
import { agentKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

// Issue #7713 — per-agent MCP server assignment. The Tools tab reads
// { assigned, available, mode, pending } from GET /api/agents/{id}/mcp_servers
// through this hook. `pending` is the half that has no other consumer: a
// declared server with no live connection contributes no tools, so it forms no
// tool group and is invisible everywhere else on the page.

vi.mock("../http/client", () => ({
  getAgentMcpServers: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useAgentMcpServers", () => {
  it("is disabled (no fetch) when agentId is empty", () => {
    const { result } = renderHook(() => useAgentMcpServers(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getAgentMcpServers).not.toHaveBeenCalled();
  });

  it("fetches and caches under agentKeys.mcpServers(id), pending included", async () => {
    const payload = {
      assigned: ["ghost-mcp", "live-mcp"],
      available: ["live-mcp"],
      mode: "allowlist" as const,
      pending: ["ghost-mcp"],
    };
    vi.mocked(httpClient.getAgentMcpServers).mockResolvedValue(payload);
    const { queryClient, wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useAgentMcpServers("agent-1"), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(payload);
    expect(httpClient.getAgentMcpServers).toHaveBeenCalledWith("agent-1");
    expect(queryClient.getQueryData(agentKeys.mcpServers("agent-1"))).toEqual(
      payload,
    );
  });

  it("keys MCP reads on their own subtree, not the skills one", () => {
    expect(agentKeys.mcpServers("agent-1")).not.toEqual(
      agentKeys.skills("agent-1"),
    );
  });
});
