'use strict';

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');
const { EchoTracker } = require('../lib/echo-tracker');

describe('EchoTracker.normalize', () => {
  it('lowercases and strips trailing punctuation', () => {
    assert.equal(EchoTracker.normalize('Hello!'), 'hello');
    assert.equal(EchoTracker.normalize('World?'), 'world');
    assert.equal(EchoTracker.normalize('done...'), 'done');
  });

  it('collapses whitespace and trims', () => {
    assert.equal(EchoTracker.normalize('  foo   bar  '), 'foo bar');
    assert.equal(EchoTracker.normalize('line\n\nbreak'), 'line break');
  });

  it('strips Extended_Pictographic emojis', () => {
    assert.equal(EchoTracker.normalize('ciao 👋'), 'ciao');
    assert.equal(EchoTracker.normalize('🔥hot🔥'), 'hot');
  });

  it('returns empty string for null/undefined/empty', () => {
    assert.equal(EchoTracker.normalize(null), '');
    assert.equal(EchoTracker.normalize(undefined), '');
    assert.equal(EchoTracker.normalize(''), '');
  });
});

describe('EchoTracker.track / isEcho', () => {
  it('track("Hello!") then isEcho("hello") is true (normalization)', () => {
    const t = new EchoTracker();
    t.track('alice@s.whatsapp.net', 'Hello!');
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'hello'), true);
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'HELLO'), true);
  });

  it('whitespace and case collapse through wiring', () => {
    const t = new EchoTracker();
    t.track('alice@s.whatsapp.net', '  foo   bar  ');
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'FOO BAR'), true);
  });

  it('emoji strip matches echo without emoji', () => {
    const t = new EchoTracker();
    t.track('alice@s.whatsapp.net', 'ciao 👋');
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'ciao'), true);
  });

  it('returns false for never-tracked text', () => {
    const t = new EchoTracker();
    t.track('alice@s.whatsapp.net', 'hello');
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'never-sent'), false);
  });

  it('does not match the same short body in another conversation', () => {
    const t = new EchoTracker();
    t.track('alice@s.whatsapp.net', 'ok');
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'OK!'), true);
    assert.equal(t.isEcho('bob@s.whatsapp.net', 'ok'), false);
  });

  it('track(null), track(""), track(undefined) are no-ops', () => {
    const t = new EchoTracker();
    t.track('alice@s.whatsapp.net', null);
    t.track('alice@s.whatsapp.net', '');
    t.track('alice@s.whatsapp.net', undefined);
    t.track('', 'hello');
    assert.equal(t.size(), 0);
    assert.equal(t.isEcho('alice@s.whatsapp.net', ''), false);
  });
});

describe('EchoTracker LRU eviction', () => {
  it('evicts oldest when size exceeds max (100 default)', () => {
    const t = new EchoTracker(100);
    for (let i = 0; i < 100; i++) t.track('alice@s.whatsapp.net', `msg-${i}`);
    assert.equal(t.size(), 100);
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'msg-0'), true);
    t.track('alice@s.whatsapp.net', 'msg-100');
    assert.equal(t.size(), 100);
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'msg-0'), false, 'oldest should be evicted');
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'msg-100'), true, 'newest should be present');
    assert.equal(t.isEcho('alice@s.whatsapp.net', 'msg-1'), true, 'second-oldest should still be present');
  });

  it('re-tracking refreshes insertion order (prevents premature eviction)', () => {
    const t = new EchoTracker(3);
    t.track('chat', 'a');
    t.track('chat', 'b');
    t.track('chat', 'c');
    t.track('chat', 'a'); // refresh a
    t.track('chat', 'd'); // should evict b, not a
    assert.equal(t.isEcho('chat', 'a'), true);
    assert.equal(t.isEcho('chat', 'b'), false);
    assert.equal(t.isEcho('chat', 'c'), true);
    assert.equal(t.isEcho('chat', 'd'), true);
  });

  it('honors small maxSize', () => {
    const t = new EchoTracker(2);
    t.track('chat', 'x');
    t.track('chat', 'y');
    t.track('chat', 'z');
    assert.equal(t.size(), 2);
    assert.equal(t.isEcho('chat', 'x'), false);
  });
});

describe('EchoTracker TTL', () => {
  it('expires stale entries below the LRU capacity', () => {
    let now = 1_000;
    const t = new EchoTracker(100, 500, () => now);
    t.track('chat', 'hello');
    now += 499;
    assert.equal(t.isEcho('chat', 'hello'), true);
    now += 1;
    assert.equal(t.isEcho('chat', 'hello'), false);
    assert.equal(t.size(), 0);
  });
});

describe('EchoTracker observability', () => {
  it('size() reports current count', () => {
    const t = new EchoTracker();
    assert.equal(t.size(), 0);
    t.track('chat', 'one');
    assert.equal(t.size(), 1);
    t.track('chat', 'two');
    assert.equal(t.size(), 2);
  });

  it('elapsedSinceLastSent returns -1 before any track, positive after', async () => {
    const t = new EchoTracker();
    assert.equal(t.elapsedSinceLastSent(), -1);
    t.track('chat', 'hello');
    await new Promise((r) => setTimeout(r, 5));
    const elapsed = t.elapsedSinceLastSent();
    assert.ok(elapsed >= 0, `expected >= 0, got ${elapsed}`);
    assert.ok(elapsed < 1000, `expected < 1s, got ${elapsed}`);
  });

  it('reset() clears map and lastSentAt', () => {
    const t = new EchoTracker();
    t.track('chat', 'a');
    t.track('chat', 'b');
    assert.equal(t.size(), 2);
    t.reset();
    assert.equal(t.size(), 0);
    assert.equal(t.elapsedSinceLastSent(), -1);
    assert.equal(t.isEcho('chat', 'a'), false);
  });
});
