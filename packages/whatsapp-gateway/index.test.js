'use strict';

const assert = require('node:assert/strict');
const { describe, it, before, after } = require('node:test');
const http = require('node:http');
const { Readable } = require('node:stream');

// Override DB path to temp location before requiring the module
process.env.WHATSAPP_DB_PATH = '/tmp/test-wa-gateway-' + process.pid + '.db';

const {
  markdownToWhatsApp,
  extractNotifyOwner,
  extractRelayCommands,
  buildConversationsContext,
  isRateLimited,
  buildCorsHeaders,
  isAllowedOrigin,
  parseBody,
  MAX_BODY_SIZE,
  encodeInstanceKey,
  decodeInstanceKey,
  authStorePathForInstance,
  parseSessionScopedRequest,
  createSessionState,
  getOrCreateSession,
  getSessionStatusView,
  dbSaveMessage,
  dbGetMessagesByJid,
  dbGetUnprocessed,
  dbUpdateLastSeen,
  dbGetLastSeen,
} = require('./index.js');

// ---------------------------------------------------------------------------
// markdownToWhatsApp
// ---------------------------------------------------------------------------
describe('markdownToWhatsApp', () => {
  it('converts bold **text** to *text*', () => {
    assert.equal(markdownToWhatsApp('Hello **world**!'), 'Hello *world*!');
  });

  it('does not convert __text__ (ambiguous with Python dunders)', () => {
    assert.equal(markdownToWhatsApp('Hello __world__!'), 'Hello __world__!');
  });

  it('converts italic *text* to _text_', () => {
    assert.equal(markdownToWhatsApp('Hello *world*!'), 'Hello _world_!');
  });

  it('does not corrupt bold into italic (ordering bug)', () => {
    // **bold** should become *bold* (WhatsApp bold), NOT _bold_ (italic)
    assert.equal(markdownToWhatsApp('**bold** and *italic*'), '*bold* and _italic_');
  });

  it('handles mixed bold and italic in same line', () => {
    assert.equal(markdownToWhatsApp('**strong** then *emphasis*'), '*strong* then _emphasis_');
  });

  it('converts strikethrough ~~text~~ to ~text~', () => {
    assert.equal(markdownToWhatsApp('~~deleted~~'), '~deleted~');
  });

  it('converts inline code `text` to ```text```', () => {
    assert.equal(markdownToWhatsApp('Use `npm install`'), 'Use ```npm install```');
  });

  it('does not touch triple backticks (code blocks)', () => {
    const input = '```\ncode block\n```';
    assert.equal(markdownToWhatsApp(input), input);
  });

  it('handles all formats together', () => {
    const input = '**bold** *italic* ~~strike~~ `code`';
    const expected = '*bold* _italic_ ~strike~ ```code```';
    assert.equal(markdownToWhatsApp(input), expected);
  });

  it('returns null/empty input unchanged', () => {
    assert.equal(markdownToWhatsApp(null), null);
    assert.equal(markdownToWhatsApp(''), '');
    assert.equal(markdownToWhatsApp(undefined), undefined);
  });

  it('does not corrupt stars inside bold placeholders (placeholder collision)', () => {
    // **some *nested* text** should keep bold wrapper, not let italic regex match inside
    assert.equal(markdownToWhatsApp('**some *nested* text**'), '*some *nested* text*');
  });

  it('does not convert Python dunder __init__ to bold', () => {
    assert.equal(markdownToWhatsApp('Call __init__ method'), 'Call __init__ method');
  });

  it('does not format inside inline code', () => {
    assert.equal(markdownToWhatsApp('Use `**bold**` in code'), 'Use ```**bold**``` in code');
  });

  it('preserves backslash-escaped stars', () => {
    assert.equal(markdownToWhatsApp('Price is \\*special\\*'), 'Price is *special*');
  });

  it('does not convert bullet list * item to italic', () => {
    assert.equal(markdownToWhatsApp('* first item\n* second item'), '* first item\n* second item');
  });

  it('does not mangle plain text', () => {
    const plain = 'Just a normal message with no formatting.';
    assert.equal(markdownToWhatsApp(plain), plain);
  });
});

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
    assert.equal(buildConversationsContext(createSessionState('whatsapp:test')), '');
  });
});

