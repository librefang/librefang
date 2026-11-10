// `useAgents` over the paginated envelope (#8596).
//
// `listAgents` answers a page plus its `total`, and the pruning consumers
// need the second half to tell "not in the page" from "gone". The hook is
// where the two are split: `data` stays the array every existing call site
// reads, `truncated` carries the flag — the same arrangement `useSessions`
// uses for `listSessions`.

import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as httpClient from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import type { AgentItem } from "../../api";
import { agentKeys } from "./keys";
import { useAgents } from "./agents";

vi.mock("../http/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../http/client")>();
  return { ...actual, listAgents: vi.fn() };
});

describe("useAgents", () => {
  beforeEach(() => vi.clearAllMocks());

  it("keeps `data` an array while surfacing truncation, and reacts to it", async () => {
    const items = [{ id: "agent-a", name: "Agent A" }] as AgentItem[];
    vi.mocked(httpClient.listAgents).mockResolvedValue({ items, truncated: false });
    const { queryClient, wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useAgents(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    // The existing call sites destructure `.data` as `AgentItem[]`; the
    // envelope must not leak into that shared shape.
    expect(result.current.data).toEqual(items);
    expect(result.current.truncated).toBe(false);

    act(() => {
      queryClient.setQueryData(agentKeys.list(), { items, truncated: true });
    });

    await waitFor(() => expect(result.current.truncated).toBe(true));
    expect(result.current.data).toEqual(items);
  });
});
