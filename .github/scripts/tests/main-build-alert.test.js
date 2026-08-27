'use strict';

const assert = require('node:assert/strict');
const {
  compareRunOrder,
  escapeInlineCode,
  escapeMarkdownText,
  isRedConclusion,
  rustTestLaneSucceeded,
  selectLatestConclusiveRun,
  selectOpenAlertIssues,
} = require('../main-build-alert.js');

for (const conclusion of [
  'failure',
  'timed_out',
  'cancelled',
  'startup_failure',
  'action_required',
  'stale',
]) {
  assert.equal(isRedConclusion(conclusion), true, conclusion);
}
for (const conclusion of ['success', 'neutral', 'skipped']) {
  assert.equal(isRedConclusion(conclusion), false, conclusion);
}

assert.equal(
  compareRunOrder(
    { run_number: 12, run_attempt: 1 },
    { run_number: 11, run_attempt: 9 },
  ) > 0,
  true,
);
assert.equal(
  compareRunOrder(
    { run_number: 12, run_attempt: 2 },
    { run_number: 12, run_attempt: 1 },
  ) > 0,
  true,
);
assert.equal(rustTestLaneSucceeded([
  { name: 'Test / Ubuntu (shard 1/4)', conclusion: 'success' },
]), true);
assert.equal(rustTestLaneSucceeded([
  { name: 'Quality', conclusion: 'success' },
  { name: 'Test / Unit (lib+bin)', conclusion: 'skipped' },
]), false);

assert.deepEqual(
  selectOpenAlertIssues([
    { number: 1 },
    { number: 2, pull_request: { url: 'https://api.github.test/pulls/2' } },
    { number: 3 },
  ]).map(issue => issue.number),
  [1, 3],
);

const escaped = escapeMarkdownText('@maintainers [fake](https://example.test) `code` *bold*');
assert.equal(escaped.includes('@maintainers'), false);
assert.equal(escaped.includes('[fake]'), false);
assert.equal(escaped.includes('`code`'), false);
assert.equal(escapeInlineCode('run `danger` for @team'), "run 'danger' for @\u200bteam");
assert.equal(
  escapeMarkdownText('job\n- [forged](https://example.test)\r@team'),
  'job \\- \\[forged\\]\\(https://example\\.test\\) @\u200bteam',
);
assert.equal(
  escapeInlineCode('step\n```\r@team'),
  "step ''' @\u200bteam",
);

async function testRunSelection() {
  const jobsByRun = new Map([
    [1, [{ name: 'Test / Unit (lib+bin)', conclusion: 'success' }]],
    [2, []],
    [3, [{ name: 'Quality', conclusion: 'success' }]],
  ]);
  const loadJobs = async run => jobsByRun.get(run.id) || [];
  const docsThenRed = await selectLatestConclusiveRun([
    { id: 2, run_number: 2, conclusion: 'failure' },
    { id: 3, run_number: 3, conclusion: 'success' },
    { id: 1, run_number: 1, conclusion: 'success' },
  ], loadJobs);
  assert.equal(docsThenRed.run.id, 2);
  assert.equal(docsThenRed.state, 'red');

  const greenThenRed = await selectLatestConclusiveRun([
    { id: 1, run_number: 3, conclusion: 'success' },
    { id: 2, run_number: 2, conclusion: 'failure' },
  ], loadJobs);
  assert.equal(greenThenRed.run.id, 1);
  assert.equal(greenThenRed.state, 'green');

  const inconclusive = await selectLatestConclusiveRun([
    { id: 3, run_number: 4, conclusion: 'success' },
    { id: 4, run_number: 3, conclusion: 'skipped' },
  ], loadJobs);
  assert.equal(inconclusive, null);
}

testRunSelection().then(() => console.log('main-build-alert tests passed'));
