'use strict';

const assert = require('node:assert/strict');
const { applyEventPull, listOpenPulls, removeLabel } = require('../stale-pr.js');

(async () => {
  const listEndpoint = Symbol('pulls.list');
  const calls = [];
  const github = {
    rest: {
      pulls: { list: listEndpoint },
      issues: {
        async removeLabel(params) {
          calls.push(params);
        },
      },
    },
    async paginate(endpoint, params) {
      assert.equal(endpoint, listEndpoint);
      assert.equal(params.per_page, 100);
      return [{ number: 1 }, { number: 101 }];
    },
  };
  assert.deepEqual(await listOpenPulls(github, { owner: 'o', repo: 'r' }), [
    { number: 1 },
    { number: 101 },
  ]);
  assert.deepEqual(
    applyEventPull(
      [{ number: 1, updated_at: 'old' }, { number: 2 }],
      { number: 1, state: 'open', updated_at: 'new' },
    ),
    [{ number: 1, state: 'open', updated_at: 'new' }, { number: 2 }],
  );
  assert.deepEqual(applyEventPull([{ number: 1 }], undefined), [{ number: 1 }]);
  assert.equal(await removeLabel(github, { owner: 'o', repo: 'r' }, 7, 'stale-pr', console), true);
  assert.equal(calls[0].issue_number, 7);

  github.rest.issues.removeLabel = async () => {
    throw Object.assign(new Error('gone'), { status: 404 });
  };
  assert.equal(await removeLabel(github, { owner: 'o', repo: 'r' }, 8, 'stale-pr', console), false);

  const failure = Object.assign(new Error('rate limited'), { status: 429 });
  github.rest.issues.removeLabel = async () => { throw failure; };
  await assert.rejects(
    removeLabel(github, { owner: 'o', repo: 'r' }, 9, 'stale-pr', console),
    (error) => error === failure,
  );
  console.log('stale-pr tests passed');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
