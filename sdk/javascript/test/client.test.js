"use strict";

const assert = require("node:assert/strict");
const { afterEach, test } = require("node:test");

const { LibreFang, LibreFangError } = require("../index.js");

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
});

test("resource methods send requests with headers, query parameters, and JSON bodies", async () => {
  const requests = [];
  globalThis.fetch = async (url, options) => {
    requests.push({ url, options });
    return new Response(JSON.stringify({ agent_id: "agent-1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const client = new LibreFang("http://localhost:4545/", {
    headers: { Authorization: "Bearer test-token" },
  });

  const result = await client.agents.spawnAgent({ name: "test-agent" });

  assert.deepEqual(result, { agent_id: "agent-1" });
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, "http://localhost:4545/api/agents");
  assert.equal(requests[0].options.method, "POST");
  assert.equal(requests[0].options.headers.Authorization, "Bearer test-token");
  assert.equal(requests[0].options.headers["Content-Type"], "application/json");
  assert.equal(requests[0].options.body, JSON.stringify({ name: "test-agent" }));

  await client.agents.listAgents({ limit: 10, cursor: undefined });
  assert.equal(requests[1].url, "http://localhost:4545/api/agents?limit=10");
  assert.equal(requests[1].options.method, "GET");
  assert.equal(requests[1].options.body, undefined);
});

test("HTTP failures reject with LibreFangError details", async () => {
  globalThis.fetch = async () => new Response("service unavailable", { status: 503 });
  const client = new LibreFang("http://localhost:4545");

  await assert.rejects(
    client.system.health(),
    (error) => {
      assert.ok(error instanceof LibreFangError);
      assert.equal(error.status, 503);
      assert.equal(error.body, "service unavailable");
      assert.match(error.message, /HTTP 503/);
      return true;
    },
  );
});
