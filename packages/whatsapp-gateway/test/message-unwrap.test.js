'use strict';

// Unit tests for unwrapMessageWrappers — the gateway helper that collapses
// nested WhatsApp message wrappers (ephemeralMessage / viewOnceMessage /
// editedMessage / deviceSentMessage / documentWithCaptionMessage) into the
// inner payload so downstream handlers see contextInfo (quotedMessage,
// mentionedJid, forwarded) regardless of which combination of wrappers WA
// applied. Coverage drives houko's CHANGES_REQUESTED on PR #5229:
//   (a) each wrapper variant in isolation,
//   (b) editedMessage→ephemeralMessage→extendedTextMessage nesting,
//   (c) non-wrapped passthrough,
//   (d) null / undefined input,
//   (e) deviceSentMessage,
//   (f) contextInfo.quotedMessage resolves after unwrap for at least one
//       wrapper.

const assert = require('node:assert/strict');
const { describe, it, after } = require('node:test');

// index.js performs work at require-time (DB init); point it at a temp DB
// and a port nothing is listening on so the http server is created but the
// require.main guard keeps it from actually starting.
process.env.WHATSAPP_DB_PATH = '/tmp/test-wa-unwrap-' + process.pid + '.db';
process.env.LIBREFANG_URL = 'http://127.0.0.1:24599';

const { unwrapMessageWrappers, MAX_UNWRAP_DEPTH } = require('../index.js');

// Synthetic inner payload — what handlers actually want to read.
function makeText(text, quotedText) {
  const m = { extendedTextMessage: { text } };
  if (quotedText) {
    m.extendedTextMessage.contextInfo = {
      quotedMessage: { conversation: quotedText },
      participant: '123@s.whatsapp.net',
      stanzaId: 'STANZA1',
    };
  }
  return m;
}

describe('unwrapMessageWrappers — single-level wrappers (a)', () => {
  it('unwraps ephemeralMessage', () => {
    const inner = makeText('hi');
    const wrapped = { ephemeralMessage: { message: inner } };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps viewOnceMessage', () => {
    const inner = makeText('hi');
    const wrapped = { viewOnceMessage: { message: inner } };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps viewOnceMessageV2', () => {
    const inner = makeText('hi');
    const wrapped = { viewOnceMessageV2: { message: inner } };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps viewOnceMessageV2Extension', () => {
    const inner = makeText('hi');
    const wrapped = { viewOnceMessageV2Extension: { message: inner } };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps editedMessage', () => {
    const inner = makeText('edited');
    const wrapped = { editedMessage: { message: inner } };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps documentWithCaptionMessage', () => {
    const inner = { documentMessage: { caption: 'doc' } };
    const wrapped = { documentWithCaptionMessage: { message: inner } };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });
});

