import assert from 'node:assert/strict';
import test from 'node:test';

import worker, { createWorker } from './index.js';

function successfulFlyFetches(onMachineRequest = () => {}) {
  return async (url, options = {}) => {
    if (String(url) === 'https://api.fly.io/graphql') {
      return Response.json({
        data: { allocateIPAddress: { ipAddress: { address: '203.0.113.1' } } },
      });
    }
    if (String(url).endsWith('/machines')) {
      onMachineRequest(JSON.parse(options.body));
      return new Response(JSON.stringify({ id: 'machine-1' }), { status: 200 });
    }
    return new Response('{}', { status: 200 });
  };
}

test('deploy uses the caller OpenRouter key and never the worker shared key', async () => {
  let machinePayload;
  const testWorker = createWorker(successfulFlyFetches((payload) => {
    machinePayload = payload;
  }));

  const request = new Request('https://deploy.librefang.ai/api/deploy', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      token: 'fly-user-token',
      openrouterApiKey: 'user-openrouter-key',
    }),
  });
  const response = await testWorker.fetch(request);

  assert.equal(response.status, 200);
  assert.ok(!(await response.text()).includes('user-openrouter-key'));
  assert.equal(
    machinePayload.config.env.OPENROUTER_API_KEY,
    'user-openrouter-key',
  );
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
  let fetchCount = 0;
  const testWorker = createWorker(async () => {
    fetchCount += 1;
    return new Response('{}', { status: 200 });
  });

  const request = new Request('https://deploy.librefang.ai/api/deploy', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token: 'fly-user-token' }),
  });
  const response = await testWorker.fetch(request);
  const body = await response.json();

  assert.equal(response.status, 400);
  assert.match(body.error, /OpenRouter API Key is required/);
  assert.equal(fetchCount, 0);
});

test('machine creation errors never reflect the caller OpenRouter key', async () => {
  const callerKey = 'user-openrouter-key-must-not-return';
  const testWorker = createWorker(async (url) => {
    if (String(url) === 'https://api.fly.io/graphql') {
      return Response.json({
        data: { allocateIPAddress: { ipAddress: { address: '203.0.113.1' } } },
      });
    }
    if (String(url).endsWith('/machines')) {
      return new Response(`invalid env OPENROUTER_API_KEY=${callerKey}`, {
        status: 400,
      });
    }
    return new Response('{}', { status: 200 });
  });
  const request = new Request('https://deploy.librefang.ai/api/deploy', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token: 'fly-user-token', openrouterApiKey: callerKey }),
  });

  const response = await testWorker.fetch(request);
  const responseText = await response.text();

  assert.equal(response.status, 500);
  assert.ok(!responseText.includes(callerKey));
  assert.ok(!responseText.includes('OPENROUTER_API_KEY='));
});

test('GraphQL allocation errors delete the partially created app', async () => {
  const requests = [];
  const testWorker = createWorker(async (url, options = {}) => {
    requests.push([String(url), options.method || 'GET', options]);
    if (String(url) === 'https://api.fly.io/graphql') {
      return Response.json({ errors: [{ message: 'allocation failed' }] });
    }
    return new Response('{}', { status: 200 });
  });

  const response = await deploy(testWorker);
  const appCreate = requests.find(([url, method]) => url.endsWith('/apps') && method === 'POST');
  const appName = JSON.parse(appCreate[2].body).app_name;

  assert.equal(response.status, 500);
  assert.ok(requests.some(([url, method]) => url.endsWith(`/apps/${appName}`) && method === 'DELETE'));
  assert.ok(!requests.some(([url]) => url.endsWith('/volumes')));
});

test('machine creation failure deletes the partially created app', async () => {
  const requests = [];
  const testWorker = createWorker(async (url, options = {}) => {
    requests.push([String(url), options.method || 'GET', options]);
    if (String(url) === 'https://api.fly.io/graphql') {
      return Response.json({
        data: { allocateIPAddress: { ipAddress: { address: '203.0.113.1' } } },
      });
    }
    if (String(url).endsWith('/machines')) {
      return new Response('machine failed', { status: 500 });
    }
    return new Response('{}', { status: 200 });
  });

  const response = await deploy(testWorker);
  const appRequest = requests.find(([url, method]) => url.endsWith('/apps') && method === 'POST');
  const appName = JSON.parse(appRequest[2].body).app_name;

  assert.equal(response.status, 500);
  assert.ok(requests.some(([url, method]) => url.endsWith(`/apps/${appName}`) && method === 'DELETE'));
});

function deploy(testWorker) {
  return testWorker.fetch(new Request('https://deploy.librefang.ai/api/deploy', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token: 'fly-user-token', openrouterApiKey: 'openrouter-key' }),
  }));
}
