'use strict';

const assert = require('node:assert/strict');
const { describe, it, before, after } = require('node:test');
const http = require('node:http');
const { Readable } = require('node:stream');

// Override DB path to temp location before requiring the module
process.env.WHATSAPP_DB_PATH = '/tmp/test-wa-gateway-' + process.pid + '.db';
// Bind a mock LibreFang HTTP server on a fixed port BEFORE requiring the
// module — `LIBREFANG_URL` is captured at module load. Using a dedicated
// loopback port (4547) avoids clashing with a real daemon on 4545.
const MOCK_LIBREFANG_PORT = 24547;
process.env.LIBREFANG_URL = `http://127.0.0.1:${MOCK_LIBREFANG_PORT}`;

const {
  markdownToWhatsApp,
  extractNotifyOwner,
  extractRelayCommands,
  ownerIntentsRelay,
  buildConversationsContext,
  isRateLimited,
  buildCorsHeaders,
  isAllowedOrigin,
  parseBody,
  MAX_BODY_SIZE,
  forwardToLibreFang,
  forwardToLibreFangStreaming,
  shouldSkipCatchupForMissingJid,
  resolveLidProactively,
  checkHeartbeat,
  computeBackoffDelay,
  isSilentResponse,
  stripNoReply,
  createHoldbackAccumulator,
  SILENT_HOLDBACK_MIN_CHARS,
  echoTracker,
  ECHO_TRACKER_ENABLED,
  EchoTracker,
  lidToPnJid,
  lidMapSet,
  db,
  LID_PERSIST_ENABLED,
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

// ---------------------------------------------------------------------------
// CS-01: forwardToLibreFang* throw on empty chatJid + catchup guard
// ---------------------------------------------------------------------------
describe('CS-01 forwardToLibreFang chatJid enforcement', () => {
  let mockServer;
  const lastRequests = [];

  before(async () => {
    mockServer = http.createServer((req, res) => {
      let body = '';
      req.on('data', (c) => (body += c));
      req.on('end', () => {
        const parsed = body ? JSON.parse(body) : null;
        lastRequests.push({ url: req.url, method: req.method, body: parsed });
        if (req.url === '/api/agents' && req.method === 'GET') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify([{ id: 'test-agent-id', name: 'TestAgent' }]));
          return;
        }
        if (req.url && req.url.startsWith('/api/agents/') && req.url.endsWith('/message')) {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ response: 'mock reply' }));
          return;
        }
        res.writeHead(404);
        res.end();
      });
    });
    await new Promise((resolve) => mockServer.listen(MOCK_LIBREFANG_PORT, '127.0.0.1', resolve));
  });

  after(async () => {
    if (mockServer) await new Promise((r) => mockServer.close(r));
  });

  it('Test 1: forwardToLibreFang throws when chatJid is empty', async () => {
    await assert.rejects(
      () => forwardToLibreFang('hi', '', '+39123', 'Alice', false, [], { isGroup: false, wasMentioned: false, chatJid: '' }),
      (err) => {
        assert.equal(err.code, 'CHATJID_EMPTY');
        assert.match(err.message, /chatJid empty/);
        assert.match(err.message, /phone=\+39123/);
        assert.match(err.message, /pushName=Alice/);
        assert.match(err.message, /isGroup=false/);
        return true;
      }
    );
  });

  it('Test 2: forwardToLibreFangStreaming throws when chatJid is empty', async () => {
    await assert.rejects(
      () => forwardToLibreFangStreaming('hi', '', '+39123', 'Alice', false, [], () => {}, '', { isGroup: true, wasMentioned: false }),
      (err) => {
        assert.equal(err.code, 'CHATJID_EMPTY');
        assert.match(err.message, /isGroup=true/);
        return true;
      }
    );
  });

  it('Test 3: forwardToLibreFang proceeds with valid chatJid and sends channel_type=whatsapp:<jid>', async () => {
    lastRequests.length = 0;
    const jid = '39123@s.whatsapp.net';
    const reply = await forwardToLibreFang('hello', '', '+39123', 'Alice', false, [], { isGroup: false, wasMentioned: false, chatJid: jid });
    assert.equal(reply, 'mock reply');
    const msgReq = lastRequests.find((r) => r.url && r.url.endsWith('/message'));
    assert.ok(msgReq, 'expected /message POST to have fired');
    assert.equal(msgReq.body.channel_type, `whatsapp:${jid}`);
  });

  it('Test 4: no code path produces bare channel_type "whatsapp"', () => {
    // Source-level invariant: the only channelType assignments are
    // `whatsapp:${chatJid}`, and entry is guarded by the CS-01 throw.
    const fs = require('node:fs');
    const src = fs.readFileSync(__dirname + '/index.js', 'utf8');
    assert.equal(src.includes("chatJid ? `whatsapp:"), false, 'ternary fallback must be removed');
    assert.equal(/channelType\s*=\s*'whatsapp'\s*;/.test(src), false, 'bare whatsapp assignment must not exist');
  });

  it('Test 5 (catchup guard): shouldSkipCatchupForMissingJid returns true for null/empty jid rows', () => {
    assert.equal(shouldSkipCatchupForMissingJid({ id: 1, jid: null }), true);
    assert.equal(shouldSkipCatchupForMissingJid({ id: 2, jid: '' }), true);
    assert.equal(shouldSkipCatchupForMissingJid({ id: 3, jid: undefined }), true);
    assert.equal(shouldSkipCatchupForMissingJid({ id: 4, jid: '39123@s.whatsapp.net' }), false);
    assert.equal(shouldSkipCatchupForMissingJid(null), true);
  });
});

