#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
const scriptsRoot = new URL("../", import.meta.url);

async function loadWorker(name) {
  const source = await readFile(new URL(`workers/${name}.ts`, scriptsRoot), "utf8");
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(source)}#${name}`);
}

async function withFetch(implementation, action) {
  const original = globalThis.fetch;
  globalThis.fetch = implementation;
  try {
    return await action();
  } finally {
    globalThis.fetch = original;
  }
}

for (const [name, asset, missingMessage] of [
  ["install-ps1", "librefang-x86_64-pc-windows-msvc.zip", "No Windows x86_64 release found"],
  ["install-sh", "librefang-x86_64-unknown-linux-gnu.tar.gz", "No Linux x86_64 release found"],
]) {
  const { onRequest } = await loadWorker(name);

  const redirect = await withFetch(
    async () => new Response(JSON.stringify({
      assets: [{ name: asset, browser_download_url: `https://github.com/librefang/librefang/releases/download/v1/${asset}` }],
    })),
    onRequest,
  );
  assert.equal(redirect.status, 302, `${name}: valid asset should redirect`);
  assert.match(redirect.headers.get("location"), /^https:\/\/github\.com\//);

  for (const fetchFailure of [
    async () => { throw new Error("network secret"); },
    async () => new Response("upstream secret", { status: 429 }),
  ]) {
    const response = await withFetch(fetchFailure, onRequest);
    assert.equal(response.status, 502, `${name}: upstream failure status`);
    assert.equal(await response.text(), "Release service unavailable");
  }

  const invalidJson = await withFetch(
    async () => new Response("not-json"),
    onRequest,
  );
  assert.equal(invalidJson.status, 502, `${name}: invalid JSON status`);
  assert.equal(await invalidJson.text(), "Invalid release service response");

  for (const payload of [
    null,
    {},
    { assets: "wrong" },
  ]) {
    const response = await withFetch(
      async () => new Response(JSON.stringify(payload)),
      onRequest,
    );
    assert.equal(response.status, 502, `${name}: malformed response shape status`);
    assert.equal(await response.text(), "Invalid release service response");
  }

  for (const payload of [
    { assets: [] },
    { assets: [{ name: "different-asset", browser_download_url: "https://github.com/file" }] },
    { assets: [{ name: `prefix-${asset}`, browser_download_url: `https://github.com/librefang/librefang/releases/download/v1/prefix-${asset}` }] },
    { assets: [{ name: `${asset}.sha256`, browser_download_url: `https://github.com/librefang/librefang/releases/download/v1/${asset}.sha256` }] },
  ]) {
    const response = await withFetch(
      async () => new Response(JSON.stringify(payload)),
      onRequest,
    );
    assert.equal(response.status, 404, `${name}: missing asset status`);
    assert.equal(await response.text(), missingMessage);
  }

  for (const browser_download_url of [
    undefined,
    "javascript:alert(1)",
    "https://attacker.example/file",
    `https://github.com:444/librefang/librefang/releases/download/v1/${asset}`,
    `https://github.com/attacker/repository/releases/download/v1/${asset}`,
    `https://github.com/librefang/librefang/releases/download/v1/${asset}?download=1`,
  ]) {
    const response = await withFetch(
      async () => new Response(JSON.stringify({
        assets: [{ name: asset, browser_download_url }],
      })),
      onRequest,
    );
    assert.equal(response.status, 502, `${name}: unsafe matching asset URL status`);
    assert.equal(await response.text(), "Invalid release service response");
  }
}

console.log("OK: install redirect workers");
