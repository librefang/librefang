'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { createDedupTracker } = require('../lib/dedup-tracker');

test('wasProcessed is false on first sight and does NOT mark', () => {
  const t = createDedupTracker();
  assert.equal(t.wasProcessed('abc'), false);
  assert.equal(t.wasProcessed('abc'), false, 're-check still false without mark');
});

test('markProcessed then wasProcessed returns true', () => {
  const t = createDedupTracker();
  t.markProcessed('abc');
  assert.equal(t.wasProcessed('abc'), true);
});

test('unmarkProcessed clears an entry so retry is admitted', () => {
  const t = createDedupTracker();
  t.markProcessed('abc');
  t.unmarkProcessed('abc');
  assert.equal(t.wasProcessed('abc'), false);
});

test('empty / missing id is never processed', () => {
  const t = createDedupTracker();
  assert.equal(t.wasProcessed(''), false);
  assert.equal(t.wasProcessed(undefined), false);
  assert.equal(t.wasProcessed(null), false);
});

test('mark/unmark with empty id is a no-op', () => {
  const t = createDedupTracker();
  t.markProcessed('');
  t.markProcessed(undefined);
  assert.equal(t.size(), 0);
  t.unmarkProcessed(null);
  assert.equal(t.size(), 0);
});

test('entries older than windowMs are pruned on check', () => {
  let clock = 1_000_000;
  const t = createDedupTracker({ windowMs: 100, now: () => clock });
  t.markProcessed('abc');
  assert.equal(t.wasProcessed('abc'), true);
  clock += 50;
  assert.equal(t.wasProcessed('abc'), true, 'still inside window');
  clock += 60; // now 110 ms past mark
  assert.equal(t.wasProcessed('abc'), false, 'pruned after window');
});

test('prune only evicts stale entries', () => {
  let clock = 1_000_000;
  const t = createDedupTracker({ windowMs: 100, now: () => clock });
  t.markProcessed('old');
  clock += 80;
  t.markProcessed('fresh');
  clock += 50; // old=130, fresh=50
  assert.equal(t.wasProcessed('old'), false);
  assert.equal(t.wasProcessed('fresh'), true);
});

// Regression test for the outage described in the module docstring:
// retransmitted msgId after a decrypt failure must reach the recovery branch.
test('retransmit of an unmarked (decrypt-failed) id is not blocked', () => {
  const t = createDedupTracker();
  // First arrival: we check wasProcessed but decrypt fails, so we never
  // call markProcessed.
  assert.equal(t.wasProcessed('signal-fail-1'), false);
  // WA retransmits — we must still see it as not-processed so the handler
  // can invoke session recovery.
  assert.equal(t.wasProcessed('signal-fail-1'), false);
  // Eventually decrypt succeeds on a later retry → mark.
  t.markProcessed('signal-fail-1');
  // Any further retransmits of the same id are now deduped.
  assert.equal(t.wasProcessed('signal-fail-1'), true);
});
