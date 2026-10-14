// Regression for #8424's ETag refresh.
//
// The dashboard query client defaults every query to a 30 s `staleTime`, and
// the page's refresh used to be a bare `qc.fetchQuery(agentQueries.manifest())`
// — inside that window it returned the cached snapshot, so a manifest write the
// form did not make never moved the token the form echoed on save. The next
// form save then hit the server's version check and was refused with 409.
//
// These tests run the real query client at the production 30 s staleTime, the
// real `useSetAgentSkills` mutation and the real `fetchManifestVersion` helper
// against a fetch mock that enforces the server-side check (409 on a mismatch),
// so "the token moved" and "the save was accepted" are observed behaviour, not
// source-string assertions.
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { agentQueries } from "./queries/agents";
import { fetchManifestVersion, rebaseManifestEdits } from "./manifestVersion";
import { parseManifestToml, serializeManifestForm } from "./agentManifest";
import { useSetAgentSkills } from "./mutations/agents";
import { patchAgent } from "../api";

const AGENT_ID = "agent-etag";

interface FakeServer {
  version: number;
  skills: string[];
  patchExpectedVersions: Array<string | undefined>;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function currentToken(server: FakeServer): string {
  return `manifest-v${server.version}`;
}

/** A fetch mock that behaves like the manifest routes: the ETag tracks the
 * manifest content, and every manifest write that carries an
 * `expected_version` is refused with 409 when it is not current. */
function installServer(): FakeServer {
  const server: FakeServer = { version: 1, skills: [], patchExpectedVersions: [] };
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = (init?.method ?? "GET").toUpperCase();
    if (url.endsWith(`/api/agents/${AGENT_ID}/manifest`) && method === "GET") {
      return new Response(`name = "etag-${server.version}"\n`, {
        status: 200,
        headers: { ETag: `"${currentToken(server)}"` },
      });
    }
    if (url.endsWith(`/api/agents/${AGENT_ID}/skills`) && method === "PUT") {
      const body = JSON.parse(String(init?.body)) as {
        skills: string[];
        expected_version?: string;
      };
      if (body.expected_version && body.expected_version !== currentToken(server)) {
        return json({ error: "Stale manifest" }, 409);
      }
      server.skills = body.skills;
      server.version += 1;
      return json({ status: "ok", skills: server.skills });
    }
    if (url.endsWith(`/api/agents/${AGENT_ID}`) && method === "PATCH") {
      const body = JSON.parse(String(init?.body)) as {
        manifest_toml?: string;
        expected_version?: string;
      };
      server.patchExpectedVersions.push(body.expected_version);
      if (body.expected_version && body.expected_version !== currentToken(server)) {
        return json({ error: "Stale manifest" }, 409);
      }
      server.version += 1;
      return json({ status: "ok" });
    }
    throw new Error(`unexpected ${method} ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return server;
}

function makeQueryClient(): QueryClient {
  // The production default, not the `staleTime: 0` shorthand other tests use:
  // a refresh that only works because nothing is ever fresh would not catch
  // this bug.
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 30_000 } },
  });
}

function wrapper(qc: QueryClient) {
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  return Wrapper;
}

describe("manifest ETag refresh (#8424)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("re-reads the token when another writer changed the manifest inside the 30 s staleTime", async () => {
    const server = installServer();
    const qc = makeQueryClient();

    // The editor seeds its token from the manifest read; the query is now
    // fresh for 30 s.
    const seed = await qc.fetchQuery(agentQueries.manifest(AGENT_ID));
    expect(seed.version).toBe("manifest-v1");

    // A writer this tab knows nothing about (another tab, the file watcher,
    // a prompt hot-swap) moves the manifest without invalidating this cache.
    server.version = 2;
    server.skills = ["alpha"];

    // The refresh the page runs after a write it did not make must go to the
    // server; the plain fetchQuery returned the cached v1 here.
    await expect(fetchManifestVersion(qc, AGENT_ID)).resolves.toBe("manifest-v2");

    qc.clear();
  });

  it("panel save → token moves → form save built on it is accepted", async () => {
    const server = installServer();
    const qc = makeQueryClient();

    const seed = await qc.fetchQuery(agentQueries.manifest(AGENT_ID));
    expect(seed.version).toBe("manifest-v1");

    // The grant panel saves through its own endpoint...
    const { result } = renderHook(() => useSetAgentSkills(), { wrapper: wrapper(qc) });
    await act(async () => {
      await result.current.mutateAsync({ agentId: AGENT_ID, skills: ["alpha"] });
    });
    expect(server.skills).toEqual(["alpha"]);

    // ...and the page re-reads the token through the same helper it uses.
    const token = await fetchManifestVersion(qc, AGENT_ID);
    expect(token).toBe("manifest-v2");

    // The form save built on that token is accepted, where the pre-panel
    // token is refused with 409 by the mock's server-side check.
    await act(async () => {
      await expect(
        patchAgent(AGENT_ID, {
          manifest_toml: `name = "etag-${server.version}"\n`,
          expected_version: token ?? undefined,
        }),
      ).resolves.toBeTruthy();
      await expect(
        patchAgent(AGENT_ID, {
          manifest_toml: `name = "etag-1"\n`,
          expected_version: "manifest-v1",
        }),
      ).rejects.toMatchObject({ status: 409 });
    });
    expect(server.patchExpectedVersions).toEqual(["manifest-v2", "manifest-v1"]);

    qc.clear();
  });
});

// #8516: adopting a fresh token is only safe together with the content it describes.
// These run the real parser and serializer, so "the other writer's change survives the retry" is observed on the TOML that would be sent, not on the merge in isolation.
describe("rebaseManifestEdits (#8516)", () => {
  const SEED = `name = "rebase"
description = "seed"

[compaction]
threshold_messages = 10
keep_recent = 4

[metadata]
team = "core"
`;

  function parseOk(toml: string) {
    const parsed = parseManifestToml(toml);
    if (!parsed.ok) throw new Error(`fixture must parse: ${parsed.message}`);
    return parsed;
  }

  it("keeps a concurrent writer's change when the form is saved again after a 409", () => {
    const seed = parseOk(SEED);
    // The operator edits the description in the form.
    const mine = { ...seed.form, description: "mine" };
    // Meanwhile another writer changed a different field and saved.
    const theirs = parseOk(SEED.replace("keep_recent = 4", "keep_recent = 9"));

    const form = rebaseManifestEdits(seed.form, mine, theirs.form);
    const extras = rebaseManifestEdits(seed.extras, seed.extras, theirs.extras);
    const resent = parseOk(serializeManifestForm(form, extras));

    expect(resent.form.description).toBe("mine");
    // Adopting the token alone would have resent the seed's `keep_recent = 4`.
    expect(resent.form.compaction.keep_recent).toBe("9");
    expect(resent.form.compaction.threshold_messages).toBe("10");
  });

  it("merges a table key by key, so an edit to one knob does not pin its siblings", () => {
    const seed = parseOk(SEED);
    const mine = {
      ...seed.form,
      compaction: { ...seed.form.compaction, threshold_messages: "20" },
    };
    const theirs = parseOk(SEED.replace("keep_recent = 4", "keep_recent = 9"));

    const form = rebaseManifestEdits(seed.form, mine, theirs.form);
    expect(form.compaction.threshold_messages).toBe("20");
    expect(form.compaction.keep_recent).toBe("9");
  });

  it("keeps the operator's value when both sides changed the same field", () => {
    const seed = parseOk(SEED);
    const mine = { ...seed.form, description: "mine" };
    const theirs = parseOk(SEED.replace('description = "seed"', 'description = "theirs"'));

    expect(rebaseManifestEdits(seed.form, mine, theirs.form).description).toBe("mine");
  });

  it("adopts the server's content for every field the operator left alone", () => {
    const seed = parseOk(SEED);
    const theirs = parseOk(
      SEED.replace('description = "seed"', 'description = "theirs"').replace(
        'team = "core"',
        'team = "platform"',
      ),
    );

    const form = rebaseManifestEdits(seed.form, seed.form, theirs.form);
    expect(form).toEqual(theirs.form);
  });

  it("does not treat freshly minted row ids as an edit", () => {
    const seed = parseOk(SEED);
    // Same content, different `_uid`s — what a re-render or a re-parse produces.
    const mine = {
      ...seed.form,
      metadata: seed.form.metadata.map((row) => ({ ...row, _uid: `${row._uid}-local` })),
    };
    const theirs = parseOk(SEED.replace('team = "core"', 'team = "platform"'));

    const form = rebaseManifestEdits(seed.form, mine, theirs.form);
    expect(form.metadata.map((row) => row.value)).toEqual(["platform"]);
  });
});
