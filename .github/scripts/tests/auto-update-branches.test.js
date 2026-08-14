'use strict';

const assert = require('node:assert/strict');
const {
  hasFailingStatus,
  listCheckRuns,
  listOpenPulls,
} = require('../auto-update-branches.js');

const calls = [];
const pullsEndpoint = Symbol('pulls.list');
const checksEndpoint = Symbol('checks.listForRef');
const github = {
  rest: {
    pulls: { list: pullsEndpoint },
    checks: { listForRef: checksEndpoint },
  },
  async paginate(endpoint, params) {
    calls.push({ endpoint, params });
    return endpoint === pullsEndpoint ? [{ number: 101 }, { number: 202 }] : [{ conclusion: 'failure' }];
  },
};

(async () => {
  assert.deepEqual(await listOpenPulls(github, { owner: 'o', repo: 'r' }), [
    { number: 101 },
    { number: 202 },
  ]);
  assert.deepEqual(await listCheckRuns(github, { owner: 'o', repo: 'r' }, 'abc'), [
    { conclusion: 'failure' },
  ]);
  assert.equal(calls[0].endpoint, pullsEndpoint);
  assert.equal(calls[0].params.per_page, 100);
  assert.equal(calls[1].endpoint, checksEndpoint);
  assert.equal(calls[1].params.ref, 'abc');
  assert.equal(hasFailingStatus([{ conclusion: 'timed_out' }], 'success'), true);
  assert.equal(hasFailingStatus([{ conclusion: 'success' }], 'failure'), true);
  assert.equal(hasFailingStatus([], 'error'), true);
  assert.equal(hasFailingStatus([{ conclusion: null }], 'pending'), false);
  console.log('auto-update-branches tests passed');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
