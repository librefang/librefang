'use strict';

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');

const { buildSessionKey, channelTypeForChat } = require('../lib/session-key');

describe('buildSessionKey', () => {
  it('Test 1: returns composite <agent>:<peer>:<chatJid> string', () => {
    assert.equal(
      buildSessionKey('agent-uuid', '+391234', '391234@s.whatsapp.net'),
      'agent-uuid:+391234:391234@s.whatsapp.net'
    );
  });

  it('Test 2: falls back to "unknown" for missing parts (robust against partial context)', () => {
    assert.equal(buildSessionKey(undefined, null, ''), 'unknown:unknown:unknown');
    assert.equal(buildSessionKey('a', null, 'c'), 'a:unknown:c');
  });

  it('Test 3: distinct chatJids produce distinct session keys', () => {
    const a = 'agent';
    const p = '+39123';
    const keyGroupA = buildSessionKey(a, p, '111-aaa@g.us');
    const keyGroupB = buildSessionKey(a, p, '222-bbb@g.us');
    assert.notEqual(keyGroupA, keyGroupB);
  });
});

describe('channelTypeForChat', () => {
  it('Test 4: returns "whatsapp:<jid>" for a non-empty chatJid', () => {
    assert.equal(
      channelTypeForChat('391234@s.whatsapp.net'),
      'whatsapp:391234@s.whatsapp.net'
    );
  });

  it('Test 5: returns bare "whatsapp" for empty/undefined chatJid', () => {
    assert.equal(channelTypeForChat(''), 'whatsapp');
    assert.equal(channelTypeForChat(undefined), 'whatsapp');
    assert.equal(channelTypeForChat(null), 'whatsapp');
  });

  it('Test 6: CS-01 invariant — distinct chatJids yield distinct channel_type strings', () => {
    const a = channelTypeForChat('111@s.whatsapp.net');
    const b = channelTypeForChat('222@s.whatsapp.net');
    assert.notEqual(a, b);
    assert.ok(a.startsWith('whatsapp:'));
    assert.ok(b.startsWith('whatsapp:'));
  });
});
