'use strict';

// ---------------------------------------------------------------------------
// lib/group-allow-from.js — Phase 3 completion: per-group sender allowlist.
//
// Pure functional module, same shape as lib/group-activation.js. When a
// group has at least one allowlist entry, only messages from those senders
// reach the agent; every other member's message is silently skipped by the
// gateway's gating layer.
//
// Schema:
//   CREATE TABLE group_allow_from (
//     group_jid   TEXT NOT NULL,
//     sender_jid  TEXT NOT NULL,    -- @s.whatsapp.net or @lid form
//     updated_at  INTEGER NOT NULL,
//     PRIMARY KEY (group_jid, sender_jid)
//   )
//
// Functions:
//   - init(db)                           — CREATE TABLE IF NOT EXISTS
//   - isAllowed(db, groupJid, senderJid) — true if allowlist has the sender,
//                                          OR if the group has no allowlist
//                                          at all (then the allowlist doesn't
//                                          apply and gating falls back to mode)
//   - add(db, groupJid, senderJid)       — INSERT OR REPLACE
//   - remove(db, groupJid, senderJid)    — DELETE the single row
//   - clear(db, groupJid)                — DELETE all rows for the group
//   - list(db, groupJid)                 — sender JIDs for this group
//   - hasAny(db, groupJid)               — true iff at least one entry
// ---------------------------------------------------------------------------

function init(db) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS group_allow_from (
      group_jid  TEXT NOT NULL,
      sender_jid TEXT NOT NULL,
      updated_at INTEGER NOT NULL,
      PRIMARY KEY (group_jid, sender_jid)
    );
    CREATE INDEX IF NOT EXISTS idx_group_allow_from_group_jid
      ON group_allow_from(group_jid);
  `);
}

function hasAny(db, groupJid) {
  if (!groupJid) return false;
  const row = db
    .prepare('SELECT 1 AS found FROM group_allow_from WHERE group_jid = ? LIMIT 1')
    .get(groupJid);
  return Boolean(row);
}

function isAllowed(db, groupJid, senderJid) {
  if (!groupJid) return true;
  if (!hasAny(db, groupJid)) return true; // no allowlist → allowlist doesn't apply
  if (!senderJid) return false;
  const row = db
    .prepare('SELECT 1 AS ok FROM group_allow_from WHERE group_jid = ? AND sender_jid = ? LIMIT 1')
    .get(groupJid, senderJid);
  return Boolean(row);
}

function add(db, groupJid, senderJid) {
  if (!groupJid) throw new Error('group_allow_from.add: empty groupJid');
  if (!senderJid) throw new Error('group_allow_from.add: empty senderJid');
  db.prepare(
    'INSERT OR REPLACE INTO group_allow_from (group_jid, sender_jid, updated_at) VALUES (?, ?, ?)'
  ).run(groupJid, senderJid, Date.now());
}

function remove(db, groupJid, senderJid) {
  if (!groupJid || !senderJid) return;
  db.prepare('DELETE FROM group_allow_from WHERE group_jid = ? AND sender_jid = ?')
    .run(groupJid, senderJid);
}

function clear(db, groupJid) {
  if (!groupJid) return;
  db.prepare('DELETE FROM group_allow_from WHERE group_jid = ?').run(groupJid);
}

function list(db, groupJid) {
  if (!groupJid) return [];
  return db
    .prepare('SELECT sender_jid FROM group_allow_from WHERE group_jid = ? ORDER BY sender_jid')
    .all(groupJid)
    .map((r) => r.sender_jid);
}

// Parse `/allowlist <group@g.us> <action> [senderJid]` from a message body.
// Shapes:
//   /allowlist <group@g.us>                            → { targetGroup, action: 'list' }
//   /allowlist <group@g.us> add <senderJid>            → { targetGroup, action: 'add', sender }
//   /allowlist <group@g.us> remove <senderJid>         → { targetGroup, action: 'remove', sender }
//   /allowlist <group@g.us> clear                      → { targetGroup, action: 'clear' }
//
// Requires a `@g.us` first argument so the parser can never be confused with
// an in-chat `/activation` command. Returns null when the text isn't an
// allowlist command, `{ error }` on invalid shape.
function parseCommand(text) {
  if (typeof text !== 'string') return null;
  const m = text.trim().match(/^\/allowlist(?:\s+(\S+)(?:\s+(\S+)(?:\s+(\S+))?)?)?\s*$/i);
  if (!m) return null;
  const group = m[1] || '';
  const action = (m[2] || '').toLowerCase();
  const sender = m[3] || '';

  if (!group) return { error: 'missing_group' };
  if (!/@g\.us$/i.test(group)) return { error: 'invalid_target', arg: group };
  if (!action) return { targetGroup: group, action: 'list' };

  if (action === 'clear') return { targetGroup: group, action: 'clear' };
  if (action === 'add' || action === 'remove') {
    if (!sender) return { error: 'missing_sender', action };
    return { targetGroup: group, action, sender };
  }
  return { error: 'invalid_action', arg: action };
}

module.exports = {
  init,
  hasAny,
  isAllowed,
  add,
  remove,
  clear,
  list,
  parseCommand,
};
