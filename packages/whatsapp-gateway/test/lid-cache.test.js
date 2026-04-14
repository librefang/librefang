'use strict';

// ---------------------------------------------------------------------------
// test/lid-cache.test.js — Phase 4 §B (ID-02) unit tests.
//
// All tests use an in-memory better-sqlite3 database (`:memory:`). Fixtures
// are in English — no persistent files, no module-level DB.
// ---------------------------------------------------------------------------

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');
const Database = require('better-sqlite3');

const lidCache = require('../lib/lid-cache');

const LID_ALICE = '111111111@lid';
const PN_ALICE  = '391230000001@s.whatsapp.net';
const LID_BOB   = '222222222@lid';
const PN_BOB    = '391230000002@s.whatsapp.net';
const LID_BOSS  = '333333333@lid';
const PN_BOSS   = '391230000003@s.whatsapp.net';

function freshDb() {
  const db = new Database(':memory:');
  lidCache.init(db);
  return db;
}

describe('lid-cache', () => {
  describe('init', () => {
    it('creates the lid_cache table on a blank DB', () => {
      const db = new Database(':memory:');
      lidCache.init(db);
      const row = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='lid_cache'")
        .get();
      assert.equal(row?.name, 'lid_cache');
    });

    it('is idempotent — running twice does not error', () => {
      const db = new Database(':memory:');
      lidCache.init(db);
      assert.doesNotThrow(() => lidCache.init(db));
      // And a third time for good measure.
      assert.doesNotThrow(() => lidCache.init(db));
    });

    it('preserves existing rows across re-init', () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      lidCache.init(db);
      const map = lidCache.loadAll(db);
      assert.equal(map.get(LID_ALICE), PN_ALICE);
    });
  });

  describe('upsert + loadAll', () => {
    it('stores a single mapping and reads it back', () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      const map = lidCache.loadAll(db);
      assert.equal(map.size, 1);
      assert.equal(map.get(LID_ALICE), PN_ALICE);
    });

    it('stores multiple mappings', () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      lidCache.upsert(db, LID_BOB, PN_BOB);
      lidCache.upsert(db, LID_BOSS, PN_BOSS);
      const map = lidCache.loadAll(db);
      assert.equal(map.size, 3);
      assert.equal(map.get(LID_ALICE), PN_ALICE);
      assert.equal(map.get(LID_BOB), PN_BOB);
      assert.equal(map.get(LID_BOSS), PN_BOSS);
    });

    it('ignores empty lid or empty pn_jid', () => {
      const db = freshDb();
      lidCache.upsert(db, '', PN_ALICE);
      lidCache.upsert(db, LID_ALICE, '');
      lidCache.upsert(db, null, null);
      const map = lidCache.loadAll(db);
      assert.equal(map.size, 0);
    });

    it('loadAll on an empty table returns an empty Map', () => {
      const db = freshDb();
      const map = lidCache.loadAll(db);
      assert.ok(map instanceof Map);
      assert.equal(map.size, 0);
    });
  });

  describe('INSERT OR REPLACE semantics', () => {
    it('overwrites pn_jid when the same lid is upserted again', () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      // Alice changes her phone number (or the mapping gets corrected).
      lidCache.upsert(db, LID_ALICE, PN_BOB);
      const map = lidCache.loadAll(db);
      assert.equal(map.size, 1);
      assert.equal(map.get(LID_ALICE), PN_BOB);
    });

    it('bumps updated_at on replace', async () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      const firstTs = db
        .prepare('SELECT updated_at FROM lid_cache WHERE lid = ?')
        .get(LID_ALICE).updated_at;

      // Wait past the millisecond boundary.
      await new Promise((r) => setTimeout(r, 5));

      lidCache.upsert(db, LID_ALICE, PN_BOB);
      const secondTs = db
        .prepare('SELECT updated_at FROM lid_cache WHERE lid = ?')
        .get(LID_ALICE).updated_at;

      assert.ok(secondTs > firstTs, `expected ${secondTs} > ${firstTs}`);
    });
  });

  describe('prune', () => {
    it('keeps the N most-recently-updated rows and deletes the rest', () => {
      const db = freshDb();
      // Insert 20 rows with manually-controlled updated_at so the ordering is
      // deterministic regardless of how fast the loop runs.
      const stmt = db.prepare(
        'INSERT OR REPLACE INTO lid_cache (lid, pn_jid, updated_at) VALUES (?, ?, ?)'
      );
      for (let i = 0; i < 20; i++) {
        stmt.run(`${i}@lid`, `${i}@s.whatsapp.net`, 1_700_000_000_000 + i);
      }
      assert.equal(lidCache.count(db), 20);

      lidCache.prune(db, 5);
      assert.equal(lidCache.count(db), 5);

      // The 5 newest (i = 15..19) must survive.
      const map = lidCache.loadAll(db);
      for (let i = 15; i < 20; i++) {
        assert.equal(map.get(`${i}@lid`), `${i}@s.whatsapp.net`);
      }
      for (let i = 0; i < 15; i++) {
        assert.equal(map.has(`${i}@lid`), false);
      }
    });

    it('is a no-op when the table already fits within keep', () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      lidCache.upsert(db, LID_BOB, PN_BOB);
      lidCache.prune(db, 10);
      assert.equal(lidCache.count(db), 2);
    });

    it('defaults keep to 10000 when called without the argument', () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      // Should not throw and should not delete anything.
      lidCache.prune(db);
      assert.equal(lidCache.count(db), 1);
      assert.equal(lidCache.DEFAULT_KEEP, 10000);
    });

    it('keep = 0 empties the table', () => {
      const db = freshDb();
      lidCache.upsert(db, LID_ALICE, PN_ALICE);
      lidCache.upsert(db, LID_BOB, PN_BOB);
      lidCache.prune(db, 0);
      assert.equal(lidCache.count(db), 0);
    });
  });

  describe('cross-restart simulation', () => {
    // better-sqlite3 `:memory:` databases are per-connection, so a true
    // "close + reopen" test requires a real file. We use a tmp path + unlink.
    it('survives close + reopen when backed by a real file', () => {
      const path = require('node:path');
      const fs = require('node:fs');
      const os = require('node:os');
      const tmp = path.join(
        os.tmpdir(),
        `lid-cache-test-${process.pid}-${Date.now()}.db`
      );

      try {
        // --- Session 1: write ---
        const db1 = new Database(tmp);
        lidCache.init(db1);
        lidCache.upsert(db1, LID_ALICE, PN_ALICE);
        lidCache.upsert(db1, LID_BOB, PN_BOB);
        lidCache.upsert(db1, LID_BOSS, PN_BOSS);
        db1.close();

        // --- Session 2: reopen, loadAll ---
        const db2 = new Database(tmp);
        lidCache.init(db2); // idempotent — simulates boot-time init
        const map = lidCache.loadAll(db2);
        db2.close();

        assert.equal(map.size, 3);
        assert.equal(map.get(LID_ALICE), PN_ALICE);
        assert.equal(map.get(LID_BOB), PN_BOB);
        assert.equal(map.get(LID_BOSS), PN_BOSS);
      } finally {
        try { fs.unlinkSync(tmp); } catch (_) { /* best-effort */ }
      }
    });
  });
});
