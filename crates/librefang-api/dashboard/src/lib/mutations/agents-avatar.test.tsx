// The three agent-identity write hooks (#8339). What is under test is which
// cached reads each one invalidates — or, for the avatar delete, drops outright:
// an avatar that changes on the server but not in the cache is exactly the bug
// an `<img>` pointed at a stale object URL produces, and it is invisible until
// someone re-opens the drawer.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import * as http from "../http/client";
import {
  useDeleteAgentAvatar,
  useUpdateAgentIdentity,
  useUploadAgentAvatar,
} from "./agents";
import { agentKeys, overviewKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  updateAgentIdentity: vi.fn().mockResolvedValue({ status: "ok" }),
  uploadAgentAvatar: vi.fn().mockResolvedValue({ status: "ok" }),
  deleteAgentAvatar: vi.fn().mockResolvedValue({ status: "ok", removed: 1 }),
}));

const AGENT = "agent-1";

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useUpdateAgentIdentity", () => {
  it("forwards the partial identity untouched", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useUpdateAgentIdentity(), { wrapper });

    await result.current.mutateAsync({ agentId: AGENT, identity: { emoji: "🤖" } });

    expect(http.updateAgentIdentity).toHaveBeenCalledWith(AGENT, { emoji: "🤖" });
  });

  it("invalidates every read that carries the identity, the snapshot included", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateAgentIdentity(), { wrapper });

    await result.current.mutateAsync({ agentId: AGENT, identity: { emoji: "🤖" } });

    expect(invalidate).toHaveBeenCalledWith({ queryKey: agentKeys.detail(AGENT) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: agentKeys.lists() });
    // The list rows render the emoji out of the dashboard snapshot, and its key
    // is a sibling of `agentKeys.all` rather than a child of it — so the two
    // agent keys above leave the row on the previous emoji.
    expect(invalidate).toHaveBeenCalledWith({ queryKey: overviewKeys.snapshot() });
  });

  it("leaves the cached avatar image alone — an emoji change is not a new image", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateAgentIdentity(), { wrapper });

    await result.current.mutateAsync({ agentId: AGENT, identity: { emoji: "🤖" } });

    expect(invalidate).not.toHaveBeenCalledWith({ queryKey: agentKeys.avatar(AGENT) });
  });
});

describe("useUploadAgentAvatar", () => {
  it("sends the blob to the agent's avatar route", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useUploadAgentAvatar(), { wrapper });
    const file = new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" });

    await result.current.mutateAsync({ agentId: AGENT, file });

    expect(http.uploadAgentAvatar).toHaveBeenCalledWith(AGENT, file);
  });

  it("invalidates the cached image as well as the reads that carry avatar_url", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUploadAgentAvatar(), { wrapper });

    await result.current.mutateAsync({
      agentId: AGENT,
      file: new Blob([new Uint8Array([1])], { type: "image/png" }),
    });

    // Without this one the drawer keeps rendering the previous image from the
    // cached Blob until the entry goes stale, so the upload looks like it did
    // nothing.
    expect(invalidate).toHaveBeenCalledWith({ queryKey: agentKeys.avatar(AGENT) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: agentKeys.detail(AGENT) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: agentKeys.lists() });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: overviewKeys.snapshot() });
  });

  it("invalidates nothing when the upload fails", async () => {
    vi.mocked(http.uploadAgentAvatar).mockRejectedValueOnce(new Error("415"));
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUploadAgentAvatar(), { wrapper });

    await expect(
      result.current.mutateAsync({
        agentId: AGENT,
        file: new Blob([new Uint8Array([1])], { type: "image/png" }),
      }),
    ).rejects.toThrow("415");

    // A refused upload changed nothing on disk; re-fetching would spend a
    // request to arrive back at the bytes already cached.
    expect(invalidate).not.toHaveBeenCalledWith({ queryKey: agentKeys.avatar(AGENT) });
  });
});

describe("useDeleteAgentAvatar", () => {
  it("takes the agent id directly and invalidates the image and the reads", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteAgentAvatar(), { wrapper });

    await result.current.mutateAsync(AGENT);

    // First argument only: the hook passes `deleteAgentAvatar` as the
    // `mutationFn` itself, and TanStack hands a bare function the mutation
    // context as a second argument. `deleteAgentAvatar` takes one parameter and
    // ignores it, which is the same shape `useResetAgentSession` already has.
    expect(vi.mocked(http.deleteAgentAvatar).mock.calls[0][0]).toBe(AGENT);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: agentKeys.detail(AGENT) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: agentKeys.lists() });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: overviewKeys.snapshot() });
  });

  it("drops the cached image rather than leaving it for a query that is now disabled", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const blob = new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" });
    queryClient.setQueryData(agentKeys.avatar(AGENT), blob);

    const { result } = renderHook(() => useDeleteAgentAvatar(), { wrapper });
    await result.current.mutateAsync(AGENT);

    // `useAgentAvatarUrl` gates on `hasAvatar`, and the `detail` refetch this
    // same `onSuccess` triggers is what clears `avatar_url` and turns the query
    // off. A disabled `useQuery` still hands back its cached `data`, and an
    // invalidation on a disabled query never becomes a refetch — so invalidating
    // this key would leave the deleted image rendering until the entry happened
    // to be collected. The URL has to stop existing, not merely go stale.
    expect(queryClient.getQueryData(agentKeys.avatar(AGENT))).toBeUndefined();
  });

  it("scopes the avatar removal to the agent it was called for", async () => {
    // Asserted on `removeQueries`, which is the call that carries the removal.
    // Spying on `invalidateQueries` here would pass whatever the scoping did,
    // because the delete path stopped calling it for this key — the assertion
    // would hold on a mutation that removed every agent's avatar.
    const { queryClient, wrapper } = createQueryClientWrapper();
    const remove = vi.spyOn(queryClient, "removeQueries");
    const { result } = renderHook(() => useDeleteAgentAvatar(), { wrapper });

    await result.current.mutateAsync(AGENT);

    expect(remove).toHaveBeenCalledWith({ queryKey: agentKeys.avatar(AGENT) });
    expect(remove).not.toHaveBeenCalledWith({ queryKey: agentKeys.avatar("agent-2") });
    expect(remove).not.toHaveBeenCalledWith({ queryKey: agentKeys.all });
  });
});
