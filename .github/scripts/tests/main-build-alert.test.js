'use strict';

const assert = require('node:assert/strict');
const {
  escapeInlineCode,
  escapeMarkdownText,
  isRedConclusion,
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

console.log('main-build-alert tests passed');
