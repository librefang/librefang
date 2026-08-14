'use strict';

const assert = require('node:assert/strict');
const { extractIssueNumbers } = require('../issue-pr-links.js');

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

console.log('issue-pr-links tests passed');
