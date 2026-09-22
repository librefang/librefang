// `useUserAvatarUrl` / `userQueries.avatar` (#8339). The thing worth guarding is
// the split the whole design turns on: the *request path* is the literal
// `/api/users/me/avatar` — `me` is resolved from the bearer credential on the
// daemon side, so no part of it was chosen by a caller — while the *cache key*
// is per-name, because signing in as somebody else has to produce a different
// entry rather than reusing the previous user's picture.
//
// Both halves are asserted below, and the name used is deliberately one that
// could not survive being a path segment. `encodeURIComponent("Juan Pérez")` is
// `Juan%20P%C3%A9rez`, and `%` is not in `AUTHENTICATED_IMAGE_PATH_RE`'s
// character class, so a by-name arm would not just be riskier — it would fail
// to fetch this user's picture at all, and widening the class to carry `%XX`
// would readmit `%2F`, which decodes to the `/` that allowlist exists to stop.
//
// The third thing guarded here is the gate: `hasAvatar` comes from `whoami` and
// decides whether the request is worth making at all, so `false` must mean "do
// not ask" and `undefined` must mean "not told, ask anyway". Collapsing those two
// is how a caller who has a picture ends up never fetching it.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import * as http from "../http/client";
import { userKeys } from "./keys";
import { useUserAvatarUrl } from "./users";
import { createQueryClientWrapper } from "../test/query-client";

// `importActual` rather than a hand-written stub, on purpose: the path builder
// is half of what is under test, so a mocked `currentUserAvatarPath` would only
// assert that the mock returns what the mock returns. The byte fetch is the one
// thing faked.
vi.mock("../http/client", async () => {
  const actual =
    await vi.importActual<typeof import("../http/client")>("../http/client");
  return { ...actual, fetchAuthenticatedImage: vi.fn() };
});

let revoked: string[];

beforeEach(() => {
  vi.clearAllMocks();
  revoked = [];
  vi.stubGlobal("URL", {
    ...URL,
    createObjectURL: vi.fn(() => "blob:test/1"),
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

const NAME = "Juan Pérez";

describe("useUserAvatarUrl", () => {
  it("requests the literal me path, not one built from the name", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(pngBlob());
    const { wrapper } = createQueryClientWrapper();

    renderHook(() => useUserAvatarUrl(NAME, true), { wrapper });

    await waitFor(() => expect(http.fetchAuthenticatedImage).toHaveBeenCalled());
    const path = vi.mocked(http.fetchAuthenticatedImage).mock.calls[0][0];

    expect(path).toBe("/api/users/me/avatar");
    // The discriminating half. A path built from the name would carry `%20` and
    // `%C3%A9` here, and `isAuthenticatedImagePath` would refuse it — so this
    // assertion fails on exactly the change it is there to catch.
    expect(path).not.toContain("%");
    expect(path).not.toContain(NAME);
  });

  it("keys the cached entry on the name, so the next caller does not read the previous picture", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(pngBlob());
    const { queryClient, wrapper } = createQueryClientWrapper();

    const first = renderHook(() => useUserAvatarUrl("alice", true), { wrapper });
    await waitFor(() => expect(first.result.current).toBe("blob:test/1"));

    // A second name on the same client. If the key ignored the name these two
    // would share one entry — and `AVATAR_STALE_MS` (5 minutes) would leave the
    // cached blob in place, so the second request would never happen and
    // `alice`'s picture would be served for `bob`.
    const second = renderHook(() => useUserAvatarUrl("bob", true), { wrapper });
    await waitFor(() =>
      expect(http.fetchAuthenticatedImage).toHaveBeenCalledTimes(2),
    );

    expect(
      queryClient.getQueryData(userKeys.avatar("alice")),
    ).toBeInstanceOf(Blob);
    expect(queryClient.getQueryData(userKeys.avatar("bob"))).toBeInstanceOf(Blob);
    expect(userKeys.avatar("alice")).not.toEqual(userKeys.avatar("bob"));
    // `useObjectUrl` mints the handle in an effect, so the second hook's URL is
    // a paint behind the fetch that filled its cache entry.
    await waitFor(() => expect(second.result.current).toBe("blob:test/1"));
  });

  it("does not request anything without a name", () => {
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useUserAvatarUrl("", true), { wrapper });

    expect(result.current).toBeUndefined();
    expect(http.fetchAuthenticatedImage).not.toHaveBeenCalled();
  });

  it("stays undefined on the 404 that means there is no picture", async () => {
    // `whoami` said there was a picture and by the time the bytes were asked
    // for there was not — a delete in another tab, or a file removed under a
    // restored database. `retry: false` is what keeps that from being three more
    // round trips that cannot turn it into a picture.
    vi.mocked(http.fetchAuthenticatedImage).mockRejectedValue(new Error("404"));
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useUserAvatarUrl(NAME, true), { wrapper });

    await waitFor(() => expect(http.fetchAuthenticatedImage).toHaveBeenCalledTimes(1));
    expect(result.current).toBeUndefined();

    // Give any retry a chance to happen before asserting it did not.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(http.fetchAuthenticatedImage).toHaveBeenCalledTimes(1);
    expect(revoked).toEqual([]);
  });

  it("does not ask when the daemon says there is no picture", () => {
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(() => useUserAvatarUrl(NAME, false), { wrapper });

    // The pair with the test below is the point: if `false` and `undefined` were
    // treated alike, one of them would be wrong, and the wrong direction is a
    // caller who has a picture and never fetches it.
    expect(result.current).toBeUndefined();
    expect(http.fetchAuthenticatedImage).not.toHaveBeenCalled();
  });

  it("asks anyway when the daemon did not say", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(pngBlob());
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(
      () => useUserAvatarUrl(NAME, undefined as unknown as boolean),
      { wrapper },
    );

    await waitFor(() => expect(http.fetchAuthenticatedImage).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(result.current).toBe("blob:test/1"));
  });
});
