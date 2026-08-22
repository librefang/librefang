'use strict';

const assert = require('node:assert/strict');
const {
  collectReconciliationState,
  extractIssueNumbers,
  getIssueOrNull,
} = require('../issue-pr-links.js');

const repo = { owner: 'librefang', repo: 'librefang' };
const body = [
  'Closes #12',
  'fixes librefang/librefang#34',
  'Resolved https://github.com/librefang/librefang/issues/56',
  'Closes #78abc',
  'Fixes other/repo#90',
  'mentions #99 without a closing keyword',
].join('\n');
assert.deepEqual([...extractIssueNumbers(body, repo)], [12, 34, 56]);
assert.deepEqual([...extractIssueNumbers('CLOSES #7 and fixes #7', repo)], [7]);
assert.deepEqual([...extractIssueNumbers('', repo)], []);
assert.deepEqual([...extractIssueNumbers('Fixes #0 and closes #99999999999999999999', repo)], []);

const reconciliation = collectReconciliationState(
  [
    { number: 42, body: 'Closes #12' },
    { number: 7, body: 'Fixes #34' },
  ],
  [
    { number: 78 },
    { number: 90, pull_request: {} },
  ],
  repo,
  { number: 42, state: 'open', body: 'Resolves #56' },
);
assert.deepEqual([...reconciliation.linkedIssueNumbers], [56, 34]);
assert.deepEqual([...reconciliation.issueNumbers], [56, 34, 78]);
const closedReconciliation = collectReconciliationState(
  [{ number: 42, body: 'Closes #12' }],
  [{ number: 12 }],
  repo,
  { number: 42, state: 'closed', body: 'Closes #12' },
);
assert.deepEqual([...closedReconciliation.linkedIssueNumbers], []);
assert.deepEqual([...closedReconciliation.issueNumbers], [12]);

async function testIssueLookup() {
  const issue = { number: 12 };
  const found = await getIssueOrNull({
    rest: { issues: { get: async () => ({ data: issue }) } },
  }, repo, 12);
  assert.equal(found, issue);

  const missing = await getIssueOrNull({
    rest: { issues: { get: async () => { throw Object.assign(new Error('missing'), { status: 404 }); } } },
  }, repo, 13);
  assert.equal(missing, null);

  const forbidden = Object.assign(new Error('forbidden'), { status: 403 });
  await assert.rejects(
    getIssueOrNull({
      rest: { issues: { get: async () => { throw forbidden; } } },
    }, repo, 14),
    error => error === forbidden,
  );
}

testIssueLookup().then(() => console.log('issue-pr-links tests passed'));
