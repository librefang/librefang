// The three user-avatar write hooks (#8339). What is under test is which cached
// reads each one invalidates — or, for the delete, drops outright: an avatar
// that changes on the daemon but not in the cache is exactly the bug an `<img>`
// pointed at a stale object URL produces, and it stays invisible until the
// drawer is reopened.
//
// The `whoami` key is the one worth reading twice. The chat bubble draws the
// caller's emoji from it, it does not hang off `userKeys` at all, and the hook
// is addressed by name so it cannot tell whether the name it was handed is the
// caller's — which makes it both the easiest invalidation to leave out and the
// hardest to notice going missing.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import * as http from "../http/client";
import {
  useDeleteUserAvatar,
  useUpdateUserIdentity,
  useUploadUserAvatar,
} from "./users";
import { authzKeys, userKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  updateUserIdentity: vi.fn().mockResolvedValue({ status: "ok" }),
  uploadUserAvatar: vi.fn().mockResolvedValue({ status: "ok" }),
  deleteUserAvatar: vi.fn().mockResolvedValue({ status: "ok", removed: 1 }),
}));

const USER = "alice";

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useUpdateUserIdentity", () => {
  it("invalidates the reads that carry the emoji, `whoami` included", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateUserIdentity(), { wrapper });

    await result.current.mutateAsync({ name: USER, emoji: "🦊" });

    expect(invalidate).toHaveBeenCalledWith({ queryKey: userKeys.detail(USER) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: userKeys.lists() });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: authzKeys.whoami() });
  });
});

describe("useUploadUserAvatar", () => {
  it("sends the blob to the named user's route", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useUploadUserAvatar(), { wrapper });
    const file = new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" });

    await result.current.mutateAsync({ name: USER, file });

    expect(http.uploadUserAvatar).toHaveBeenCalledWith(USER, file);
  });

  it("invalidates the cached image as well as the reads that report one", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUploadUserAvatar(), { wrapper });

    await result.current.mutateAsync({
      name: USER,
      file: new Blob([new Uint8Array([1])], { type: "image/png" }),
    });

    expect(invalidate).toHaveBeenCalledWith({ queryKey: userKeys.avatar(USER) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: userKeys.detail(USER) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: userKeys.lists() });
  });
});

describe("useDeleteUserAvatar", () => {
  it("takes the name directly and drops the image rather than refetching it", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const remove = vi.spyOn(queryClient, "removeQueries");
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteUserAvatar(), { wrapper });

    await result.current.mutateAsync(USER);

    expect(vi.mocked(http.deleteUserAvatar).mock.calls[0][0]).toBe(USER);
    // Dropped, not invalidated: a refetch after a delete spends a request to be
    // told there is nothing there, and that answer is already known.
    expect(remove).toHaveBeenCalledWith({ queryKey: userKeys.avatar(USER) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: userKeys.detail(USER) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: userKeys.lists() });
  });
});