describe('unwrapMessageWrappers — nested wrappers (b)', () => {
  it('unwraps editedMessage → ephemeralMessage → extendedTextMessage', () => {
    const inner = makeText('nested');
    const wrapped = {
      editedMessage: {
        message: {
          ephemeralMessage: { message: inner },
        },
      },
    };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps viewOnceMessageV2 inside ephemeralMessage (disappearing mode)', () => {
    const inner = { imageMessage: { caption: 'view-once-in-ephemeral' } };
    const wrapped = {
      ephemeralMessage: {
        message: {
          viewOnceMessageV2: { message: inner },
        },
      },
    };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps a 3-deep chain edited → ephemeral → documentWithCaption', () => {
    const inner = { documentMessage: { caption: 'three-deep' } };
    const wrapped = {
      editedMessage: {
        message: {
          ephemeralMessage: {
            message: {
              documentWithCaptionMessage: { message: inner },
            },
          },
        },
      },
    };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('caps recursion at MAX_UNWRAP_DEPTH (returns partially unwrapped, never throws)', () => {
    // Build a chain of N+1 ephemeralMessage wrappers around a final inner.
    const inner = makeText('too-deep');
    let chain = inner;
    for (let i = 0; i < MAX_UNWRAP_DEPTH + 2; i++) {
      chain = { ephemeralMessage: { message: chain } };
    }
    const out = unwrapMessageWrappers(chain);
    // We can't reach `inner` (depth-capped). The result must still be a
    // valid (truthy, non-throwing) message-shaped object.
    assert.ok(out);
    assert.ok(typeof out === 'object');
  });

  it('handles a hand-crafted cycle without exploding the stack', () => {
    // Adversarial: m.ephemeralMessage.message === m. With recursion bounded,
    // this must terminate. The `inner === m` short-circuit covers it
    // directly even before MAX_UNWRAP_DEPTH triggers.
    const cycle = {};
    cycle.ephemeralMessage = { message: cycle };
    const out = unwrapMessageWrappers(cycle);
    assert.equal(out, cycle);
  });
});

describe('unwrapMessageWrappers — passthrough (c)', () => {
  it('returns a non-wrapped extendedTextMessage unchanged', () => {
    const m = makeText('plain');
    assert.equal(unwrapMessageWrappers(m), m);
  });

  it('returns a conversation-only message unchanged', () => {
    const m = { conversation: 'hello' };
    assert.equal(unwrapMessageWrappers(m), m);
  });

  it('returns a media-only message unchanged', () => {
    const m = { imageMessage: { caption: 'pic' } };
    assert.equal(unwrapMessageWrappers(m), m);
  });

  it('returns a protocolMessage unchanged (NOT a content wrapper)', () => {
    const m = { protocolMessage: { type: 0, key: { id: 'X' } } };
    assert.equal(unwrapMessageWrappers(m), m);
  });
});

describe('unwrapMessageWrappers — null / undefined (d)', () => {
  it('returns null for null input', () => {
    assert.equal(unwrapMessageWrappers(null), null);
  });
  it('returns undefined for undefined input', () => {
    assert.equal(unwrapMessageWrappers(undefined), undefined);
  });
  it('returns empty object unchanged', () => {
    const m = {};
    assert.equal(unwrapMessageWrappers(m), m);
  });
});

describe('unwrapMessageWrappers — deviceSentMessage (e)', () => {
  it('unwraps a deviceSentMessage payload (WA Web reply from sibling device)', () => {
    const inner = makeText('from-other-device');
    const wrapped = {
      deviceSentMessage: {
        destinationJid: 'peer@s.whatsapp.net',
        message: inner,
      },
    };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });

  it('unwraps deviceSentMessage nested inside an editedMessage', () => {
    const inner = makeText('edited-from-other-device');
    const wrapped = {
      editedMessage: {
        message: {
          deviceSentMessage: { message: inner },
        },
      },
    };
    assert.deepEqual(unwrapMessageWrappers(wrapped), inner);
  });
});

describe('unwrapMessageWrappers — contextInfo resolves after unwrap (f)', () => {
  it('quotedMessage is visible on the unwrapped extendedTextMessage', () => {
    const inner = makeText('reply', 'original text being quoted');
    const wrapped = { ephemeralMessage: { message: inner } };
    const out = unwrapMessageWrappers(wrapped);
    assert.equal(
      out.extendedTextMessage.contextInfo.quotedMessage.conversation,
      'original text being quoted',
      'quotedMessage must survive the unwrap so the [In risposta a: …] prefix can be built',
    );
    assert.equal(out.extendedTextMessage.contextInfo.stanzaId, 'STANZA1');
  });

  it('quotedMessage survives a 2-deep edited-of-ephemeral unwrap', () => {
    const inner = makeText('edit', 'quoted via deep nesting');
    const wrapped = {
      editedMessage: {
        message: { ephemeralMessage: { message: inner } },
      },
    };
    const out = unwrapMessageWrappers(wrapped);
    assert.equal(
      out.extendedTextMessage.contextInfo.quotedMessage.conversation,
      'quoted via deep nesting',
    );
  });
});

// Force exit — index.js leaves the SQLite handle + reconnect/catchup
// setInterval timers attached to the event loop. Without this hook the test
// process hangs after all assertions pass. Pattern mirrors index.test.js:1277.
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
  setTimeout(() => process.exit(0), 100);
});
