import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import {
  pickLatestSessionId,
  deriveDropdownActiveSessionId,
  pickSessionDropdownLabel,
  shouldAutoPinResolvedSession,
} from "./sessionSelector";
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

describe("deriveDropdownActiveSessionId", () => {
  it("returns the session id when the URL is pinned", () => {
    expect(deriveDropdownActiveSessionId("session-abc")).toBe("session-abc");
  });

  it("returns undefined when urlSessionId is null (unpinned connection)", () => {
    expect(deriveDropdownActiveSessionId(null)).toBeUndefined();
  });

  it("returns undefined when urlSessionId is undefined", () => {
    expect(deriveDropdownActiveSessionId(undefined)).toBeUndefined();
  });

  it("returns the value as-is — callers are responsible for not passing empty strings", () => {
    // The function passes through whatever the URL param contains.
    expect(deriveDropdownActiveSessionId("some-id")).toBe("some-id");
  });
});

describe("pickSessionDropdownLabel (issue #5199-C)", () => {
  const sessions: SessionListItem[] = [
    { session_id: "abcd1234ef56", agent_id: "a", label: "My Session" },
    { session_id: "deadbeefcafe", agent_id: "a" },
  ];

  it("returns null when the active session is undefined (unpinned)", () => {
    // The caller is responsible for rendering the localized "Unpinned"
    // hint string — keeping i18n out of this pure helper.
    expect(pickSessionDropdownLabel(undefined, sessions)).toBeNull();
  });

  it("returns the session's label when one exists in the list", () => {
    expect(pickSessionDropdownLabel("abcd1234ef56", sessions)).toBe("My Session");
  });

  it("falls back to the first 8 chars of the id when the session has no label", () => {
    expect(pickSessionDropdownLabel("deadbeefcafe", sessions)).toBe("deadbeef");
  });

  it("returns the short id prefix even when the session list does not contain a match", () => {
    // Active session may not have surfaced in the per-agent list yet
    // (just-created, server reload pending). Show what we know rather
    // than nothing.
    expect(pickSessionDropdownLabel("12345678abcd", [])).toBe("12345678");
    expect(pickSessionDropdownLabel("12345678abcd", undefined)).toBe("12345678");
  });

  it("prefers a non-empty label even if it would be longer than 8 chars", () => {
    // Sanity check that we're not accidentally truncating user-visible
    // labels — that's a different bug than the original placeholder fix.
    const long = [
      { session_id: "x", agent_id: "a", label: "A very long human-friendly label" },
    ];
    expect(pickSessionDropdownLabel("x", long)).toBe("A very long human-friendly label");
  });

  it("returns null when the id is somehow empty (defensive — caller hides spinner)", () => {
    // deriveDropdownActiveSessionId already guards against this on the
    // URL path; the test pins the contract so a future refactor does
    // not silently render an empty <span>.
    expect(pickSessionDropdownLabel("", sessions)).toBeNull();
  });
});

describe("shouldAutoPinResolvedSession (issue #5199 — WS + HTTP shared gate)", () => {
  const base = {
    sendAgentId: "agent-a",
    currentAgentId: "agent-a",
    currentSessionId: null,
    urlSessionId: null,
    resolvedSessionId: "11111111-2222-3333-4444-555555555555",
  };

  it("auto-pins on the happy path (unpinned URL, same agent, no manual pin)", () => {
    expect(shouldAutoPinResolvedSession(base)).toBe(true);
  });

  it("does NOT pin when the user navigated to a different agent mid-flight", () => {
    // Codex P1 scenario for the WS path; HTTP fallback inherits the same
    // window because `await mutateAsync` similarly yields the event loop.
    expect(
      shouldAutoPinResolvedSession({ ...base, currentAgentId: "agent-b" }),
    ).toBe(false);
  });

  it("does NOT pin when the user manually pinned a session between send and response", () => {
    // Round-1 review concern: `ws.close()` is async, a queued frame can
    // still reach the about-to-detach listener before close takes effect.
    // The HTTP path has the same window through the awaited mutation.
    expect(
      shouldAutoPinResolvedSession({ ...base, currentSessionId: "manually-pinned" }),
    ).toBe(false);
  });

  it("does NOT pin when the request itself was pinned with ?sessionId=", () => {
    // Server already mirrors this by omitting `session_id` from the body
    // when the request supplied one, but the client guard is defense in
    // depth: a regression that flips the server back to echoing would
    // not silently rewrite the URL on top of a deliberately-pinned tab.
    expect(
      shouldAutoPinResolvedSession({ ...base, urlSessionId: "explicit-pin" }),
    ).toBe(false);
  });

  it("does NOT pin when the server omitted session_id (request was pinned, response carries no body field)", () => {
    expect(
      shouldAutoPinResolvedSession({ ...base, resolvedSessionId: undefined }),
    ).toBe(false);
  });

  it("does NOT pin on a null session_id (defensive — pre-#5199 servers could emit null)", () => {
    expect(
      shouldAutoPinResolvedSession({ ...base, resolvedSessionId: null }),
    ).toBe(false);
  });

  it("does NOT pin on an empty-string session_id (defensive — would land on bare ?sessionId=)", () => {
    expect(
      shouldAutoPinResolvedSession({ ...base, resolvedSessionId: "" }),
    ).toBe(false);
  });

  it("does NOT pin on a non-string session_id (defensive — protects against malformed wire data)", () => {
    expect(
      shouldAutoPinResolvedSession({ ...base, resolvedSessionId: 42 }),
    ).toBe(false);
    expect(
      shouldAutoPinResolvedSession({ ...base, resolvedSessionId: { id: "x" } }),
    ).toBe(false);
  });

  it("narrows resolvedSessionId to string when the guard returns true (type predicate)", () => {
    const args = { ...base };
    if (shouldAutoPinResolvedSession(args)) {
      // If the guard's type predicate is wrong this will fail to compile,
      // not at runtime — but documenting the contract here makes the
      // assumption explicit. `args.resolvedSessionId.toUpperCase()` would
      // be a `unknown` operation without the narrowing.
      expect(args.resolvedSessionId.toUpperCase()).toBe(base.resolvedSessionId.toUpperCase());
    } else {
      throw new Error("guard must have returned true on the happy-path base");
    }
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
