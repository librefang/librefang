'use strict';

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');

const {
  isLidJid,
  isGroupJid,
  normalizeDeviceScopedJid,
  extractE164,
  phoneToJid,
  resolvePeerId,
  deriveOwnerJids,
} = require('../lib/identity');

describe('isLidJid', () => {
  it('detects @lid suffix', () => {
    assert.equal(isLidJid('123@lid'), true);
  });
  it('detects @hosted.lid suffix', () => {
    assert.equal(isLidJid('123@hosted.lid'), true);
  });
  it('rejects phone JID', () => {
    assert.equal(isLidJid('123@s.whatsapp.net'), false);
  });
  it('rejects group JID', () => {
    assert.equal(isLidJid('123-456@g.us'), false);
  });
  it('rejects empty / null / undefined', () => {
    assert.equal(isLidJid(''), false);
    assert.equal(isLidJid(null), false);
    assert.equal(isLidJid(undefined), false);
  });
  it('rejects non-string input', () => {
    assert.equal(isLidJid(123), false);
    assert.equal(isLidJid({}), false);
  });
});

describe('isGroupJid', () => {
  it('detects @g.us suffix', () => {
    assert.equal(isGroupJid('123-456@g.us'), true);
  });
  it('rejects @lid', () => {
    assert.equal(isGroupJid('123@lid'), false);
  });
  it('rejects phone JID', () => {
    assert.equal(isGroupJid('123@s.whatsapp.net'), false);
  });
  it('rejects empty / null', () => {
    assert.equal(isGroupJid(''), false);
    assert.equal(isGroupJid(null), false);
  });
});

describe('normalizeDeviceScopedJid', () => {
  it('strips :<device> from phone JID', () => {
    assert.equal(normalizeDeviceScopedJid('123:45@s.whatsapp.net'), '123@s.whatsapp.net');
  });
  it('passthrough plain phone JID', () => {
    assert.equal(normalizeDeviceScopedJid('123@s.whatsapp.net'), '123@s.whatsapp.net');
  });
  it('strips :<device> from LID', () => {
    assert.equal(normalizeDeviceScopedJid('123:45@lid'), '123@lid');
  });
  it('leaves group JID untouched', () => {
    assert.equal(normalizeDeviceScopedJid('123-456@g.us'), '123-456@g.us');
  });
  it('empty passthrough', () => {
    assert.equal(normalizeDeviceScopedJid(''), '');
    assert.equal(normalizeDeviceScopedJid(null), '');
    assert.equal(normalizeDeviceScopedJid(undefined), '');
  });
});

describe('extractE164', () => {
  it('returns +E164 for phone JID', () => {
    assert.equal(extractE164('393331234567@s.whatsapp.net'), '+393331234567');
  });
  it('strips device suffix first', () => {
    assert.equal(extractE164('393331234567:3@s.whatsapp.net'), '+393331234567');
  });
  it('returns empty for LID', () => {
    assert.equal(extractE164('123@lid'), '');
  });
  it('returns empty for hosted.lid', () => {
    assert.equal(extractE164('123@hosted.lid'), '');
  });
  it('returns empty for group JID', () => {
    assert.equal(extractE164('123-456@g.us'), '');
  });
  it('returns empty for empty / null', () => {
    assert.equal(extractE164(''), '');
    assert.equal(extractE164(null), '');
  });
});

describe('phoneToJid', () => {
  it('+E164 -> phone JID', () => {
    assert.equal(phoneToJid('+393331234567'), '393331234567@s.whatsapp.net');
  });
  it('bare digits -> phone JID', () => {
    assert.equal(phoneToJid('393331234567'), '393331234567@s.whatsapp.net');
  });
  it('group JID passthrough', () => {
    assert.equal(phoneToJid('123-456@g.us'), '123-456@g.us');
  });
  it('already-formed phone JID passthrough', () => {
    assert.equal(phoneToJid('123@s.whatsapp.net'), '123@s.whatsapp.net');
  });
  it('empty / null -> empty', () => {
    assert.equal(phoneToJid(''), '');
    assert.equal(phoneToJid(null), '');
  });
});

