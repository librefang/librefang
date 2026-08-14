'use strict';

const assert = require('node:assert/strict');
const {
  escapeInlineCode,
  escapeMarkdownText,
  isRedConclusion,
} = require('../main-build-alert.js');

for (const conclusion of ['failure', 'timed_out', 'cancelled', 'startup_failure']) {
  assert.equal(isRedConclusion(conclusion), true, conclusion);
}
for (const conclusion of ['success', 'neutral', 'skipped', 'action_required']) {
  assert.equal(isRedConclusion(conclusion), false, conclusion);
}

const escaped = escapeMarkdownText('@maintainers [fake](https://example.test) `code` *bold*');
assert.equal(escaped.includes('@maintainers'), false);
assert.equal(escaped.includes('[fake]'), false);
assert.equal(escaped.includes('`code`'), false);
assert.equal(escapeInlineCode('run `danger` for @team'), "run 'danger' for @\u200bteam");

console.log('main-build-alert tests passed');