describe('session partitioning helpers', () => {
  it('round-trips encoded instance keys for auth-store directories', () => {
    const instanceKey = 'whatsapp:acct-a/primary';
    assert.equal(decodeInstanceKey(encodeInstanceKey(instanceKey)), instanceKey);
  });

  it('creates isolated session state per instance', () => {
    const first = createSessionState('whatsapp:acct-a');
    const second = createSessionState('whatsapp:acct-b');

    first.activeConversations.set('a@s.whatsapp.net', { messageCount: 1 });
    first.rateLimitMap.set('a@s.whatsapp.net', { timestamps: [Date.now()] });

    assert.equal(first.instanceKey, 'whatsapp:acct-a');
    assert.equal(second.instanceKey, 'whatsapp:acct-b');
    assert.equal(second.activeConversations.size, 0);
    assert.equal(second.rateLimitMap.size, 0);
    assert.notEqual(first.activeConversations, second.activeConversations);
    assert.notEqual(first.rateLimitMap, second.rateLimitMap);
  });

  it('reuses the same session record for the same instance key', () => {
    const sessions = new Map();

    const first = getOrCreateSession(sessions, 'whatsapp:acct-a');
    const second = getOrCreateSession(sessions, 'whatsapp:acct-a');
    const third = getOrCreateSession(sessions, 'whatsapp:acct-b');

    assert.equal(first, second);
    assert.notEqual(first, third);
    assert.equal(sessions.size, 2);
  });

  it('builds status from the owning session record', () => {
    const session = createSessionState('whatsapp:acct-a');
    session.sessionId = 'sess-123';
    session.connStatus = 'qr_ready';
    session.statusMessage = 'Scan QR';
    session.qrExpired = false;
    session.qrDataUrl = 'data:image/png;base64,abc';

    assert.deepEqual(getSessionStatusView(session), {
      instance_key: 'whatsapp:acct-a',
      connected: false,
      message: 'Scan QR',
      expired: false,
      session_id: 'sess-123',
      qr_data_url: 'data:image/png;base64,abc',
    });
  });

  it('persists and scopes sqlite messages by instance key', () => {
    const instanceA = 'whatsapp:acct-a';
    const instanceB = 'whatsapp:acct-b';
    const jid = `store-${Date.now()}@s.whatsapp.net`;
    const idA = `msg-a-${Date.now()}`;
    const idB = `msg-b-${Date.now()}`;

    dbSaveMessage({
      instanceKey: instanceA,
      id: idA,
      jid,
      senderJid: 'sender-a',
      pushName: 'A',
      phone: '+100',
      text: 'one',
      direction: 'inbound',
      timestamp: Date.now() - 2000,
      processed: 0,
      rawType: 'text',
    });
    dbSaveMessage({
      instanceKey: instanceB,
      id: idB,
      jid,
      senderJid: 'sender-b',
      pushName: 'B',
      phone: '+200',
      text: 'two',
      direction: 'inbound',
      timestamp: Date.now() - 1000,
      processed: 0,
      rawType: 'text',
    });

    const rowsA = dbGetMessagesByJid(instanceA, jid, 20, 0);
    const rowsB = dbGetMessagesByJid(instanceB, jid, 20, 0);
    assert.equal(rowsA.length, 1);
    assert.equal(rowsB.length, 1);
    assert.equal(rowsA[0].instance_key, instanceA);
    assert.equal(rowsB[0].instance_key, instanceB);

    const unprocessedA = dbGetUnprocessed(instanceA, Date.now() + 1);
    const unprocessedB = dbGetUnprocessed(instanceB, Date.now() + 1);
    assert.equal(unprocessedA.some((row) => row.id === idA), true);
    assert.equal(unprocessedA.some((row) => row.id === idB), false);
    assert.equal(unprocessedB.some((row) => row.id === idB), true);
    assert.equal(unprocessedB.some((row) => row.id === idA), false);
  });

  it('stores last-seen rows per instance key', () => {
    const instanceA = 'whatsapp:lastseen-a';
    const instanceB = 'whatsapp:lastseen-b';
    const jid = `seen-${Date.now()}@s.whatsapp.net`;
    const tsA = Date.now() - 5000;
    const tsB = Date.now() - 1000;

    dbUpdateLastSeen(instanceA, jid, tsA);
    dbUpdateLastSeen(instanceB, jid, tsB);

    const rowsA = dbGetLastSeen(instanceA);
    const rowsB = dbGetLastSeen(instanceB);
    assert.equal(rowsA.some((row) => row.jid === jid && row.last_timestamp === tsA), true);
    assert.equal(rowsB.some((row) => row.jid === jid && row.last_timestamp === tsB), true);
  });
});

