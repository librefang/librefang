'use strict';

// ---------------------------------------------------------------------------
// lib/lid-cache.js — Phase 4 §B (ID-02): persisted LID → phone-number JID cache.
//
// Pure functional module. Takes a `better-sqlite3` Database handle as input —
// no hidden state, no singleton DB, no module-level prepared statements. The
// caller (index.js) owns the DB lifecycle and the in-memory `lidToPnJid` Map;
// this module only handles the SQL layer.
//
// Schema:
//   CREATE TABLE lid_cache (
//     lid         TEXT PRIMARY KEY,           -- '<digits>@lid'
//     pn_jid      TEXT NOT NULL,              -- '<digits>@s.whatsapp.net'
//     updated_at  INTEGER NOT NULL            -- unix ms, for LRU-style pruning
//   )
//
// Strategy:
//   - `init(db)`                   — idempotent CREATE TABLE IF NOT EXISTS.
//   - `loadAll(db)`                — read every row, return Map<lid, pn_jid>.
//   - `upsert(db, lid, pnJid)`     — INSERT OR REPLACE; bumps updated_at.
//   - `prune(db, keep)`            — keep the `keep` most-recently-updated rows
//                                    (default 10000); delete the rest.
//
// All functions throw on SQL errors; the caller decides whether to swallow
// the error (ID-02 §Concerns: write failures are non-blocking via try/catch
// at the call site, logged as `lid_cache_write_failed`).
// ---------------------------------------------------------------------------

const DEFAULT_KEEP = 10000;

function init(db) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS lid_cache (
      lid        TEXT PRIMARY KEY,
      pn_jid     TEXT NOT NULL,
      updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_lid_cache_updated_at
      ON lid_cache(updated_at);
  `);
}

function loadAll(db) {
  const rows = db.prepare('SELECT lid, pn_jid FROM lid_cache').all();
  const map = new Map();
  for (const r of rows) {
    if (r && r.lid && r.pn_jid) map.set(r.lid, r.pn_jid);
  }
  return map;
}

function upsert(db, lid, pnJid) {
  if (!lid || !pnJid) return;
  db.prepare(
    'INSERT OR REPLACE INTO lid_cache (lid, pn_jid, updated_at) VALUES (?, ?, ?)'
  ).run(lid, pnJid, Date.now());
}

function prune(db, keep) {
  const k = typeof keep === 'number' && keep >= 0 ? keep : DEFAULT_KEEP;
  // Keep top-K most recently updated, delete the rest. Single statement so
  // better-sqlite3 runs it atomically.
  db.prepare(`
    DELETE FROM lid_cache
    WHERE lid NOT IN (
      SELECT lid FROM lid_cache
      ORDER BY updated_at DESC
      LIMIT ?
    )
  `).run(k);
}

function count(db) {
  const row = db.prepare('SELECT COUNT(*) AS c FROM lid_cache').get();
  return row ? row.c : 0;
}

module.exports = {
  init,
  loadAll,
  upsert,
  prune,
  count,
  DEFAULT_KEEP,
};
