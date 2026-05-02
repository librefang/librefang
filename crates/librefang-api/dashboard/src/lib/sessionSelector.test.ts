import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { pickLatestSessionId } from "./sessionSelector";
import { useAgentSessions } from "./queries/agents";
import * as httpClient from "./http/client";
import { createQueryClientWrapper } from "./test/query-client";
import type { SessionListItem } from "../api";

vi.mock("./http/client", () => ({
  listAgentSessions: vi.fn(),
  listSessions: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("pickLatestSessionId", () => {
  it("returns undefined for an empty or missing list", () => {
    expect(pickLatestSessionId(undefined)).toBeUndefined();
    expect(pickLatestSessionId([])).toBeUndefined();
  });

  it("returns the session with the newest created_at", () => {
    const list: SessionListItem[] = [
      { session_id: "older", agent_id: "a1", created_at: "2026-01-01T00:00:00Z" },
      { session_id: "newest", agent_id: "a1", created_at: "2026-04-01T00:00:00Z" },
      { session_id: "middle", agent_id: "a1", created_at: "2026-02-15T00:00:00Z" },
    ];
    expect(pickLatestSessionId(list)).toBe("newest");
  });

  it("treats sessions without created_at as epoch 0 but still returns one if alone", () => {
    const list: SessionListItem[] = [
      { session_id: "no-ts", agent_id: "a1" },
    ];
    expect(pickLatestSessionId(list)).toBe("no-ts");
  });

  it("prefers any timestamped session over an undated one", () => {
    const list: SessionListItem[] = [
      { session_id: "no-ts", agent_id: "a1" },
      { session_id: "dated", agent_id: "a1", created_at: "2020-01-01T00:00:00Z" },
    ];
    expect(pickLatestSessionId(list)).toBe("dated");
  });
});

// Regression test for #4294: Conversation tab MUST source its session list
// from the per-agent endpoint (/api/agents/{id}/sessions via listAgentSessions),
// NOT the global /api/sessions which is capped at 50 rows. If a future change
// re-routes the Conversation tab to the global endpoint, this test will fail
// because the per-agent endpoint will not be hit.
describe("Conversation tab data source (issue #4294)", () => {
  it("useAgentSessions hits the per-agent endpoint, not the global sessions list", async () => {
    const agentSpecific: SessionListItem[] = [
      // Simulate sessions that would NOT appear in the global /api/sessions
      // top-50 because 50 newer sessions for other agents pushed them off.
      { session_id: "agent-1-newest", agent_id: "agent-1", created_at: "2026-04-01T00:00:00Z" },
      { session_id: "agent-1-older", agent_id: "agent-1", created_at: "2026-03-01T00:00:00Z" },
    ];
    vi.mocked(httpClient.listAgentSessions).mockResolvedValue(agentSpecific);

    const { result } = renderHook(() => useAgentSessions("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    // Selector picks the newest from the per-agent list — even though the
    // global list (mocked to throw if called) would have hidden these rows.
    expect(pickLatestSessionId(result.current.data)).toBe("agent-1-newest");
    expect(httpClient.listAgentSessions).toHaveBeenCalledWith("agent-1");
    expect(httpClient.listSessions).not.toHaveBeenCalled();
  });
});
