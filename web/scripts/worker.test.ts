import { describe, expect, it, vi } from 'vitest';
// @ts-expect-error — Cloudflare Pages worker, plain .js without types
import worker from '../public/_worker.js';

type AssetsBinding = { fetch: ReturnType<typeof vi.fn> };

// Cloudflare's ASSETS.fetch accepts Request | URL | string. Mock handles all
// three so we mirror the real binding contract.
function assetKey(arg: Request | URL | string): string {
  if (typeof arg === 'string') return new URL(arg).pathname;
  if (arg instanceof URL) return arg.pathname;
  return new URL(arg.url).pathname;
}

function makeEnv(responses: Record<string, Response>): { ASSETS: AssetsBinding } {
  const fetchMock = vi.fn(async (arg: Request | URL | string) => {
    const res = responses[assetKey(arg)];
    if (res) return res;
    return new Response('not found', { status: 404 });
  });
  return { ASSETS: { fetch: fetchMock } };
}

function calledPaths(mock: ReturnType<typeof vi.fn>): string[] {
  return mock.mock.calls.map((c) => assetKey(c[0]));
}

function req(path: string, userAgent = ''): Request {
  return new Request(`https://librefang.ai${path}`, {
    headers: userAgent ? { 'user-agent': userAgent } : {},
  });
}

describe('Cloudflare Pages _worker.js — /install UA routing', () => {
  it('curl gets install.sh content, not HTML', async () => {
    const env = makeEnv({
      '/install.sh': new Response('#!/bin/sh\necho ok\n', {
        status: 200,
        headers: { 'content-type': 'application/x-sh' },
      }),
    });

    const res = await worker.fetch(req('/install', 'curl/8.4.0'), env);

    expect(res.status).toBe(200);
    expect(await res.text()).toMatch(/^#!\/bin\/sh/);
    expect(env.ASSETS.fetch).toHaveBeenCalledTimes(1);
    expect(calledPaths(env.ASSETS.fetch)[0]).toBe('/install.sh');
  });

  it('wget gets install.sh content', async () => {
    const env = makeEnv({
      '/install.sh': new Response('#!/bin/sh\n', { status: 200 }),
    });
    const res = await worker.fetch(req('/install', 'Wget/1.21.3'), env);
    expect(res.status).toBe(200);
    expect(await res.text()).toMatch(/^#!/);
  });

  it('PowerShell 7 gets install.ps1 content', async () => {
    const env = makeEnv({
      '/install.ps1': new Response('# powershell install', { status: 200 }),
    });
    const ua =
      'Mozilla/5.0 (Windows NT; Microsoft Windows 10.0.19045.3448; en-US) PowerShell/7.4.0';
    const res = await worker.fetch(req('/install', ua), env);
    expect(res.status).toBe(200);
    expect(await res.text()).toBe('# powershell install');
    expect(calledPaths(env.ASSETS.fetch)[0]).toBe('/install.ps1');
  });

  it('Windows PowerShell 5.1 gets install.ps1, not install.sh (Mozilla prefix must not fool CLI regex)', async () => {
    const env = makeEnv({
      '/install.ps1': new Response('# ps1', { status: 200 }),
      '/install.sh': new Response('#!/bin/sh', { status: 200 }),
    });
    const ua =
      'Mozilla/5.0 (Windows NT; Microsoft Windows 10.0.19045.3448; en-US) WindowsPowerShell/5.1.19041.3803';
    await worker.fetch(req('/install', ua), env);
    expect(calledPaths(env.ASSETS.fetch)[0]).toBe('/install.ps1');
  });

  it('browser UA falls through to SPA (does NOT get a shell script)', async () => {
    const env = makeEnv({
      '/': new Response('<!doctype html><html>spa</html>', {
        status: 200,
        headers: { 'content-type': 'text/html' },
      }),
    });
    const ua =
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
    const res = await worker.fetch(req('/install', ua), env);
    expect(res.status).toBe(200);
    const body = await res.text();
    expect(body).toContain('<!doctype html>');
    // Confirm we did NOT serve install.sh to a browser.
    const paths = calledPaths(env.ASSETS.fetch);
    expect(paths).not.toContain('/install.sh');
    expect(paths).not.toContain('/install.ps1');
  });

  it('/install with no user-agent falls through to SPA (no false positive)', async () => {
    const env = makeEnv({
      '/': new Response('<!doctype html>', { status: 200 }),
    });
    const res = await worker.fetch(req('/install'), env);
    expect(res.status).toBe(200);
    expect(calledPaths(env.ASSETS.fetch)).not.toContain('/install.sh');
  });

  it('/install.sh direct request is unaffected by the rewrite path', async () => {
    const env = makeEnv({
      '/install.sh': new Response('#!/bin/sh\n', {
        status: 200,
        headers: { 'content-type': 'application/x-sh' },
      }),
    });
    const res = await worker.fetch(req('/install.sh', 'curl/8.4.0'), env);
    expect(res.status).toBe(200);
    expect(await res.text()).toMatch(/^#!\/bin\/sh/);
    // Exactly one asset fetch, straight to /install.sh (no rewrite hop).
    expect(env.ASSETS.fetch).toHaveBeenCalledTimes(1);
    expect(calledPaths(env.ASSETS.fetch)[0]).toBe('/install.sh');
  });

  it('unrelated path with curl UA is not misrouted to install.sh', async () => {
    const env = makeEnv({
      '/about': new Response('<!doctype html>about', { status: 200 }),
    });
    await worker.fetch(req('/about', 'curl/8.4.0'), env);
    const paths = calledPaths(env.ASSETS.fetch);
    expect(paths).not.toContain('/install.sh');
    expect(paths).toContain('/about');
  });
});
