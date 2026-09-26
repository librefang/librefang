// `useAgentAvatarUrl` (#8339). The thing worth guarding is not that a URL comes
// out — it is the object-URL lifecycle. `URL.createObjectURL` hands back a
// document-scoped handle that lives until it is revoked, so a drawer that stays
// mounted while the user clicks through a list of agents leaks one per click
// unless the effect cleans up.
//
// The request itself is gated on the drawer being open: selecting an agent in
// the list (or the desktop auto-select on first paint) must not download an
// image whose only consumer is not mounted.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import * as http from "../http/client";
import { useAgentAvatarUrl } from "./agents";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  fetchAuthenticatedImage: vi.fn(),
  agentAvatarPath: (id: string) => `/api/agents/${encodeURIComponent(id)}/avatar`,
  // The module's other exports are pulled in by `agents.ts`'s import list, so
  // every one it names has to exist on the mock or the import throws.
  listAgents: vi.fn(),
  getAgentDetail: vi.fn(),
  getAgentStats: vi.fn(),
  listAgentEvents: vi.fn(),
  listAgentSessions: vi.fn(),
  listAgentTemplates: vi.fn(),
  listPromptVersions: vi.fn(),
  listExperiments: vi.fn(),
  getExperimentMetrics: vi.fn(),
  loadAgentSession: vi.fn(),
  getAgentSessionContext: vi.fn(),
  listTools: vi.fn(),
  getAgentTools: vi.fn(),
  getAgentSkills: vi.fn(),
  getAgentMcpServers: vi.fn(),
  getAgentChannels: vi.fn(),
}));

let created: string[];
let revoked: string[];
let counter: number;

beforeEach(() => {
  vi.clearAllMocks();
  created = [];
  revoked = [];
  counter = 0;
  vi.stubGlobal("URL", {
    ...URL,
    createObjectURL: vi.fn(() => {
      const url = `blob:test/${++counter}`;
      created.push(url);
      return url;
    }),
    revokeObjectURL: vi.fn((url: string) => {
      revoked.push(url);
    }),
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function pngBlob() {
  return new Blob([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], { type: "image/png" });
}

describe("useAgentAvatarUrl", () => {
  it("does not request anything for an agent with no avatar", async () => {
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useAgentAvatarUrl("agent-1", false), { wrapper });

    expect(result.current).toBeUndefined();
    expect(http.fetchAuthenticatedImage).not.toHaveBeenCalled();
  });

  it("does not request anything without an agent id", () => {
    const { wrapper } = createQueryClientWrapper();

    renderHook(() => useAgentAvatarUrl("", true), { wrapper });

    expect(http.fetchAuthenticatedImage).not.toHaveBeenCalled();
  });

  it("does not request the blob while the drawer that renders it is closed", async () => {
    const { wrapper } = createQueryClientWrapper();

    // AgentsPage keeps `detailAgent` selected after the drawer closes, so
    // without the third gate the auto-selected agent's image is downloaded and
    // kept for a header that is not mounted.
    const { result } = renderHook(() => useAgentAvatarUrl("agent-1", true, false), { wrapper });

    expect(result.current).toBeUndefined();
    expect(http.fetchAuthenticatedImage).not.toHaveBeenCalled();
    expect(created).toEqual([]);
  });

  it("fetches when the drawer opens, and lets the URL go when it closes", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(pngBlob());
    const { wrapper } = createQueryClientWrapper();

    const { result, rerender } = renderHook(
      ({ open }: { open: boolean }) => useAgentAvatarUrl("agent-1", true, open),
      { wrapper, initialProps: { open: false } },
    );
    expect(http.fetchAuthenticatedImage).not.toHaveBeenCalled();

    rerender({ open: true });
    await waitFor(() => expect(result.current).toBe("blob:test/1"));

    // Re-closing must not leave the handle alive: the blob stays in the query
    // cache, but nothing renders it until the drawer reopens.
    rerender({ open: false });
    await waitFor(() => expect(result.current).toBeUndefined());
    expect(revoked).toEqual(["blob:test/1"]);
  });

  it("fetches the agent's own avatar path and returns an object URL for it", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(pngBlob());
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useAgentAvatarUrl("agent-1", true), { wrapper });

    await waitFor(() => expect(result.current).toBe("blob:test/1"));
    // The request must carry React Query's signal: without it a superseded
    // fetch is not cancelled and holds a connection slot until it ends.
    expect(http.fetchAuthenticatedImage).toHaveBeenCalledWith(
      "/api/agents/agent-1/avatar",
      expect.any(AbortSignal),
    );
  });

  it("revokes the object URL on unmount", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(pngBlob());
    const { wrapper } = createQueryClientWrapper();

    const { result, unmount } = renderHook(() => useAgentAvatarUrl("agent-1", true), { wrapper });
    await waitFor(() => expect(result.current).toBe("blob:test/1"));

    unmount();

    expect(revoked).toEqual(["blob:test/1"]);
  });

  it("revokes the previous URL when the drawer switches to another agent", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(pngBlob());
    const { wrapper } = createQueryClientWrapper();

    const { result, rerender } = renderHook(
      ({ id }: { id: string }) => useAgentAvatarUrl(id, true),
      { wrapper, initialProps: { id: "agent-1" } },
    );
    await waitFor(() => expect(result.current).toBe("blob:test/1"));

    rerender({ id: "agent-2" });
    await waitFor(() => expect(result.current).toBe("blob:test/2"));

    // One created per agent, and the first one let go. Without the cleanup this
    // is `[]` and the handle lives for the rest of the page's life.
    expect(created).toEqual(["blob:test/1", "blob:test/2"]);
    expect(revoked).toEqual(["blob:test/1"]);
  });

  it("stays undefined when the image cannot be fetched, so the initials show", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockRejectedValue(new Error("404"));
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useAgentAvatarUrl("agent-1", true), { wrapper });

    await waitFor(() => expect(http.fetchAuthenticatedImage).toHaveBeenCalled());
    expect(result.current).toBeUndefined();
    expect(created).toEqual([]);
  });
});