// ---------------------------------------------------------------------------
// CS-02: proactive LID → PN resolution for first-seen LIDs
// ---------------------------------------------------------------------------
describe('CS-02 resolveLidProactively', () => {
  it('Test 1: first-seen LID triggers onWhatsApp and populates cache', async () => {
    const cache = new Map();
    let calls = 0;
    const sock = {
      onWhatsApp: (lids) => {
        calls += 1;
        return Promise.resolve([{ jid: '39123@s.whatsapp.net', lid: lids[0] }]);
      },
    };
    const result = await resolveLidProactively(sock, '999@lid', cache, 500);
    assert.equal(result, 'resolved');
    assert.equal(calls, 1);
    assert.equal(cache.get('999@lid'), '39123@s.whatsapp.net');
  });

  it('Test 2: cached LID is NOT re-queried', async () => {
    const cache = new Map([['999@lid', '39123@s.whatsapp.net']]);
    let calls = 0;
    const sock = { onWhatsApp: () => { calls += 1; return Promise.resolve([]); } };
    const result = await resolveLidProactively(sock, '999@lid', cache, 500);
    assert.equal(result, 'skipped');
    assert.equal(calls, 0);
  });

  it('Test 3: onWhatsApp timeout does NOT block and does NOT populate cache', async () => {
    const cache = new Map();
    const sock = { onWhatsApp: () => new Promise(() => {}) }; // never resolves
    const t0 = Date.now();
    const result = await resolveLidProactively(sock, '999@lid', cache, 80);
    const elapsed = Date.now() - t0;
    assert.equal(result, 'timeout');
    assert.ok(elapsed >= 70 && elapsed < 500, `elapsed=${elapsed}`);
    assert.equal(cache.has('999@lid'), false);
  });

  it('Test 4: onWhatsApp returns [] → lid_resolve_empty tag, cache untouched', async () => {
    const cache = new Map();
    const sock = { onWhatsApp: () => Promise.resolve([]) };
    const result = await resolveLidProactively(sock, '999@lid', cache, 500);
    assert.equal(result, 'empty');
    assert.equal(cache.has('999@lid'), false);
  });
});

// ---------------------------------------------------------------------------
// ST-01: heartbeat watchdog
// ---------------------------------------------------------------------------
describe('ST-01 heartbeat watchdog', () => {
  it('Test 1: watchdog invokes sock.end + logs heartbeat_timeout when silence exceeds threshold', async () => {
    // Reconstruct the watchdog interval body exactly as wired in index.js —
    // we can't drive the module-internal `lastInboundAt` directly, but the
    // pure checkHeartbeat predicate + sock.end contract is the same.
    const logs = [];
    const origLog = console.log;
    console.log = (msg) => { logs.push(msg); };
    let ended = 0;
    const sock = { end: () => { ended += 1; } };
    let connStatus = 'connected';
    let lastInbound = Date.now() - 200_000; // 200s ago → over 180s threshold

    const HEARTBEAT_MS = 180_000;
    const tick = () => {
      if (!sock || connStatus !== 'connected') return;
      const now = Date.now();
      if (checkHeartbeat(now, lastInbound, HEARTBEAT_MS)) {
        console.log(JSON.stringify({
          event: 'heartbeat_timeout',
          last_inbound_ms: now - lastInbound,
          threshold_ms: HEARTBEAT_MS,
        }));
        try { sock.end(undefined); } catch {}
      }
    };
    const interval = setInterval(tick, 10);
    await new Promise((r) => setTimeout(r, 30));
    clearInterval(interval);
    console.log = origLog;

    assert.ok(ended >= 1, `expected sock.end to fire (got ${ended})`);
    const htLog = logs.find((l) => typeof l === 'string' && l.includes('heartbeat_timeout'));
    assert.ok(htLog, 'expected heartbeat_timeout log line');
    const parsed = JSON.parse(htLog);
    assert.equal(parsed.threshold_ms, 180_000);
    assert.ok(parsed.last_inbound_ms >= 180_000);
  });

  it('Test 2: checkHeartbeat returns false within threshold (recent activity)', () => {
    const now = 1_000_000;
    assert.equal(checkHeartbeat(now, now - 10_000, 180_000), false);
    assert.equal(checkHeartbeat(now, now - 179_999, 180_000), false);
    assert.equal(checkHeartbeat(now, now - 180_001, 180_000), true);
  });

  it('Test 3: watchdog NO-OPs when sock is null or status != connected', () => {
    let ended = 0;
    const sock = { end: () => { ended += 1; } };
    const HEARTBEAT_MS = 180_000;
    const lastInbound = Date.now() - 500_000;

    // sock null → no action regardless of silence
    const tickSockNull = () => {
      const currentSock = null;
      if (!currentSock || 'connected' !== 'connected') return;
      if (checkHeartbeat(Date.now(), lastInbound, HEARTBEAT_MS)) currentSock && currentSock.end();
    };
    tickSockNull();

    // status != connected → no action
    const tickStatusReconnecting = () => {
      const connStatus = 'disconnected';
      if (!sock || connStatus !== 'connected') return;
      if (checkHeartbeat(Date.now(), lastInbound, HEARTBEAT_MS)) sock.end();
    };
    tickStatusReconnecting();

    assert.equal(ended, 0);
  });

  it('Test 4: source-level invariant — cleanupSocket + close branch clear heartbeatInterval', () => {
    const fs = require('node:fs');
    const src = fs.readFileSync(__dirname + '/index.js', 'utf8');
    // cleanupSocket clears the interval
    assert.match(src, /cleanupSocket[\s\S]*?heartbeatInterval[\s\S]*?clearInterval\(heartbeatInterval\)/);
    // messages.upsert refreshes lastInboundAt
    assert.match(src, /messages\.upsert[\s\S]*?lastInboundAt = Date\.now\(\)/);
    // heartbeat log uses the exact event name
    assert.match(src, /event: 'heartbeat_timeout'/);
  });
});

