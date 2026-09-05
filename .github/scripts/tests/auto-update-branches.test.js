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
  assert.deepEqual(calls[0].params, {
    owner: 'o',
    repo: 'r',
    state: 'open',
    per_page: 100,
  });
  assert.equal(calls[1].endpoint, checksEndpoint);
  assert.deepEqual(calls[1].params, {
    owner: 'o',
    repo: 'r',
    ref: 'abc',
    filter: 'latest',
    per_page: 100,
  });
  const gate = (conclusion) => ({ name: 'CI Gate', conclusion });

  assert.equal(hasFailingStatus([gate('timed_out')], 'success'), true);
  assert.equal(hasFailingStatus([gate('stale')], 'success'), true);
  assert.equal(hasFailingStatus([gate('success')], 'failure'), true);
  assert.equal(hasFailingStatus([], 'error'), true);
  assert.equal(hasFailingStatus([gate('neutral')], 'success'), false);
  assert.equal(hasFailingStatus([gate(null)], 'pending'), false);

  // A timeout-killed required check reports as `cancelled`, not `failure`, and
  // must still count — this is the signal the updater exists to react to.
  assert.equal(hasFailingStatus([gate('cancelled')], 'success'), true);

  // The regression this narrowing exists for: an advisory check that cannot
  // block a merge must not be able to trigger a branch update either. Two
  // bookkeeping workflows sharing one concurrency bucket left exactly this
  // `cancelled` check on nearly every open PR, which turned each scheduled
  // sweep into a full-repository CI cascade.
  assert.equal(
    hasFailingStatus([{ name: 'stale', conclusion: 'cancelled' }], 'success'),
    false,
  );
  assert.equal(
    hasFailingStatus([{ name: 'Issue-PR Link Labels', conclusion: 'cancelled' }], 'success'),
    false,
  );
  // An advisory check that genuinely failed is still not a merge blocker.
  assert.equal(
    hasFailingStatus([{ name: 'cargo-deny (advisories)', conclusion: 'failure' }], 'success'),
    false,
  );
  // Mixed: the advisory noise is ignored, the required check decides.
  assert.equal(
    hasFailingStatus(
      [{ name: 'stale', conclusion: 'cancelled' }, gate('success')],
      'success',
    ),
    false,
  );
  assert.equal(
    hasFailingStatus(
      [{ name: 'stale', conclusion: 'cancelled' }, gate('failure')],
      'success',
    ),
    true,
  );
  // A check run with no name (older payloads / hand-built fixtures) is not a
  // required check and must not decide anything.
  assert.equal(hasFailingStatus([{ conclusion: 'failure' }], 'success'), false);
  console.log('auto-update-branches tests passed');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
