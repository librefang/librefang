import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as httpClient from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import { sessionKeys } from "./keys";
import { useSessions } from "./sessions";

vi.mock("../http/client", () => ({
  listSessions: vi.fn(),
  getSessionDetails: vi.fn(),
}));

describe("useSessions", () => {
  beforeEach(() => vi.clearAllMocks());

  it("reacts when truncation changes without an item-list change", async () => {
    const items = [{ session_id: "session-1" }];
    vi.mocked(httpClient.listSessions).mockResolvedValue({ items, truncated: false });
    const { queryClient, wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useSessions(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(items);
    expect(result.current.truncated).toBe(false);

    act(() => {
      queryClient.setQueryData(sessionKeys.lists(), { items, truncated: true });
    });

    await waitFor(() => expect(result.current.truncated).toBe(true));
    expect(result.current.data).toEqual(items);
  });
});
