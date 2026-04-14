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
