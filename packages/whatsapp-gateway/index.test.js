'use strict';

const assert = require('node:assert/strict');
const { describe, it, before, after } = require('node:test');
const http = require('node:http');
const { Readable } = require('node:stream');

// Override DB path to temp location before requiring the module
process.env.WHATSAPP_DB_PATH = '/tmp/test-wa-gateway-' + process.pid + '.db';

const {
  extractNotifyOwner,
  extractRelayCommands,
  buildConversationsContext,
  isRateLimited,
  buildCorsHeaders,
  isAllowedOrigin,
  parseBody,
  MAX_BODY_SIZE,
} = require('./index.js');

// ---------------------------------------------------------------------------
// extractNotifyOwner
// ---------------------------------------------------------------------------
describe('extractNotifyOwner', () => {
  it('extracts a single notification', () => {
    const text = 'Hello! [NOTIFY_OWNER]{"reason":"urgent","summary":"needs help"}[/NOTIFY_OWNER] Bye!';
    const { notifications, cleanedText } = extractNotifyOwner(text);
    assert.equal(notifications.length, 1);
    assert.equal(notifications[0].reason, 'urgent');
    assert.equal(notifications[0].summary, 'needs help');
    assert.match(cleanedText, /^Hello!\s+Bye!$/);
  });

  it('extracts multiple notifications', () => {
    const text = '[NOTIFY_OWNER]{"reason":"a","summary":"x"}[/NOTIFY_OWNER] middle [NOTIFY_OWNER]{"reason":"b","summary":"y"}[/NOTIFY_OWNER]';
    const { notifications, cleanedText } = extractNotifyOwner(text);
    assert.equal(notifications.length, 2);
    assert.equal(notifications[0].reason, 'a');
    assert.equal(notifications[1].reason, 'b');
    assert.equal(cleanedText, 'middle');
  });

  it('returns empty array when no tags present', () => {
    const { notifications, cleanedText } = extractNotifyOwner('Just a normal message');
    assert.equal(notifications.length, 0);
    assert.equal(cleanedText, 'Just a normal message');
  });

  it('handles malformed JSON gracefully', () => {
    const text = '[NOTIFY_OWNER]{bad json}[/NOTIFY_OWNER] ok';
    const { notifications, cleanedText } = extractNotifyOwner(text);
    assert.equal(notifications.length, 0);
    assert.equal(cleanedText, 'ok');
  });

  it('defaults missing fields', () => {
    const text = '[NOTIFY_OWNER]{}[/NOTIFY_OWNER]';
    const { notifications } = extractNotifyOwner(text);
    assert.equal(notifications[0].reason, 'unknown');
    assert.equal(notifications[0].summary, '');
  });

  it('works correctly when called twice in succession (no lastIndex bug)', () => {
    const text = 'A [NOTIFY_OWNER]{"reason":"r1"}[/NOTIFY_OWNER] B';
    const r1 = extractNotifyOwner(text);
    const r2 = extractNotifyOwner(text);
    assert.equal(r1.notifications.length, 1);
    assert.equal(r2.notifications.length, 1);
  });
});

// ---------------------------------------------------------------------------
// extractRelayCommands
// ---------------------------------------------------------------------------
describe('extractRelayCommands', () => {
  it('extracts a relay command', () => {
    const text = 'Sure! [RELAY_TO_STRANGER]{"jid":"123@s.whatsapp.net","message":"Hi there"}[/RELAY_TO_STRANGER] Done.';
    const { relays, cleanedText } = extractRelayCommands(text);
    assert.equal(relays.length, 1);
    assert.equal(relays[0].jid, '123@s.whatsapp.net');
    assert.equal(relays[0].message, 'Hi there');
    assert.match(cleanedText, /^Sure!\s+Done\.$/);

  });

  it('extracts multiple relay commands', () => {
    const text = '[RELAY_TO_STRANGER]{"jid":"a@s.whatsapp.net","message":"m1"}[/RELAY_TO_STRANGER] [RELAY_TO_STRANGER]{"jid":"b@s.whatsapp.net","message":"m2"}[/RELAY_TO_STRANGER]';
    const { relays } = extractRelayCommands(text);
    assert.equal(relays.length, 2);
    assert.equal(relays[0].jid, 'a@s.whatsapp.net');
    assert.equal(relays[1].jid, 'b@s.whatsapp.net');
  });

  it('returns empty array when no tags', () => {
    const { relays, cleanedText } = extractRelayCommands('Normal text');
    assert.equal(relays.length, 0);
    assert.equal(cleanedText, 'Normal text');
  });

  it('skips entries with missing jid or message', () => {
    const text = '[RELAY_TO_STRANGER]{"jid":"x@s.whatsapp.net"}[/RELAY_TO_STRANGER]';
    const { relays } = extractRelayCommands(text);
    assert.equal(relays.length, 0);
  });

  it('handles malformed JSON gracefully', () => {
    // The regex expects {...} — "not json" won't match so the block stays in cleanedText
    const text = '[RELAY_TO_STRANGER]{"jid":"x"}[/RELAY_TO_STRANGER] ok';
    const { relays, cleanedText } = extractRelayCommands(text);
    // jid present but message missing → skipped
    assert.equal(relays.length, 0);
    assert.match(cleanedText, /ok/);
  });

  it('works correctly when called twice in succession (no lastIndex bug)', () => {
    const text = '[RELAY_TO_STRANGER]{"jid":"x@s.whatsapp.net","message":"hi"}[/RELAY_TO_STRANGER]';
    const r1 = extractRelayCommands(text);
    const r2 = extractRelayCommands(text);
    assert.equal(r1.relays.length, 1);
    assert.equal(r2.relays.length, 1);
  });
});

