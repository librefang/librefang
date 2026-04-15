'use strict';

// ---------------------------------------------------------------------------
// test/group-activation.test.js — Phase 5 §A (GA-01) unit tests.
//
// All tests use an in-memory better-sqlite3 database (`:memory:`).
// ---------------------------------------------------------------------------

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');
const Database = require('better-sqlite3');

const groupActivation = require('../lib/group-activation');

const G1 = '120363100000000001@g.us';
const G2 = '120363100000000002@g.us';

function freshDb() {
  const db = new Database(':memory:');
  groupActivation.init(db);
  return db;
}

describe('group-activation', () => {
  describe('init', () => {
    it('creates the group_activation table on a blank DB', () => {
      const db = freshDb();
      const row = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='group_activation'")
        .get();
      assert.equal(row?.name, 'group_activation');
    });

    it('is idempotent — running twice does not error', () => {
      const db = freshDb();
      assert.doesNotThrow(() => groupActivation.init(db));
    });
  });

  describe('get / set', () => {
    it('returns null for unknown group', () => {
      const db = freshDb();
      assert.equal(groupActivation.get(db, G1), null);
    });

    it('persists a mode and reads it back', () => {
      const db = freshDb();
      groupActivation.set(db, G1, 'always');
      assert.equal(groupActivation.get(db, G1), 'always');
    });

    it('REPLACE updates mode for the same group', () => {
      const db = freshDb();
      groupActivation.set(db, G1, 'always');
      groupActivation.set(db, G1, 'off');
      assert.equal(groupActivation.get(db, G1), 'off');
    });

    it('rejects invalid mode in set', () => {
      const db = freshDb();
      assert.throws(() => groupActivation.set(db, G1, 'spam'), /invalid mode/);
    });

    it('rejects empty groupJid in set', () => {
      const db = freshDb();
      assert.throws(() => groupActivation.set(db, '', 'always'), /empty groupJid/);
    });

    it('returns null for empty groupJid in get', () => {
      const db = freshDb();
      assert.equal(groupActivation.get(db, ''), null);
    });
  });

  describe('remove / list', () => {
    it('remove deletes a row, get returns null afterwards', () => {
      const db = freshDb();
      groupActivation.set(db, G1, 'always');
      groupActivation.remove(db, G1);
      assert.equal(groupActivation.get(db, G1), null);
    });

    it('list returns rows ordered by updated_at desc', () => {
      const db = freshDb();
      groupActivation.set(db, G1, 'always');
      // Force a clearly-later timestamp by busy-waiting one ms boundary.
      const start = Date.now();
      while (Date.now() === start) { /* spin */ }
      groupActivation.set(db, G2, 'mention');
      const rows = groupActivation.list(db);
      assert.equal(rows.length, 2);
      assert.equal(rows[0].group_jid, G2);
      assert.equal(rows[1].group_jid, G1);
    });
  });

  describe('parseCommand', () => {
    it('parses /activation always', () => {
      assert.deepEqual(groupActivation.parseCommand('/activation always'), { mode: 'always' });
    });

    it('parses /activation mention', () => {
      assert.deepEqual(groupActivation.parseCommand('/activation mention'), { mode: 'mention' });
    });

    it('parses /activation off', () => {
      assert.deepEqual(groupActivation.parseCommand('/activation off'), { mode: 'off' });
    });

    it('is case-insensitive on the command and the argument', () => {
      assert.deepEqual(groupActivation.parseCommand('/Activation OFF'), { mode: 'off' });
    });

    it('tolerates surrounding whitespace', () => {
      assert.deepEqual(groupActivation.parseCommand('   /activation   always   '), { mode: 'always' });
    });

    it('returns { query: true } for a bare /activation', () => {
      assert.deepEqual(groupActivation.parseCommand('/activation'), { query: true });
    });

    it('returns { error } for an unknown mode', () => {
      assert.deepEqual(groupActivation.parseCommand('/activation spam'), { error: 'invalid_mode', arg: 'spam' });
    });

    it('returns null for non-command text', () => {
      assert.equal(groupActivation.parseCommand('hello /activation always'), null);
      assert.equal(groupActivation.parseCommand('look at /activation'), null);
      assert.equal(groupActivation.parseCommand(''), null);
      assert.equal(groupActivation.parseCommand(null), null);
      assert.equal(groupActivation.parseCommand(undefined), null);
    });

    it('parses DM form /activation <groupJid> <mode>', () => {
      assert.deepEqual(
        groupActivation.parseCommand('/activation 120363100000000001@g.us mention'),
        { targetGroup: '120363100000000001@g.us', mode: 'mention' },
      );
    });

    it('parses DM form with only groupJid as a query', () => {
      assert.deepEqual(
        groupActivation.parseCommand('/activation 120363100000000001@g.us'),
        { targetGroup: '120363100000000001@g.us', query: true },
      );
    });

    it('rejects a first arg that is not @g.us and not a valid mode', () => {
      assert.deepEqual(
        groupActivation.parseCommand('/activation somechat@s.whatsapp.net'),
        { error: 'invalid_mode', arg: 'somechat@s.whatsapp.net' },
      );
    });

    it('rejects a target+bad-mode combination with invalid_mode', () => {
      assert.deepEqual(
        groupActivation.parseCommand('/activation 120363100000000001@g.us spam'),
        { error: 'invalid_mode', arg: 'spam' },
      );
    });

    it('rejects a non-group first arg when a second arg is present', () => {
      assert.deepEqual(
        groupActivation.parseCommand('/activation bogus mention'),
        { error: 'invalid_target', arg: 'bogus' },
      );
    });
  });

  describe('MODES / DEFAULT_MODE', () => {
    it('MODES exposes the three valid modes', () => {
      assert.deepEqual(groupActivation.MODES, ['always', 'mention', 'off']);
    });

    it('DEFAULT_MODE is mention', () => {
      assert.equal(groupActivation.DEFAULT_MODE, 'mention');
    });
  });
});
