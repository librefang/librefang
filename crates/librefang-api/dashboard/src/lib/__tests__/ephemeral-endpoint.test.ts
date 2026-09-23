import { afterEach, describe, it, expect, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { spawnEphemeral } from "../../api";

// `src/lib/__tests__` → `crates/librefang-api/dashboard/src/lib/__tests__`, so
// six levels up is the repository root.
const OPENAPI_PATH = join(
  __dirname, "..", "..", "..", "..", "..", "..", "openapi.json",
);

afterEach(() => {
  vi.unstubAllGlobals();
});

/**
 * The two halves of one claim: the dashboard posts to a path the daemon
 * actually serves.
 *
 * The literal in `api.ts` is otherwise tied to nothing. A test that only
 * asserted the literal would be a second copy of it, and would keep passing
 * while the daemon renamed the route out from under it — the modal would 404
 * at runtime, on the one code path no unit test drives.
 *
 * `openapi.json` is regenerated from the router by the `openapi-drift` CI job,
 * which fails the build when it is out of sync with the source, so the
 * published document is the router's own claim about its paths rather than
 * another hand-maintained list.
 */
describe("POST /api/agents/spawn-ephemeral", () => {
  it("is the path the dashboard posts to, and one the daemon publishes", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({ name: "worker-1", response: "ok", iterations: 1, tools: [] }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    await spawnEphemeral({ parent: "agent-1", message: "hi" });

    const posted = fetchMock.mock.calls[0]?.[0];
    expect(posted).toBe("/api/agents/spawn-ephemeral");

    const document = JSON.parse(readFileSync(OPENAPI_PATH, "utf8")) as {
      paths: Record<string, unknown>;
    };
    expect(Object.keys(document.paths)).toContain(posted);
  });
});