// ---------------------------------------------------------------------------
// isRateLimited
// ---------------------------------------------------------------------------
describe('isRateLimited', () => {
  it('allows first message', () => {
    const jid = 'test-rate-' + Date.now() + '@s.whatsapp.net';
    const session = createSessionState('whatsapp:rate-1');
    assert.equal(isRateLimited(session, jid), false);
  });

  it('allows up to 3 messages within window', () => {
    const jid = 'test-rate-3-' + Date.now() + '@s.whatsapp.net';
    const session = createSessionState('whatsapp:rate-3');
    assert.equal(isRateLimited(session, jid), false); // 1
    assert.equal(isRateLimited(session, jid), false); // 2
    assert.equal(isRateLimited(session, jid), false); // 3
  });

  it('blocks the 4th message within window', () => {
    const jid = 'test-rate-4-' + Date.now() + '@s.whatsapp.net';
    const session = createSessionState('whatsapp:rate-4');
    isRateLimited(session, jid); // 1
    isRateLimited(session, jid); // 2
    isRateLimited(session, jid); // 3
    assert.equal(isRateLimited(session, jid), true); // 4 → blocked
  });

  it('different JIDs have independent limits', () => {
    const jid1 = 'test-rate-ind1-' + Date.now() + '@s.whatsapp.net';
    const jid2 = 'test-rate-ind2-' + Date.now() + '@s.whatsapp.net';
    const session = createSessionState('whatsapp:rate-ind');
    isRateLimited(session, jid1);
    isRateLimited(session, jid1);
    isRateLimited(session, jid1);
    assert.equal(isRateLimited(session, jid1), true);
    assert.equal(isRateLimited(session, jid2), false);
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

// ---------------------------------------------------------------------------
// Session partitioning helpers
// ---------------------------------------------------------------------------
describe('session partitioning helpers', () => {
  it('encodes instance keys into filesystem-safe auth store names', () => {
    assert.equal(
      encodeInstanceKey('whatsapp:tenant-a/us-east-1'),
      'whatsapp_3Atenant_2Da_2Fus_2Deast_2D1'
    );
  });

  it('derives distinct auth store paths per instance key', () => {
    const a = authStorePathForInstance('/tmp/wa-gateway', 'whatsapp:tenant-a');
    const b = authStorePathForInstance('/tmp/wa-gateway', 'whatsapp:tenant-b');
    assert.match(a, /auth_store[\\/]/);
    assert.notEqual(a, b);
    assert.match(a, /tenant/);
  });

  it('requires instance_key on session-scoped requests', () => {
    assert.throws(
      () => parseSessionScopedRequest({}),
      /Missing "instance_key"/
    );
  });

  it('returns the requested instance_key for session-scoped requests', () => {
    assert.deepEqual(
      parseSessionScopedRequest({ instance_key: 'whatsapp:tenant-a' }),
      { instanceKey: 'whatsapp:tenant-a' }
    );
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
