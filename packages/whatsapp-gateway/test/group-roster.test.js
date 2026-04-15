'use strict';

// Phase 2 §C — group participant roster cache + invalidation tests (GS-01).
//
// Run via `node --test packages/whatsapp-gateway/test/group-roster.test.js`
// or, from inside the package, `node --test test/group-roster.test.js`.

const assert = require('node:assert/strict');
const { describe, it, beforeEach } = require('node:test');

// Override DB path before requiring the module — same pattern as index.test.js.
process.env.WHATSAPP_DB_PATH =
  '/tmp/test-wa-gateway-roster-' + process.pid + '.db';

const {
  getGroupParticipants,
  invalidateGroupRoster,
  groupMetadataCache,
} = require('../index.js');

const GROUP_JID = '120363000000000000@g.us';

function fixtureMeta() {
  return {
    id: GROUP_JID,
    participants: [
      { id: '391112223333@s.whatsapp.net', notify: 'Caterina' },
      { id: '391999888777@s.whatsapp.net', notify: 'Ambrogio' },
      { id: '393334445555@s.whatsapp.net', name: 'Marco' },
    ],
  };
}

function makeSock(throwsAfter = -1) {
  let calls = 0;
  return {
    calls() {
      return calls;
    },
    groupMetadata: async (jid) => {
      calls += 1;
      if (throwsAfter >= 0 && calls > throwsAfter) {
        throw new Error('socket disconnected');
      }
      assert.equal(jid, GROUP_JID);
      return fixtureMeta();
    },
  };
}

describe('group roster cache', () => {
  beforeEach(() => {
    groupMetadataCache.clear();
  });

  it('caches roster across 3 inbounds (1 network call)', async () => {
    const sock = makeSock();
    const r1 = await getGroupParticipants(sock, GROUP_JID);
    const r2 = await getGroupParticipants(sock, GROUP_JID);
    const r3 = await getGroupParticipants(sock, GROUP_JID);
    assert.equal(sock.calls(), 1, 'only first call hits the network');
    assert.equal(r1.length, 3);
    assert.deepEqual(r1, r2);
    assert.deepEqual(r1, r3);
    // Display-name resolution: notify > name > id-prefix
    assert.equal(r1[0].display_name, 'Caterina');
    assert.equal(r1[2].display_name, 'Marco');
  });

  it('re-fetches after invalidation', async () => {
    const sock = makeSock();
    await getGroupParticipants(sock, GROUP_JID);
    await getGroupParticipants(sock, GROUP_JID);
    invalidateGroupRoster(GROUP_JID);
    await getGroupParticipants(sock, GROUP_JID);
    assert.equal(sock.calls(), 2, 'invalidation forces a 2nd network call');
  });

  it('returns empty array on fetch failure (graceful degradation)', async () => {
    const sock = {
      groupMetadata: async () => {
        throw new Error('not connected');
      },
    };
    const r = await getGroupParticipants(sock, GROUP_JID);
    assert.deepEqual(r, []);
    assert.equal(
      groupMetadataCache.has(GROUP_JID),
      false,
      'failed fetch must not poison cache',
    );
  });

  it('returns empty array for non-group JID without calling the network', async () => {
    const sock = makeSock();
    const r = await getGroupParticipants(sock, '391112223333@s.whatsapp.net');
    assert.deepEqual(r, []);
    assert.equal(sock.calls(), 0);
  });

  it('returns empty array for falsy JID', async () => {
    const sock = makeSock();
    assert.deepEqual(await getGroupParticipants(sock, ''), []);
    assert.deepEqual(await getGroupParticipants(sock, null), []);
    assert.equal(sock.calls(), 0);
  });
});
