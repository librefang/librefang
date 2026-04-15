'use strict';

// ---------------------------------------------------------------------------
// lib/identity.js — Phase 4 §A (ID-01): centralized JID/LID/E.164 normalization.
//
// Pure functional module. No side effects, no hidden state, no SQLite, no
// network. All caches are passed in as parameters (the gateway owns the
// lidToPnJid Map; Plan 02 will back that Map with SQLite persistence).
//
// WhatsApp JID shapes we care about:
//   - '<digits>@s.whatsapp.net'            — standard phone-number JID
//   - '<digits>:<device>@s.whatsapp.net'   — device-scoped multi-device JID
//   - '<digits>@lid'                       — WhatsApp anonymous LID (opaque)
//   - '<digits>@hosted.lid'                — hosted LID (Baileys docs; guard)
//   - '<digits>-<digits>@g.us'             — group JID
// ---------------------------------------------------------------------------

const LID_SUFFIX_RE = /@(lid|hosted\.lid)$/;
const GROUP_SUFFIX_RE = /@g\.us$/;
const DEVICE_SUFFIX_RE = /:(\d+)@/;
const E164_JID_RE = /^(\d+)@s\.whatsapp\.net$/;

function isLidJid(jid) {
  if (!jid || typeof jid !== 'string') return false;
  return LID_SUFFIX_RE.test(jid);
}

function isGroupJid(jid) {
  if (!jid || typeof jid !== 'string') return false;
  return GROUP_SUFFIX_RE.test(jid);
}

// Strip device-suffix from JIDs like '123:45@s.whatsapp.net' -> '123@s.whatsapp.net'.
// Groups are returned unchanged (the '-' in a group JID is not a device suffix).
function normalizeDeviceScopedJid(jid) {
  if (!jid || typeof jid !== 'string') return jid || '';
  if (isGroupJid(jid)) return jid;
  return jid.replace(DEVICE_SUFFIX_RE, '@');
}

// Returns '+<digits>' for phone JIDs, empty string otherwise.
// Device-suffix is stripped first so '123:45@s.whatsapp.net' -> '+123'.
function extractE164(jid) {
  if (!jid || typeof jid !== 'string') return '';
  const normalized = normalizeDeviceScopedJid(jid);
  const m = E164_JID_RE.exec(normalized);
  return m ? '+' + m[1] : '';
}

// Accepts '+391234', '391234', '123@s.whatsapp.net', '123-456@g.us'.
// Returns a sendable Baileys JID. Group JIDs and existing JIDs passthrough.
function phoneToJid(phoneOrJid) {
  if (!phoneOrJid || typeof phoneOrJid !== 'string') return '';
  if (isGroupJid(phoneOrJid)) return phoneOrJid;
  if (phoneOrJid.includes('@')) return phoneOrJid;
  return phoneOrJid.replace(/^\+/, '') + '@s.whatsapp.net';
}

// Owner JIDs derived from a list of '+E.164' numbers.
function deriveOwnerJids(ownerNumbers) {
  const out = new Set();
  if (!Array.isArray(ownerNumbers)) return out;
  for (const n of ownerNumbers) {
    if (!n || typeof n !== 'string') continue;
    out.add(n.replace(/^\+/, '') + '@s.whatsapp.net');
  }
  return out;
}

// resolvePeerId — 5-step heuristic from CONTEXT §A Specifics.
// Caller still owns side effects (cache writes, proactive lookups). This
// function is pure.
//
// Heuristic order (locked):
//   1. senderPn present          -> { peer: senderPn,           confidence: 'direct' }
//   2. isGroupJid(jid)           -> { peer: jid,                confidence: 'group'  }
//   3. isLidJid && cache.has(jid)-> { peer: cache.get(jid),     confidence: 'cache'  }
//   4. !isLidJid && !isGroupJid  -> { peer: normalizeDevice(jid),confidence: 'direct' }
//   5. participant && !isLidJid  -> { peer: normalize(participant), confidence: 'participant' }
//   6. otherwise                 -> { peer: '',                 confidence: 'lid_unresolved' }
function resolvePeerId(jid, opts) {
  const options = opts || {};
  const senderPn = options.senderPn || '';
  const participant = options.participant || '';
  const cache = options.lidToPnCache || null;

  if (senderPn) {
    return { peer: senderPn, confidence: 'direct' };
  }
  if (isGroupJid(jid)) {
    return { peer: jid, confidence: 'group' };
  }
  if (isLidJid(jid) && cache && typeof cache.has === 'function' && cache.has(jid)) {
    return { peer: cache.get(jid), confidence: 'cache' };
  }
  if (jid && !isLidJid(jid) && !isGroupJid(jid)) {
    return { peer: normalizeDeviceScopedJid(jid), confidence: 'direct' };
  }
  if (participant && !isLidJid(participant)) {
    return { peer: normalizeDeviceScopedJid(participant), confidence: 'participant' };
  }
  return { peer: '', confidence: 'lid_unresolved' };
}

module.exports = {
  isLidJid,
  isGroupJid,
  normalizeDeviceScopedJid,
  extractE164,
  phoneToJid,
  resolvePeerId,
  deriveOwnerJids,
};
