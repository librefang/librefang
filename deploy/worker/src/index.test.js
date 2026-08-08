import assert from 'node:assert/strict';
import test from 'node:test';

import worker from './index.js';

function successfulFlyFetches(onMachineRequest = () => {}) {
  return async (url, options = {}) => {
    if (String(url).endsWith('/machines')) {
      onMachineRequest(JSON.parse(options.body));
      return new Response(JSON.stringify({ id: 'machine-1' }), { status: 200 });
    }
    return new Response('{}', { status: 200 });
  };
}

test('deploy uses the caller OpenRouter key and never the worker shared key', async () => {
  const originalFetch = globalThis.fetch;
  let machinePayload;
  globalThis.fetch = successfulFlyFetches((payload) => {
    machinePayload = payload;
  });

  try {
    const request = new Request('https://deploy.librefang.ai/api/deploy', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        token: 'fly-user-token',
        openrouterApiKey: 'user-openrouter-key',
      }),
    });
    const response = await worker.fetch(request, {
      OPENROUTER_API_KEY: 'worker-shared-key-must-not-leak',
    });

    assert.equal(response.status, 200);
    assert.ok(!(await response.text()).includes('user-openrouter-key'));
    assert.equal(
      machinePayload.config.env.OPENROUTER_API_KEY,
      'user-openrouter-key',
    );
    assert.ok(!JSON.stringify(machinePayload).includes('worker-shared-key-must-not-leak'));
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('deploy page collects and submits the caller OpenRouter key', async () => {
  const response = await worker.fetch(new Request('https://deploy.librefang.ai/'));
  const html = await response.text();

  assert.equal(response.status, 200);
  assert.match(html, /id="openrouterApiKey"/);
  assert.match(html, /JSON\.stringify\(\{ token, openrouterApiKey \}\)/);
  assert.ok(!html.includes('No API key needed'));
  assert.ok(!html.includes('no API keys required'));
});

test('deploy rejects a missing caller OpenRouter key before contacting Fly', async () => {
  const originalFetch = globalThis.fetch;
  let fetchCount = 0;
  globalThis.fetch = async () => {
    fetchCount += 1;
    return new Response('{}', { status: 200 });
  };

  try {
    const request = new Request('https://deploy.librefang.ai/api/deploy', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token: 'fly-user-token' }),
    });
    const response = await worker.fetch(request, {
      OPENROUTER_API_KEY: 'worker-shared-key-must-not-be-a-fallback',
    });
    const body = await response.json();

    assert.equal(response.status, 400);
    assert.match(body.error, /OpenRouter API Key is required/);
    assert.equal(fetchCount, 0);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
