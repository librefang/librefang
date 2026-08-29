'use strict';

const assert = require('node:assert/strict');
const {
  collectRefs,
  commentDisposition,
  parseLookback,
  sanitizeInlineCode,
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
const sha = '0123456789abcdef0123456789abcdef01234567';
const log = `${sha}\0Fixes #20 with ---COMMIT-END---\0${negations.join('\n')}\nFixes #21\nCloses #22\n\0`;
assert.deepEqual([...collectRefs(log).keys()], [20, 21, 22]);
assert.deepEqual(
  collectRefs(`${sha}\0subject\0Fixes #999999999999999999999999\0`).size,
  0,
);
assert.throws(() => collectRefs(`${sha}\0subject only\0`), /triples/);
assert.equal(
  sanitizeInlineCode('unsafe `code`\n@maintainers'),
  "unsafe 'code' @\u200bmaintainers",
);

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
