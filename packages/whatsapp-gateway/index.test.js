'use strict';

const assert = require('node:assert/strict');
const { PassThrough } = require('node:stream');
const test = require('node:test');

const {
  MAX_BODY_SIZE,
  buildCorsHeaders,
  parseBody,
} = require('./index.js');

test('buildCorsHeaders only allows local origins', () => {
  assert.deepEqual(buildCorsHeaders('https://example.com'), {});
  assert.equal(
    buildCorsHeaders('http://localhost:4545')['Access-Control-Allow-Origin'],
    'http://localhost:4545',
  );
});

test('parseBody parses valid JSON', async () => {
  const req = new PassThrough();
  const promise = parseBody(req);
  req.end('{"ok":true}');
  assert.deepEqual(await promise, { ok: true });
});

test('parseBody rejects invalid JSON with 400', async () => {
  const req = new PassThrough();
  const promise = parseBody(req);
  req.end('{bad json');
  await assert.rejects(promise, (err) => err.statusCode === 400);
});

test('parseBody rejects oversized payloads with 413', async () => {
  const req = new PassThrough();
  const promise = parseBody(req);
  req.write('{"data":"');
  req.write('x'.repeat(MAX_BODY_SIZE));
  req.end('"}');
  await assert.rejects(promise, (err) => err.statusCode === 413);
});