describe('deriveOwnerJids', () => {
  it('maps +E164 list to JID Set', () => {
    const got = deriveOwnerJids(['+39111', '+39222']);
    assert.ok(got instanceof Set);
    assert.equal(got.size, 2);
    assert.ok(got.has('39111@s.whatsapp.net'));
    assert.ok(got.has('39222@s.whatsapp.net'));
  });
  it('empty list -> empty Set', () => {
    const got = deriveOwnerJids([]);
    assert.equal(got.size, 0);
  });
  it('non-array -> empty Set', () => {
    assert.equal(deriveOwnerJids(null).size, 0);
    assert.equal(deriveOwnerJids(undefined).size, 0);
  });
  it('filters junk entries', () => {
    const got = deriveOwnerJids(['+39111', '', null, 123]);
    assert.equal(got.size, 1);
    assert.ok(got.has('39111@s.whatsapp.net'));
  });
});

describe('resolvePeerId', () => {
  it('step 1: senderPn present -> direct', () => {
    const r = resolvePeerId('123@lid', { senderPn: '+391234@s.whatsapp.net', lidToPnCache: new Map() });
    assert.equal(r.confidence, 'direct');
    assert.equal(r.peer, '+391234@s.whatsapp.net');
  });
  it('step 2: group JID -> group', () => {
    const r = resolvePeerId('123-456@g.us', { lidToPnCache: new Map() });
    assert.equal(r.confidence, 'group');
    assert.equal(r.peer, '123-456@g.us');
  });
  it('step 3: LID in cache -> cache', () => {
    const cache = new Map([['123@lid', '391234@s.whatsapp.net']]);
    const r = resolvePeerId('123@lid', { lidToPnCache: cache });
    assert.equal(r.confidence, 'cache');
    assert.equal(r.peer, '391234@s.whatsapp.net');
  });
  it('step 4: plain phone JID -> direct (normalized)', () => {
    const r = resolvePeerId('391234@s.whatsapp.net', { lidToPnCache: new Map() });
    assert.equal(r.confidence, 'direct');
    assert.equal(r.peer, '391234@s.whatsapp.net');
  });
  it('step 4: device-scoped phone JID -> direct, normalized', () => {
    const r = resolvePeerId('391234:45@s.whatsapp.net', { lidToPnCache: new Map() });
    assert.equal(r.confidence, 'direct');
    assert.equal(r.peer, '391234@s.whatsapp.net');
  });
  it('step 5: LID not in cache, participant non-LID -> participant', () => {
    const r = resolvePeerId('123@lid', {
      lidToPnCache: new Map(),
      participant: '391234@s.whatsapp.net',
    });
    assert.equal(r.confidence, 'participant');
    assert.equal(r.peer, '391234@s.whatsapp.net');
  });
  it('step 5: participant is device-scoped -> normalized', () => {
    const r = resolvePeerId('123@lid', {
      lidToPnCache: new Map(),
      participant: '391234:7@s.whatsapp.net',
    });
    assert.equal(r.confidence, 'participant');
    assert.equal(r.peer, '391234@s.whatsapp.net');
  });
  it('step 6: LID, no cache, no participant -> lid_unresolved', () => {
    const r = resolvePeerId('123@lid', { lidToPnCache: new Map() });
    assert.equal(r.confidence, 'lid_unresolved');
    assert.equal(r.peer, '');
  });
  it('step 6: LID, no cache, LID participant -> lid_unresolved', () => {
    const r = resolvePeerId('123@lid', {
      lidToPnCache: new Map(),
      participant: '456@lid',
    });
    assert.equal(r.confidence, 'lid_unresolved');
  });
  it('missing options object does not throw', () => {
    const r = resolvePeerId('391234@s.whatsapp.net');
    assert.equal(r.confidence, 'direct');
    assert.equal(r.peer, '391234@s.whatsapp.net');
  });
  it('senderPn wins over group (defensive, rare)', () => {
    const r = resolvePeerId('123-456@g.us', { senderPn: '+391234@s.whatsapp.net' });
    assert.equal(r.confidence, 'direct');
  });
});