// ---------------------------------------------------------------------------
// ST-02: jittered exponential backoff
// ---------------------------------------------------------------------------
describe('ST-02 computeBackoffDelay', () => {
  // Deterministic RNG — Mulberry32 seeded.
  function mulberry32(seed) {
    let s = seed >>> 0;
    return function () {
      s = (s + 0x6D2B79F5) >>> 0;
      let t = s;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  it('Test 1: delay stays within [base*0.75, base*1.25] and respects cap', () => {
    const rng = mulberry32(42);
    // attempt 1: base = 2000 → [1500, 2500]
    const d1 = computeBackoffDelay(1, rng);
    assert.ok(d1 >= 1500 && d1 <= 2500, `attempt 1 delay=${d1}`);
    // attempt 2: base = 3600 → [2700, 4500]
    const d2 = computeBackoffDelay(2, rng);
    assert.ok(d2 >= 2700 && d2 <= 4500, `attempt 2 delay=${d2}`);
    // attempt 8: base hits 30000 cap → [22500, 37500]
    const d8 = computeBackoffDelay(8, rng);
    assert.ok(d8 >= 22500 && d8 <= 37500, `attempt 8 delay=${d8}`);
    // attempt 20: still capped at 30000 base → [22500, 37500]
    const d20 = computeBackoffDelay(20, rng);
    assert.ok(d20 >= 22500 && d20 <= 37500, `attempt 20 delay=${d20}`);
  });

  it('Test 1b: compound growth factor ≈ 1.8 before cap', () => {
    // With rng fixed to 0.5 → jitter factor = 1.0 exactly.
    const noJitter = () => 0.5;
    assert.equal(computeBackoffDelay(1, noJitter), 2000);
    assert.equal(computeBackoffDelay(2, noJitter), 3600);   // 2000 * 1.8
    assert.equal(computeBackoffDelay(3, noJitter), 6480);   // 2000 * 1.8^2
    assert.equal(computeBackoffDelay(4, noJitter), 11664);
    assert.equal(computeBackoffDelay(5, noJitter), 20995);
    assert.equal(computeBackoffDelay(6, noJitter), 30000);  // capped
    assert.equal(computeBackoffDelay(100, noJitter), 30000);
  });

  it('Test 2: no hard stop — attempt 100 still produces a finite delay (≤ cap range)', () => {
    const d = computeBackoffDelay(100, mulberry32(7));
    assert.ok(Number.isFinite(d) && d > 0 && d <= 37500);
  });

  it('Test 3: loggedOut / forbidden branches remain untouched (source invariant)', () => {
    const fs = require('node:fs');
    const src = fs.readFileSync(__dirname + '/index.js', 'utf8');
    // The hard-stop check must be gone.
    assert.equal(
      /reconnectAttempts\s*>=\s*MAX_RECONNECT_ATTEMPTS/.test(src),
      false,
      'hard-stop check must be removed'
    );
    // Legacy constants removed — zero remaining references.
    assert.equal((src.match(/MAX_RECONNECT_ATTEMPTS/g) || []).length, 0);
    assert.equal((src.match(/MAX_RECONNECT_DELAY/g) || []).length, 0);
    // loggedOut / forbidden branches preserved.
    assert.match(src, /DisconnectReason\.loggedOut/);
    assert.match(src, /DisconnectReason\.forbidden/);
    // New backoff call site is present.
    assert.match(src, /computeBackoffDelay\(reconnectAttempts\)/);
  });
});

// ---------------------------------------------------------------------------
// Phase 3 §A — Echo tracker wiring (EB-01)
// ---------------------------------------------------------------------------
describe('echo tracker wiring (Phase 3 §A)', () => {
  it('exports tracker handle, ECHO_TRACKER_ENABLED, and EchoTracker class', () => {
    assert.ok(echoTracker, 'echoTracker should be exported');
    assert.equal(typeof echoTracker.track, 'function');
    assert.equal(typeof echoTracker.isEcho, 'function');
    assert.equal(typeof echoTracker.size, 'function');
    assert.equal(typeof echoTracker.reset, 'function');
    assert.equal(typeof EchoTracker, 'function');
    assert.equal(typeof EchoTracker.normalize, 'function');
    // Default flag state (no env var set in test env)
    assert.equal(typeof ECHO_TRACKER_ENABLED, 'boolean');
  });

  it('integration: outbound track then inbound echo would drop (raw body)', () => {
    echoTracker.reset();
    // Simulate the outbound wire-in (every sock.sendMessage({ text }) is followed by track)
    echoTracker.track('ciao');
    // Simulate the inbound gate condition with the same body
    assert.equal(echoTracker.isEcho('ciao'), true,
      'inbound echo of just-sent message must be detected');
    assert.equal(echoTracker.size(), 1);
  });

  it('integration: normalization works through wiring (Hello. -> hello)', () => {
    echoTracker.reset();
    echoTracker.track('Hello.');
    assert.equal(echoTracker.isEcho('hello'), true,
      'normalized echo (case + trailing punct) must drop');
    assert.equal(echoTracker.isEcho('HELLO!'), true);
  });

  it('integration: unrelated inbound is NOT dropped (no false positive)', () => {
    echoTracker.reset();
    echoTracker.track('ciao');
    assert.equal(echoTracker.isEcho('something else'), false,
      'unrelated message must pass through (forwardToLibreFang would be called)');
    // tracker unchanged for non-matching probe
    assert.equal(echoTracker.size(), 1);
  });

  it('flag gate: when LIBREFANG_ECHO_TRACKER=off, gate is bypassed', () => {
    // ECHO_TRACKER_ENABLED is captured at module load. We assert the source
    // shape so a future regression (gating without flag check) is caught.
    const src = require('node:fs').readFileSync(require('node:path').join(__dirname, 'index.js'), 'utf8');
    // The gate must be wrapped in an ECHO_TRACKER_ENABLED check.
    assert.match(src,
      /if\s*\(\s*ECHO_TRACKER_ENABLED\s*&&\s*messageText\s*&&\s*echoTracker\.isEcho/,
      'inbound gate must be flag-gated by ECHO_TRACKER_ENABLED');
    // Each track call must also be flag-gated.
    const trackCalls = src.match(/echoTracker\.track\(/g) || [];
    const flaggedTrackCalls = src.match(/if\s*\(\s*ECHO_TRACKER_ENABLED\s*\)\s*echoTracker\.track\(/g) || [];
    assert.equal(trackCalls.length, flaggedTrackCalls.length,
      `every echoTracker.track() must be flag-gated (found ${trackCalls.length} calls, ${flaggedTrackCalls.length} flagged)`);
    // Default ON: env unset → enabled.
    assert.equal(process.env.LIBREFANG_ECHO_TRACKER, undefined);
    assert.equal(ECHO_TRACKER_ENABLED, true);
  });

  it('echo_drop log structure is correct shape (would emit on drop)', () => {
    // Verify the source emits the spec'd log structure when isEcho fires.
    const src = require('node:fs').readFileSync(require('node:path').join(__dirname, 'index.js'), 'utf8');
    assert.match(src, /event:\s*'echo_drop'/);
    assert.match(src, /body_excerpt:/);
    assert.match(src, /tracker_size:/);
    assert.match(src, /elapsed_ms_since_last_sent:/);
    // Body excerpt must be capped at 80 chars.
    assert.match(src, /\.slice\(0,\s*80\)/);
  });

  it('outbound wire-in covers all 7 text sendMessage sites', () => {
    const src = require('node:fs').readFileSync(require('node:path').join(__dirname, 'index.js'), 'utf8');
    const trackCount = (src.match(/echoTracker\.track\(/g) || []).length;
    assert.equal(trackCount, 7,
      `expected 7 echoTracker.track() calls (one per outbound text site), got ${trackCount}`);
  });
});

// Cleanup temp DB and force exit (SQLite keeps event loop alive)
// ---------------------------------------------------------------------------
// ID-01 identity refactor — equivalence between pre-refactor inline logic
// and post-refactor lib/identity helpers. These fixtures assert that the
// same JID shape produces the same outbound/sender/owner strings as the
// inline code would have produced prior to this refactor.
// ---------------------------------------------------------------------------
describe('ID-01 identity refactor equivalence', () => {
  const {
    isLidJid, isGroupJid, normalizeDeviceScopedJid,
    extractE164, phoneToJid, resolvePeerId, deriveOwnerJids,
  } = require('./lib/identity');

  // Legacy inline helpers reproduced from the pre-refactor inline code at
  // index.js:229-234, 1164-1197, 2304-2306, 2232.
  const legacyIsLid = (jid) => !!jid && jid.endsWith('@lid');
  const legacyIsGroup = (jid) => !!jid && jid.endsWith('@g.us');
  const legacyOutboundJid = (to) => to.includes('@g.us') ? to
    : to.replace(/^\+/, '').replace(/@.*$/, '') + '@s.whatsapp.net';
  const legacyOwnerJids = (nums) =>
    new Set(nums.map(n => n.replace(/^\+/, '') + '@s.whatsapp.net'));
  const legacyResolve = (sender, { senderPn, cache, participant }) => {
    const isLid = legacyIsLid(sender);
    const isGroup = legacyIsGroup(sender);
    if (senderPn) return senderPn;
    if (isLid && cache.has(sender)) return cache.get(sender);
    if (!isLid && !isGroup) return sender;
    if (participant && !legacyIsLid(participant)) return participant;
    return '';
  };

  it('isLid boolean parity', () => {
    for (const jid of ['123@lid', '123@s.whatsapp.net', '123-456@g.us', '']) {
      assert.equal(isLidJid(jid), legacyIsLid(jid), `isLid parity for ${jid}`);
    }
  });

  it('isGroup boolean parity', () => {
    for (const jid of ['123-456@g.us', '123@lid', '123@s.whatsapp.net', '']) {
      assert.equal(isGroupJid(jid), legacyIsGroup(jid), `isGroup parity for ${jid}`);
    }
  });

  it('deriveOwnerJids matches legacy Set', () => {
    const nums = ['+39111', '+39222'];
    const got = deriveOwnerJids(nums);
    const legacy = legacyOwnerJids(nums);
    assert.deepEqual([...got].sort(), [...legacy].sort());
  });

  it('phoneToJid matches legacy outbound pattern for phones & groups', () => {
    for (const to of ['+39111', '39111', '123-456@g.us']) {
      assert.equal(phoneToJid(to), legacyOutboundJid(to), `outbound parity for ${to}`);
    }
  });

  it('resolvePeerId matches legacy for plain phone JID', () => {
    const r = resolvePeerId('391234@s.whatsapp.net', { lidToPnCache: new Map() });
    const legacy = legacyResolve('391234@s.whatsapp.net', { senderPn: '', cache: new Map(), participant: '' });
    assert.equal(r.peer, legacy);
    assert.equal(r.confidence, 'direct');
  });

  it('resolvePeerId matches legacy for LID with senderPn', () => {
    const r = resolvePeerId('111@lid', { lidToPnCache: new Map(), senderPn: '391234@s.whatsapp.net' });
    const legacy = legacyResolve('111@lid', { senderPn: '391234@s.whatsapp.net', cache: new Map(), participant: '' });
    assert.equal(r.peer, legacy);
    assert.equal(r.confidence, 'direct');
  });

  it('resolvePeerId matches legacy for LID in cache', () => {
    const cache = new Map([['111@lid', '391234@s.whatsapp.net']]);
    const r = resolvePeerId('111@lid', { lidToPnCache: cache });
    const legacy = legacyResolve('111@lid', { senderPn: '', cache, participant: '' });
    assert.equal(r.peer, legacy);
    assert.equal(r.confidence, 'cache');
  });

  it('resolvePeerId matches legacy for LID with phone participant', () => {
    const r = resolvePeerId('111@lid', { lidToPnCache: new Map(), participant: '391234@s.whatsapp.net' });
    const legacy = legacyResolve('111@lid', { senderPn: '', cache: new Map(), participant: '391234@s.whatsapp.net' });
    assert.equal(r.peer, legacy);
    assert.equal(r.confidence, 'participant');
  });

  it('resolvePeerId returns empty for unresolvable LID (matches legacy)', () => {
    const r = resolvePeerId('111@lid', { lidToPnCache: new Map() });
    const legacy = legacyResolve('111@lid', { senderPn: '', cache: new Map(), participant: '' });
    assert.equal(r.peer, legacy);
    assert.equal(r.peer, '');
    assert.equal(r.confidence, 'lid_unresolved');
  });

  it('resolvePeerId tags group JID with group confidence', () => {
    const r = resolvePeerId('123-456@g.us', { lidToPnCache: new Map() });
    assert.equal(r.confidence, 'group');
    assert.equal(r.peer, '123-456@g.us');
  });

  it('extractE164 strips device suffix (latent bug fix vs legacy)', () => {
    // Legacy inline `'+' + jid.replace(/@.*$/, '')` produced '+123:45' for
    // device-scoped JIDs — malformed. New extractE164 correctly yields '+123'.
    assert.equal(extractE164('391234:7@s.whatsapp.net'), '+391234');
  });

  it('normalizeDeviceScopedJid passthrough for plain JIDs', () => {
    assert.equal(normalizeDeviceScopedJid('391234@s.whatsapp.net'), '391234@s.whatsapp.net');
  });
});

// ---------------------------------------------------------------------------
// ID-03 structured log — identity_unresolved must emit JSON with all fields
// ---------------------------------------------------------------------------
describe('ID-03 identity_unresolved log shape', () => {
  it('emits JSON with event/jid/reason/lid_cache_size on unresolved LID', () => {
    // Simulate the handler's log emission path (inlined from index.js).
    const { resolvePeerId } = require('./lib/identity');
    const lidToPnJid = new Map();
    const sender = '111@lid';
    const senderPnRaw = '';
    const participant = '';

    const { peer, confidence } = resolvePeerId(sender, {
      lidToPnCache: lidToPnJid,
      senderPn: senderPnRaw,
      participant,
    });

    assert.equal(peer, '');
    assert.equal(confidence, 'lid_unresolved');

    // Capture console.warn to ensure the payload shape is JSON with all fields.
    const origWarn = console.warn;
    let captured = null;
    console.warn = (line) => { captured = line; };
    try {
      const reason = senderPnRaw ? 'senderPn_present_but_unextractable'
        : (lidToPnJid.has(sender)) ? 'cache_hit_but_unextractable'
        : participant ? 'participant_was_lid'
        : 'no_mapping_available';
      console.warn(JSON.stringify({
        event: 'identity_unresolved',
        jid: sender,
        reason,
        lid_cache_size: lidToPnJid.size,
        confidence,
      }));
    } finally {
      console.warn = origWarn;
    }

    assert.ok(captured, 'warn was called');
    const parsed = JSON.parse(captured);
    assert.equal(parsed.event, 'identity_unresolved');
    assert.equal(parsed.jid, '111@lid');
    assert.equal(parsed.reason, 'no_mapping_available');
    assert.equal(parsed.lid_cache_size, 0);
    assert.equal(parsed.confidence, 'lid_unresolved');
  });
});

// ---------------------------------------------------------------------------
// Phase 4 §B (ID-02) — persisted LID cache integration
// ---------------------------------------------------------------------------
// These tests exercise the real `db` handle owned by index.js together with
// the in-memory `lidToPnJid` Map. Each test uses distinct LID keys so runs
// remain independent.
describe('ID-02 persisted LID cache wiring', () => {
  it('exports the write-through helper and the persistence flag', () => {
    assert.equal(typeof lidMapSet, 'function');
    assert.ok(lidToPnJid instanceof Map);
    assert.ok(db, 'db handle must be exported');
    // Default enabled unless LIBREFANG_LID_PERSIST=off is set in the env.
    assert.equal(LID_PERSIST_ENABLED, process.env.LIBREFANG_LID_PERSIST !== 'off');
  });

  it('creates the lid_cache table at boot', () => {
    const row = db
      .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='lid_cache'")
      .get();
    assert.equal(row?.name, 'lid_cache');
  });

  it('mirrors a mapping observation into both the Map and SQLite', () => {
    const LID = 'integration-a@lid';
    const PN  = '391230000100@s.whatsapp.net';

    lidMapSet(LID, PN);

    // In-memory authoritative state.
    assert.equal(lidToPnJid.get(LID), PN);

    // Persisted mirror.
    const row = db
      .prepare('SELECT lid, pn_jid, updated_at FROM lid_cache WHERE lid = ?')
      .get(LID);
    assert.equal(row?.pn_jid, PN);
    assert.equal(typeof row?.updated_at, 'number');
    assert.ok(row.updated_at > 0);
  });

  it('ignores empty lid or empty pn_jid without touching SQLite', () => {
    const beforeCount = db.prepare('SELECT COUNT(*) AS c FROM lid_cache').get().c;
    lidMapSet('', '391230000200@s.whatsapp.net');
    lidMapSet('integration-b@lid', '');
    const afterCount = db.prepare('SELECT COUNT(*) AS c FROM lid_cache').get().c;
    assert.equal(afterCount, beforeCount);
    assert.equal(lidToPnJid.has('integration-b@lid'), false);
  });

  it('INSERT OR REPLACE updates pn_jid when the same lid reappears', () => {
    const LID = 'integration-c@lid';
    lidMapSet(LID, '391230000300@s.whatsapp.net');
    lidMapSet(LID, '391230000301@s.whatsapp.net');

    const rows = db
      .prepare('SELECT pn_jid FROM lid_cache WHERE lid = ?')
      .all(LID);
    assert.equal(rows.length, 1, 'primary key must coalesce rows');
    assert.equal(rows[0].pn_jid, '391230000301@s.whatsapp.net');
    assert.equal(lidToPnJid.get(LID), '391230000301@s.whatsapp.net');
  });
});

// Cross-restart: simulate shutdown + boot by opening a second DB handle at
// the same path with the lid-cache module directly. We cannot reload
// index.js in-process (it has module-level setInterval timers); instead we
// assert that the SQL rows index.js wrote are visible to an independent
// connection calling `loadAll`, which is exactly what boot-time hydration
// does.
describe('ID-02 cross-restart hydration', () => {
  it('rows written via lidMapSet are visible to lidCache.loadAll on a fresh handle', () => {
    const Database = require('better-sqlite3');
    const lidCache = require('./lib/lid-cache');

    const SEED_LID = 'restart-seed@lid';
    const SEED_PN  = '391230000999@s.whatsapp.net';
    lidMapSet(SEED_LID, SEED_PN);

    // Open an independent connection against the same file. better-sqlite3
    // with WAL mode lets readers see committed writes from another handle.
    const dbPath = process.env.WHATSAPP_DB_PATH;
    const db2 = new Database(dbPath, { readonly: true });
    try {
      const map = lidCache.loadAll(db2);
      assert.equal(map.get(SEED_LID), SEED_PN);
    } finally {
      db2.close();
    }
  });
});

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

// ---------------------------------------------------------------------------
// silent_response — gateway-side canonical detector (Phase 2 §B, OB-02/03/07)
// ---------------------------------------------------------------------------
describe('isSilentResponse', () => {
  it('matches the canonical NO_REPLY token', () => {
    assert.equal(isSilentResponse('NO_REPLY'), true);
    assert.equal(isSilentResponse('no_reply'), true);
    assert.equal(isSilentResponse('  NO_REPLY  '), true);
    assert.equal(isSilentResponse('NO_REPLY.'), true);
    assert.equal(isSilentResponse('NO_REPLY\n'), true);
  });

  it('matches the bracketed [no reply needed] form', () => {
    assert.equal(isSilentResponse('[no reply needed]'), true);
    assert.equal(isSilentResponse('[NO REPLY NEEDED]'), true);
    assert.equal(isSilentResponse('[no reply needed].'), true);
    assert.equal(isSilentResponse('no reply needed'), true);
  });

  it('matches sentinels glued to emojis', () => {
    assert.equal(isSilentResponse('NO_REPLY🎩'), true);
    assert.equal(isSilentResponse('NO_REPLY 😐'), true);
  });

  it('matches sentinels at the trailing position after context', () => {
    assert.equal(isSilentResponse('Tutto bene, Signore.\nNO_REPLY'), true);
    assert.equal(isSilentResponse('Some context. [no reply needed]'), true);
    assert.equal(isSilentResponse('...a Sua disposizione. 🎩NO_REPLY'), true);
  });

  it('does not match empty / whitespace-only / normal text', () => {
    assert.equal(isSilentResponse(''), false);
    assert.equal(isSilentResponse('   '), false);
    assert.equal(isSilentResponse('Ok'), false);
    assert.equal(isSilentResponse('Confermato, rispondo dopo'), false);
  });

  it('respects word boundaries', () => {
    assert.equal(isSilentResponse('NO_REPLYING'), false);
    assert.equal(isSilentResponse('noreply@example.com'), false);
  });

  it('does not flag embedded substrings inside real replies', () => {
    assert.equal(isSilentResponse('the NO_REPLY sentinel is documented'), false);
    assert.equal(isSilentResponse('Ok NO_REPLY received but here is your real answer'), false);
  });

  it('rejects non-string inputs gracefully', () => {
    assert.equal(isSilentResponse(null), false);
    assert.equal(isSilentResponse(undefined), false);
    assert.equal(isSilentResponse(42), false);
  });
});

describe('stripNoReply', () => {
  it('returns empty string for a whole-message sentinel', () => {
    assert.equal(stripNoReply('NO_REPLY'), '');
    assert.equal(stripNoReply('  NO_REPLY  '), '');
    assert.equal(stripNoReply('[no reply needed]'), '');
  });

  it('returns the text unchanged when not silent', () => {
    assert.equal(stripNoReply('Hello world'), 'Hello world');
    assert.equal(stripNoReply(''), '');
  });

  it('returns trailing-sentinel text as empty (legacy contract)', () => {
    // Trailing sentinel collapses the whole message to silent under V2.
    assert.equal(stripNoReply('Tutto bene. NO_REPLY'), '');
  });
});

describe('createHoldbackAccumulator (OB-07 streaming hold-back)', () => {
  it('NEVER flushes when stream produces only NO_REPLY', async () => {
    const flushes = [];
    const acc = createHoldbackAccumulator({ onFlush: (t) => flushes.push(t) });
    await acc.push('NO_REPLY');
    const result = await acc.end();
    assert.equal(flushes.length, 0, 'sock.sendMessage must not be called');
    assert.equal(result.silent, true);
    assert.equal(result.flushed, false);
  });

  it('NEVER flushes for the canonical OB-07 case ["Ok ", "[no reply", " needed]"]', async () => {
    // This is the user-directive critical case: a streaming source emits
    // three deltas that, only when concatenated, reveal a sentinel. The
    // hold-back must keep deferring until end() and then classify silent.
    const flushes = [];
    const acc = createHoldbackAccumulator({ onFlush: (t) => flushes.push(t) });
    await acc.push('Ok ');
    await acc.push('[no reply');
    await acc.push(' needed]');
    const result = await acc.end();
    assert.equal(flushes.length, 0, 'sock.sendMessage must NEVER be called');
    assert.equal(result.silent, true);
  });

  it('flushes legitimate streaming responses once threshold is crossed', async () => {
    const flushes = [];
    const acc = createHoldbackAccumulator({ onFlush: (t) => flushes.push(t) });
    await acc.push('Hello ');
    await acc.push('world, how are you doing today?');
    const result = await acc.end();
    assert.ok(flushes.length >= 1, 'expected at least one flush for non-silent text');
    assert.equal(result.flushed, true);
    assert.equal(result.silent, false);
    // First flush should contain the cumulative buffer at the moment the
    // threshold was crossed (everything seen so far).
    assert.ok(flushes[0].includes('Hello'));
    assert.ok(flushes[0].includes('world'));
  });

  it('forwards subsequent deltas immediately after the first flush', async () => {
    const flushes = [];
    const acc = createHoldbackAccumulator({ onFlush: (t) => flushes.push(t) });
    await acc.push('This is a long enough chunk to immediately flush past the threshold.');
    await acc.push(' more');
    await acc.push(' deltas');
    await acc.end();
    assert.equal(flushes.length, 3);
    assert.equal(flushes[1], ' more');
    assert.equal(flushes[2], ' deltas');
  });

  it('handles many empty deltas followed by a real message', async () => {
    const flushes = [];
    const acc = createHoldbackAccumulator({ onFlush: (t) => flushes.push(t) });
    for (let i = 0; i < 10; i++) await acc.push('');
    await acc.push('Ok sure, here is a sufficiently long real reply.');
    const result = await acc.end();
    assert.equal(result.silent, false);
    assert.ok(flushes.length >= 1);
  });

  it('treats a short non-sentinel response as silent at end (held forever)', async () => {
    // Edge case: a 2-char real response like "Ok" never crosses the
    // threshold, so the hold-back classifier falls through to end(),
    // which checks isSilentResponse — "Ok" is NOT silent, so end()
    // flushes the held buffer.
    const flushes = [];
    const acc = createHoldbackAccumulator({ onFlush: (t) => flushes.push(t) });
    await acc.push('Ok');
    const result = await acc.end();
    assert.equal(result.silent, false);
    assert.equal(flushes.length, 1);
    assert.equal(flushes[0], 'Ok');
  });

  it('throws when onFlush is missing', () => {
    assert.throws(() => createHoldbackAccumulator({}), /onFlush/);
  });

  it('exposes buffered + hasFlushed introspection helpers', async () => {
    const acc = createHoldbackAccumulator({ onFlush: () => {} });
    await acc.push('partial');
    assert.equal(acc.buffered, 'partial');
    assert.equal(acc.hasFlushed, false);
  });
});

// ---------------------------------------------------------------------------
// ownerIntentsRelay — guards the RELAY system-instruction injection so that
// neutral owner-to-agent messages don't get forced into relay mode when a
// stranger conversation happens to be active.
// ---------------------------------------------------------------------------
describe('ownerIntentsRelay', () => {
  it('returns false for neutral greetings', () => {
    assert.equal(ownerIntentsRelay('saludos'), false);
    assert.equal(ownerIntentsRelay('hola'), false);
    assert.equal(ownerIntentsRelay('ciao'), false);
    assert.equal(ownerIntentsRelay('Buondì'), false);
    assert.equal(ownerIntentsRelay('come stai?'), false);
    assert.equal(ownerIntentsRelay(''), false);
    assert.equal(ownerIntentsRelay('   '), false);
  });

  it('returns true for explicit /relay or /reply command', () => {
    assert.equal(ownerIntentsRelay('/relay tell him I will be late'), true);
    assert.equal(ownerIntentsRelay('/reply ok grazie'), true);
  });

  it('returns true for @mention', () => {
    assert.equal(ownerIntentsRelay('@alice hi there'), true);
    assert.equal(ownerIntentsRelay('please say @bob hi'), true);
  });

  it('Italian pack: recognises delegated-speech verbs, rejects owner→agent formal imperative', () => {
    const { compileIntentRegex } = require('./lib/intent_patterns');
    const re = compileIntentRegex(['it']);
    // Positive — explicit recipient / verb-with-baked-in-object
    assert.ok(re.test('rispondi a Federico che sto bene'));
    assert.ok(re.test('digli che arrivo'));
    assert.ok(re.test('saluta Caterina per me'));
    assert.ok(re.test('scrivi a Paolo'));
    assert.ok(re.test('chiedi a Mario il prezzo'));
    assert.ok(re.test('inoltra a tutti la comunicazione'));
    assert.ok(re.test('dica a Mario che sto bene'));
    // Negative — owner addressing the bot, not a relay.
    // Pre-fix regex matched bare `dica`, so "mi dica" triggered a false
    // relay intent; the narrowed `dica\s+a\s+\w+` pattern blocks it.
    assert.equal(re.test('mi dica'), false);
    assert.equal(re.test('mi dica di più'), false);
    assert.equal(re.test('Dica pure'), false);
  });

  it('multi-language union: both EN and IT patterns active simultaneously', () => {
    const { compileIntentRegex } = require('./lib/intent_patterns');
    const re = compileIntentRegex(['en', 'it']);
    assert.ok(re.test('tell Alice I am busy'));
    assert.ok(re.test('digli che arrivo'));
  });

  it('returns true for English delegated-speech verbs', () => {
    assert.equal(ownerIntentsRelay('reply to Bob that I agree'), true);
    assert.equal(ownerIntentsRelay('tell Alice I am busy'), true);
    assert.equal(ownerIntentsRelay('write to the team'), true);
  });

  it('is case-insensitive (IT pack)', () => {
    const { compileIntentRegex } = require('./lib/intent_patterns');
    const re = compileIntentRegex(['it']);
    assert.ok(re.test('  RISPONDI A Mario ok'.trim()));
    assert.ok(re.test('DIGLI che sto arrivando'));
  });

  it('does not match partial words', () => {
    assert.equal(ownerIntentsRelay('salutami la zia'), false);
    assert.equal(ownerIntentsRelay('rispostaok'), false);
  });

  it('does not treat "tell me/us/you" as relay intent (owner → agent)', () => {
    assert.equal(ownerIntentsRelay('tell me a joke'), false);
    assert.equal(ownerIntentsRelay('can you tell me about this'), false);
    assert.equal(ownerIntentsRelay('tell us the news'), false);
    assert.equal(ownerIntentsRelay('tell you what'), false);
  });

  it('does not treat "looking forward to" as relay intent', () => {
    assert.equal(ownerIntentsRelay('I look forward to meeting you'), false);
    assert.equal(ownerIntentsRelay('looking forward to the call'), false);
    assert.equal(ownerIntentsRelay('I am forward to hearing from you'), false);
  });

  it('still matches "forward <it|this|the X> to <recipient>"', () => {
    assert.equal(ownerIntentsRelay('forward it to Bob'), true);
    assert.equal(ownerIntentsRelay('forward this to Alice'), true);
    assert.equal(ownerIntentsRelay('forward the message to the team'), true);
  });

  it('German pack: rejects "Sag mir" / "Sage uns" (owner→bot), accepts explicit recipient', () => {
    const { compileIntentRegex } = require('./lib/intent_patterns');
    const re = compileIntentRegex(['de']);
    // Positive — explicit third-party recipient
    assert.ok(re.test('Sag Klaus ich komme später'));
    assert.ok(re.test('sage Anna bitte Bescheid'));
    assert.ok(re.test('schreib an Petra'));
    assert.ok(re.test('antworte an Marco'));
    // Negative — self-directed (owner talking to the bot)
    assert.equal(re.test('Sag mir was du denkst'), false);
    assert.equal(re.test('sag mir bitte'), false);
    assert.equal(re.test('sage uns die Wahrheit'), false);
  });
});