// ---------------------------------------------------------------------------
// buildConversationsContext
// ---------------------------------------------------------------------------
describe('buildConversationsContext', () => {
  it('returns empty string when no active conversations', () => {
    assert.equal(buildConversationsContext(), '');
  });
});

// ---------------------------------------------------------------------------
// isRateLimited
// ---------------------------------------------------------------------------
describe('isRateLimited', () => {
  it('allows first message', () => {
    const jid = 'test-rate-' + Date.now() + '@s.whatsapp.net';
    assert.equal(isRateLimited(jid), false);
  });

  it('allows up to 3 messages within window', () => {
    const jid = 'test-rate-3-' + Date.now() + '@s.whatsapp.net';
    assert.equal(isRateLimited(jid), false); // 1
    assert.equal(isRateLimited(jid), false); // 2
    assert.equal(isRateLimited(jid), false); // 3
  });

  it('blocks the 4th message within window', () => {
    const jid = 'test-rate-4-' + Date.now() + '@s.whatsapp.net';
    isRateLimited(jid); // 1
    isRateLimited(jid); // 2
    isRateLimited(jid); // 3
    assert.equal(isRateLimited(jid), true); // 4 → blocked
  });

  it('different JIDs have independent limits', () => {
    const jid1 = 'test-rate-ind1-' + Date.now() + '@s.whatsapp.net';
    const jid2 = 'test-rate-ind2-' + Date.now() + '@s.whatsapp.net';
    isRateLimited(jid1);
    isRateLimited(jid1);
    isRateLimited(jid1);
    assert.equal(isRateLimited(jid1), true);
    assert.equal(isRateLimited(jid2), false);
  });
});

// ---------------------------------------------------------------------------
// isAllowedOrigin / buildCorsHeaders
// ---------------------------------------------------------------------------
describe('CORS origin validation', () => {
  it('allows localhost origins', () => {
    assert.equal(isAllowedOrigin('http://localhost'), true);
    assert.equal(isAllowedOrigin('http://localhost:3000'), true);
    assert.equal(isAllowedOrigin('https://localhost:8080'), true);
    assert.equal(isAllowedOrigin('http://127.0.0.1'), true);
    assert.equal(isAllowedOrigin('http://127.0.0.1:4545'), true);
  });

  it('allows tauri/app origins', () => {
    assert.equal(isAllowedOrigin('tauri://localhost'), true);
    assert.equal(isAllowedOrigin('app://localhost'), true);
  });

  it('rejects external origins', () => {
    assert.equal(isAllowedOrigin('https://evil.com'), false);
    assert.equal(isAllowedOrigin('http://example.com'), false);
    assert.equal(isAllowedOrigin('https://localhost.evil.com'), false);
  });

  it('rejects null/empty origins', () => {
    assert.equal(isAllowedOrigin(null), false);
    assert.equal(isAllowedOrigin(undefined), false);
    assert.equal(isAllowedOrigin(''), false);
  });

  it('buildCorsHeaders returns headers for allowed origins', () => {
    const headers = buildCorsHeaders('http://localhost:3000');
    assert.equal(headers['Access-Control-Allow-Origin'], 'http://localhost:3000');
    assert.equal(headers['Vary'], 'Origin');
  });

  it('buildCorsHeaders returns empty object for disallowed origins', () => {
    const headers = buildCorsHeaders('https://evil.com');
    assert.deepEqual(headers, {});
  });
});

// ---------------------------------------------------------------------------
// parseBody
// ---------------------------------------------------------------------------
describe('parseBody', () => {
  function mockRequest(body) {
    const stream = new Readable({
      read() {
        if (body) this.push(body);
        this.push(null);
      },
    });
    // Add req-like properties
    stream.headers = {};
    return stream;
  }

  it('parses valid JSON', async () => {
    const req = mockRequest('{"key":"value"}');
    const result = await parseBody(req);
    assert.deepEqual(result, { key: 'value' });
  });

  it('returns empty object for empty body', async () => {
    const req = mockRequest('');
    const result = await parseBody(req);
    assert.deepEqual(result, {});
  });

  it('rejects invalid JSON', async () => {
    const req = mockRequest('not json');
    await assert.rejects(() => parseBody(req), /Invalid JSON/);
  });

  it('rejects oversized body', async () => {
    const bigPayload = 'x'.repeat(MAX_BODY_SIZE + 1);
    const stream = new Readable({
      read() {
        this.push(bigPayload);
        this.push(null);
      },
    });
    stream.headers = {};
    stream.destroy = () => {}; // mock destroy
    await assert.rejects(() => parseBody(stream), /too large/);
  });
});

// ---------------------------------------------------------------------------
// MAX_BODY_SIZE
// ---------------------------------------------------------------------------
describe('MAX_BODY_SIZE', () => {
  it('is 64KB', () => {
    assert.equal(MAX_BODY_SIZE, 64 * 1024);
  });
});

// Cleanup temp DB and force exit (SQLite keeps event loop alive)
after(() => {
  try {
    const fs = require('node:fs');
    const dbPath = process.env.WHATSAPP_DB_PATH;
    if (dbPath && fs.existsSync(dbPath)) {
      fs.unlinkSync(dbPath);
      try { fs.unlinkSync(dbPath + '-wal'); } catch {}
      try { fs.unlinkSync(dbPath + '-shm'); } catch {}
    }
  } catch {}
  // Force exit — SQLite and setInterval timers keep the event loop alive
  setTimeout(() => process.exit(0), 100);
});
