'use strict';

// ---------------------------------------------------------------------------
// test/group-allow-from.test.js — Phase 3 completion (GA-02) unit tests.
// ---------------------------------------------------------------------------

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');
const Database = require('better-sqlite3');

const groupAllowFrom = require('../lib/group-allow-from');

const G1 = '120363100000000001@g.us';
const G2 = '120363100000000002@g.us';
const S_ALICE = '391230000001@s.whatsapp.net';
const S_BOB   = '391230000002@s.whatsapp.net';
const L_CHRIS = '333333333@lid';

function freshDb() {
  const db = new Database(':memory:');
  groupAllowFrom.init(db);
  return db;
}

describe('group-allow-from', () => {
  describe('init', () => {
    it('creates the group_allow_from table', () => {
      const db = freshDb();
      const row = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='group_allow_from'")
        .get();
      assert.equal(row?.name, 'group_allow_from');
    });

    it('is idempotent', () => {
      const db = freshDb();
      assert.doesNotThrow(() => groupAllowFrom.init(db));
    });
  });

  describe('hasAny / isAllowed', () => {
    it('hasAny returns false for an empty allowlist', () => {
      const db = freshDb();
      assert.equal(groupAllowFrom.hasAny(db, G1), false);
    });

    it('isAllowed returns true when the group has no allowlist', () => {
      // No rows means "allowlist does not apply" — gating falls back to mode.
      const db = freshDb();
      assert.equal(groupAllowFrom.isAllowed(db, G1, S_ALICE), true);
    });

    it('isAllowed returns true only for listed senders when allowlist active', () => {
      const db = freshDb();
      groupAllowFrom.add(db, G1, S_ALICE);
      assert.equal(groupAllowFrom.hasAny(db, G1), true);
      assert.equal(groupAllowFrom.isAllowed(db, G1, S_ALICE), true);
      assert.equal(groupAllowFrom.isAllowed(db, G1, S_BOB), false);
    });

    it('isAllowed returns false for empty senderJid when allowlist active', () => {
      const db = freshDb();
      groupAllowFrom.add(db, G1, S_ALICE);
      assert.equal(groupAllowFrom.isAllowed(db, G1, ''), false);
    });

    it('different groups have independent allowlists', () => {
      const db = freshDb();
      groupAllowFrom.add(db, G1, S_ALICE);
      assert.equal(groupAllowFrom.hasAny(db, G2), false);
      assert.equal(groupAllowFrom.isAllowed(db, G2, S_ALICE), true); // empty allowlist → allowed
    });
  });

  describe('add / remove / clear / list', () => {
    it('add is idempotent', () => {
      const db = freshDb();
      groupAllowFrom.add(db, G1, S_ALICE);
      groupAllowFrom.add(db, G1, S_ALICE);
      assert.deepEqual(groupAllowFrom.list(db, G1), [S_ALICE]);
    });

    it('list returns sorted sender JIDs', () => {
      const db = freshDb();
      groupAllowFrom.add(db, G1, S_BOB);
      groupAllowFrom.add(db, G1, S_ALICE);
      groupAllowFrom.add(db, G1, L_CHRIS);
      const listed = groupAllowFrom.list(db, G1);
      assert.deepEqual(listed, [...listed].sort());
      assert.equal(listed.length, 3);
    });

    it('remove deletes a single entry', () => {
      const db = freshDb();
      groupAllowFrom.add(db, G1, S_ALICE);
      groupAllowFrom.add(db, G1, S_BOB);
      groupAllowFrom.remove(db, G1, S_BOB);
      assert.deepEqual(groupAllowFrom.list(db, G1), [S_ALICE]);
    });

    it('clear removes every entry for the group', () => {
      const db = freshDb();
      groupAllowFrom.add(db, G1, S_ALICE);
      groupAllowFrom.add(db, G1, S_BOB);
      groupAllowFrom.add(db, G2, S_ALICE);
      groupAllowFrom.clear(db, G1);
      assert.deepEqual(groupAllowFrom.list(db, G1), []);
      assert.deepEqual(groupAllowFrom.list(db, G2), [S_ALICE]);
    });

    it('add rejects empty groupJid or senderJid', () => {
      const db = freshDb();
      assert.throws(() => groupAllowFrom.add(db, '', S_ALICE), /empty groupJid/);
      assert.throws(() => groupAllowFrom.add(db, G1, ''), /empty senderJid/);
    });
  });

  describe('parseCommand', () => {
    it('parses list form', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand(`/allowlist ${G1}`),
        { targetGroup: G1, action: 'list' },
      );
    });

    it('parses add with senderJid', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand(`/allowlist ${G1} add ${S_ALICE}`),
        { targetGroup: G1, action: 'add', sender: S_ALICE },
      );
    });

    it('parses remove with senderJid', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand(`/allowlist ${G1} remove ${S_ALICE}`),
        { targetGroup: G1, action: 'remove', sender: S_ALICE },
      );
    });

    it('parses clear without senderJid', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand(`/allowlist ${G1} clear`),
        { targetGroup: G1, action: 'clear' },
      );
    });

    it('requires a @g.us first arg', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand('/allowlist notagroup add foo'),
        { error: 'invalid_target', arg: 'notagroup' },
      );
    });

    it('rejects unknown action', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand(`/allowlist ${G1} whatever`),
        { error: 'invalid_action', arg: 'whatever' },
      );
    });

    it('rejects add/remove without sender', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand(`/allowlist ${G1} add`),
        { error: 'missing_sender', action: 'add' },
      );
    });

    it('requires a group argument', () => {
      assert.deepEqual(
        groupAllowFrom.parseCommand('/allowlist'),
        { error: 'missing_group' },
      );
    });

    it('returns null for non-command text', () => {
      assert.equal(groupAllowFrom.parseCommand('look at /allowlist'), null);
      assert.equal(groupAllowFrom.parseCommand(''), null);
      assert.equal(groupAllowFrom.parseCommand(null), null);
      assert.equal(groupAllowFrom.parseCommand(undefined), null);
    });
  });
});
