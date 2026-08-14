'use strict';

const assert = require('node:assert/strict');
const {
  collectRefs,
  commentDisposition,
  parseLookback,
} = require('../auto-close-resolved-issues.js');

assert.equal(parseLookback('14'), 14);
for (const invalid of ['', '0', '-1', '1.5', '14days', '366', 'NaN']) {
  assert.throws(() => parseLookback(invalid), /integer from 1 to 365/);
}

const negations = [
  "doesn't fix #1",
  "don't close #2",
  "didn't resolve #3",
  "isn't fixed #4",
  "aren't closing #5",
  "wasn't resolved #6",
  "weren't fixes #7",
  "couldn't close #8",
  "shouldn't fix #9",
  "hasn't resolved #10",
  "haven't closed #11",
  "won't fix #12",
  "wouldn't close #13",
  "can't resolve #14",
  'cannot fix #15',
  'never close #16',
  'not resolved #17',
];
const log = `0123456789abcdef\n${negations.join('\n')}\nFixes #21\nCloses #22\n---COMMIT-END---`;
assert.deepEqual([...collectRefs(log).keys()], [21, 22]);

assert.deepEqual(commentDisposition(false, []), { skipIssue: false, addComment: true });
assert.deepEqual(commentDisposition(false, ['<!-- auto-close-reconciler:flagged -->']), {
  skipIssue: true,
  addComment: false,
});
assert.deepEqual(commentDisposition(true, ['<!-- auto-close-reconciler:flagged -->']), {
  skipIssue: false,
  addComment: true,
});
assert.deepEqual(commentDisposition(true, ['<!-- auto-close-reconciler:closing -->']), {
  skipIssue: false,
  addComment: false,
});

console.log('auto-close-resolved-issues tests passed');
