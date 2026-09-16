// The api.ts half of the avatar UI (#8339): the authenticated-image allowlist,
// and the three request shapes the routes in
// `crates/librefang-api/src/routes/agents/avatar.rs` expect.
//
// The allowlist is shared between the agent's avatar and the signed-in user's,
// so both arms are pinned here and read together: an arm that goes missing is a
// silent failure in the browser, not a compile error.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  agentAvatarPath,
  currentUserAvatarPath,
  deleteAgentAvatar,
  fetchAuthenticatedImage,
  isAuthenticatedImagePath,
  updateAgentIdentity,
  uploadAgentAvatar,
} from "./api";

const AGENT_ID = "11111111-1111-1111-1111-111111111111";

describe("isAuthenticatedImagePath", () => {
  it("admits an agent avatar path", () => {
    expect(isAuthenticatedImagePath(agentAvatarPath(AGENT_ID))).toBe(true);
  });

  it("keeps admitting the two path shapes that predate the avatar route", () => {
    expect(isAuthenticatedImagePath("/api/uploads/abc_123-x")).toBe(true);
    expect(isAuthenticatedImagePath("/api/media/artifacts/abc_123-x")).toBe(true);
  });

  it("admits the signed-in user's avatar path, which is the only user path there is", () => {
    expect(isAuthenticatedImagePath(currentUserAvatarPath())).toBe(true);
    // Both halves, on purpose: `currentUserAvatarPath()` is what the query
    // calls, but the arm that has to match it is a literal, so a refactor that
    // routed the builder through something else would leave the two out of step
    // while still returning a truthy string.
    expect(currentUserAvatarPath()).toBe("/api/users/me/avatar");
  });

  // The refusal is as load-bearing as the admission: it is *why* the appearance
  // editor is drawn for the caller's own row and no other, and why the dashboard
  // never fetches another operator's picture.
  it.each([
    ["an ASCII name the class would have carried anyway", "/api/users/alice/avatar"],
    ["a name that could not be a path segment", `/api/users/${encodeURIComponent("Juan Pérez")}/avatar`],
    ["a uuid-shaped user id", `/api/users/${AGENT_ID}/avatar`],
  ])("refuses the by-name user avatar path — %s", (_label, path) => {
    // Not an oversight to be fixed by widening the class: `encodeURIComponent`
    // carries `%`, and accepting `%XX` would readmit `%2F`, which decodes to
    // the `/` this allowlist exists to stop. A by-name arm would also fail
    // *selectively* — `alice` would work and `Juan Pérez` would not — which is
    // the kind of bug nobody finds from a screenshot.
    expect(isAuthenticatedImagePath(path)).toBe(false);
  });

  // The point of an allowlist is what it refuses. Each of these would send this
  // origin's bearer token somewhere it was never meant to go, and the first two
  // are the shapes an id built from user input could take.
  it.each([
    ["a traversal in the id segment", `/api/agents/../../secrets/avatar`],
    ["a second path segment after the id", `/api/agents/${AGENT_ID}/sessions/avatar`],
    ["a sibling route under the same agent", `/api/agents/${AGENT_ID}/memory`],
    ["the avatar route without the suffix", `/api/agents/${AGENT_ID}`],
    ["a trailing segment after avatar", `/api/agents/${AGENT_ID}/avatar/raw`],
    ["an absolute URL to another origin", "https://example.invalid/avatar.png"],
    ["a protocol-relative URL", `//example.invalid/api/agents/${AGENT_ID}/avatar`],
  ])("refuses %s", (_label, path) => {
    expect(isAuthenticatedImagePath(path)).toBe(false);
  });

  it("refuses to fetch a path it would not admit, before any request goes out", async () => {
    const fetchMock = vi.fn();
    Object.defineProperty(globalThis, "fetch", { configurable: true, value: fetchMock });

    await expect(fetchAuthenticatedImage("https://example.invalid/x.png")).rejects.toThrow(
      /not allowed/i,
    );
    expect(fetchMock).not.toHaveBeenCalled();

    vi.restoreAllMocks();
  });
});

describe("agent avatar requests", () => {
  const fetchMock = vi.fn();

  beforeEach(() => {
    fetchMock.mockReset();
    Object.defineProperty(globalThis, "fetch", { configurable: true, value: fetchMock });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function jsonResponse(body: unknown) {
    return new Response(JSON.stringify(body), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }

  it("posts the image as the raw body, with no filename anywhere", async () => {
    fetchMock.mockResolvedValue(
      jsonResponse({
        status: "ok",
        avatar_url: agentAvatarPath(AGENT_ID),
        content_type: "image/png",
        bytes: 4,
      }),
    );
    const file = new File([new Uint8Array([1, 2, 3, 4])], "../../etc/cron.d/x.png", {
      type: "image/png",
    });

    await expect(uploadAgentAvatar(AGENT_ID, file)).resolves.toMatchObject({
      avatar_url: `/api/agents/${AGENT_ID}/avatar`,
    });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(`/api/agents/${AGENT_ID}/avatar`);
    expect(init.method).toBe("POST");
    expect(init.body).toBe(file);

    // The route reads the bytes and nothing else. This asserts we do not
    // reintroduce the `X-Filename` header `uploadAgentFile` sends — that name
    // is the thing the backend deliberately never has.
    const headers = new Headers(init.headers);
    expect(headers.get("X-Filename")).toBeNull();
    expect(headers.get("Content-Type")).toBe("application/octet-stream");
    expect(JSON.stringify(init)).not.toContain("cron.d");
  });

  it("deletes through the avatar route", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ status: "ok", removed: 1 }));

    await deleteAgentAvatar(AGENT_ID);

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(`/api/agents/${AGENT_ID}/avatar`);
    expect(init.method).toBe("DELETE");
  });

  it("patches only the identity fields it was given, so the PATCH stays partial", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ status: "ok" }));

    await updateAgentIdentity(AGENT_ID, { emoji: "🤖" });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(`/api/agents/${AGENT_ID}/identity`);
    expect(init.method).toBe("PATCH");
    // Exactly `{ emoji }`. `avatar_url` must never ride along: the route
    // accepts it, and a stray one here would be the free-text URL write #8349
    // exists to close. `color` must not either, or omitting it from the UI
    // would silently clear what someone set elsewhere.
    expect(JSON.parse(init.body as string)).toEqual({ emoji: "🤖" });
  });

  it("sends an empty string to clear the emoji, because an omitted field means 'leave it'", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ status: "ok" }));

    await updateAgentIdentity(AGENT_ID, { emoji: "" });

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(init.body as string)).toEqual({ emoji: "" });
  });

  it("escapes the id it puts in the path", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ status: "ok" }));

    await deleteAgentAvatar("agent/one");

    expect(fetchMock.mock.calls[0][0]).toBe("/api/agents/agent%2Fone/avatar");
  });
});
