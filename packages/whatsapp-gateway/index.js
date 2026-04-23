#!/usr/bin/env node
'use strict';

const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { randomUUID } = require('node:crypto');
const toml = require('toml');
const { EchoTracker } = require('./lib/echo-tracker');
const lidCache = require('./lib/lid-cache');
const { createDedupTracker } = require('./lib/dedup-tracker');
const {
  isLidJid,
  isGroupJid,
  normalizeDeviceScopedJid,
  extractE164,
  phoneToJid,
  resolvePeerId,
  deriveOwnerJids,
} = require('./lib/identity');
const { buildSessionKey, channelTypeForChat } = require('./lib/session-key');

// ---------------------------------------------------------------------------
// Persisted LID cache (ID-02, Phase 4 §B)
// ---------------------------------------------------------------------------
// The in-memory `lidToPnJid` Map is populated on every senderPn observation
// and every successful `resolveLidProactively` call. To survive restarts, we
// mirror every insertion into the SQLite `lid_cache` table (init'd below,
// loaded into the Map at boot). Failures are logged as
// `lid_cache_write_failed` and never block the caller.
// Flag `LIBREFANG_LID_PERSIST=off` disables persistence (in-memory only) —
// useful for ephemeral CI runs or debugging with a fresh map each boot.
const LID_PERSIST_ENABLED = process.env.LIBREFANG_LID_PERSIST !== 'off';

// ---------------------------------------------------------------------------
// Echo tracker (EB-01, Phase 3 §A)
// ---------------------------------------------------------------------------
// Process-local LRU that records every outbound text sent via
// `sock.sendMessage({ text })`. On inbound `messages.upsert` we consult
// `echoTracker.isEcho(...)` and drop the self-loop echo before forwarding to
// librefang. Flag `LIBREFANG_ECHO_TRACKER=off` disables end-to-end (no-op).
const ECHO_TRACKER_ENABLED = process.env.LIBREFANG_ECHO_TRACKER !== 'off';
const echoTracker = new EchoTracker(100);

// Phase 3 §B (EB-02) — gate the `forward_dispatch` structured log.
// Default ON ('verbose'); set LIBREFANG_DISPATCH_LOG to any other value
// (e.g. 'off') to silence the diagnostic line without redeploy.
const DISPATCH_LOG_VERBOSE = (process.env.LIBREFANG_DISPATCH_LOG || 'verbose') === 'verbose';

// Phase 3 §B — CS-01 regression guard. Runs once at boot (and is exported
// for unit tests). Two distinct chatJids must yield two distinct
// channel_type strings; otherwise the gateway-to-kernel per-conversation
// isolation contract is broken and we refuse to boot.
function runDispatchSelfTest(channelTypeFn) {
  const fn = channelTypeFn || channelTypeForChat;
  const a = fn('111@s.whatsapp.net');
  const b = fn('222@s.whatsapp.net');
  if (a === b || !a.startsWith('whatsapp:') || !b.startsWith('whatsapp:')) {
    return { ok: false, reason: `channel_type regression: a=${a} b=${b}` };
  }
  return { ok: true };
}

// ---------------------------------------------------------------------------
// SQLite Message Store (better-sqlite3)
// ---------------------------------------------------------------------------
const Database = require('better-sqlite3');
const DB_PATH = process.env.WHATSAPP_DB_PATH || path.join(__dirname, 'messages.db');

const db = new Database(DB_PATH);
db.pragma('journal_mode = WAL');
db.pragma('busy_timeout = 5000');

// Set file permissions to 600 (owner read/write only)
fs.chmodSync(DB_PATH, 0o600);

// Schema
db.exec(`
  CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    jid TEXT NOT NULL,
    sender_jid TEXT,
    push_name TEXT,
    phone TEXT,
    text TEXT,
    direction TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    processed INTEGER DEFAULT 0,
    retry_count INTEGER DEFAULT 0,
    raw_type TEXT,
    created_at TEXT DEFAULT (datetime('now'))
  );
  CREATE INDEX IF NOT EXISTS idx_messages_jid_ts ON messages(jid, timestamp);
  CREATE INDEX IF NOT EXISTS idx_messages_processed ON messages(processed);
`);

// Track last-seen timestamp per JID (for gap detection — Fase 3.2 Option C)
db.exec(`
  CREATE TABLE IF NOT EXISTS jid_last_seen (
    jid TEXT PRIMARY KEY,
    last_timestamp INTEGER NOT NULL,
    updated_at TEXT DEFAULT (datetime('now'))
  );
`);

// Phase 4 §B (ID-02): persisted LID → phone-number JID cache.
if (LID_PERSIST_ENABLED) {
  try {
    lidCache.init(db);
  } catch (err) {
    console.warn(JSON.stringify({
      event: 'lid_cache_init_failed',
      error: err.message,
    }));
  }
}

console.log(`[gateway] SQLite message store initialized: ${DB_PATH}`);

// --- Prepared statements (reusable, faster) ---
const stmtInsertMsg = db.prepare(`
  INSERT OR IGNORE INTO messages (id, jid, sender_jid, push_name, phone, text, direction, timestamp, processed, raw_type)
  VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
`);

const stmtMarkProcessed = db.prepare(`
  UPDATE messages SET processed = ? WHERE id = ?
`);

const stmtIncrRetry = db.prepare(`
  UPDATE messages SET retry_count = retry_count + 1 WHERE id = ?
`);

const stmtMarkFailed = db.prepare(`
  UPDATE messages SET processed = -1 WHERE id = ?
`);

const stmtGetByJid = db.prepare(`
  SELECT id, jid, sender_jid, push_name, phone, text, direction, timestamp, processed, raw_type
  FROM messages WHERE jid = ? AND timestamp >= ? ORDER BY timestamp DESC LIMIT ?
`);

const stmtGetUnprocessed = db.prepare(`
  SELECT id, jid, sender_jid, push_name, phone, text, direction, timestamp, retry_count, raw_type
  FROM messages WHERE processed = 0 AND timestamp < ? ORDER BY timestamp ASC
`);

const stmtCleanupOld = db.prepare(`
  DELETE FROM messages WHERE timestamp < ? AND processed IN (1, -1)
`);

const stmtUpsertLastSeen = db.prepare(`
  INSERT INTO jid_last_seen (jid, last_timestamp, updated_at)
  VALUES (?, ?, datetime('now'))
  ON CONFLICT(jid) DO UPDATE SET last_timestamp = excluded.last_timestamp, updated_at = datetime('now')
`);

const stmtGetLastSeen = db.prepare(`
  SELECT jid, last_timestamp FROM jid_last_seen
`);

/**
 * Save a message to the SQLite store.
 */
function dbSaveMessage({ id, jid, senderJid, pushName, phone, text, direction, timestamp, processed, rawType }) {
  try {
    stmtInsertMsg.run(id, jid, senderJid || null, pushName || null, phone || null, text || null, direction, timestamp, processed || 0, rawType || null);
  } catch (err) {
    console.error(`[gateway][db] Failed to save message ${id}: ${err.message}`);
  }
}

/**
 * Mark a message as processed (1) or failed (-1).
 */
// CS-01 iter 2: a catchup journal row with null/empty jid is orphan — cannot
// be scoped to any WhatsApp chat. Pure predicate so tests can exercise it
// without spinning up the full catchup loop / socket / DB.
function shouldSkipCatchupForMissingJid(msg) {
  return !msg || !msg.jid;
}

function dbMarkProcessed(msgId, status) {
  try {
    stmtMarkProcessed.run(status, msgId);
  } catch (err) {
    console.error(`[gateway][db] Failed to mark message ${msgId}: ${err.message}`);
  }
}

/**
 * Get messages for a JID, optionally filtered by since timestamp.
 */
function dbGetMessagesByJid(jid, limit = 20, since = 0) {
  return stmtGetByJid.all(jid, since, limit);
}

/**
 * Get all unprocessed messages older than a threshold (epoch ms).
 */
function dbGetUnprocessed(olderThan) {
  return stmtGetUnprocessed.all(olderThan);
}

/**
 * Increment retry count for a message. If retry_count >= maxRetries, mark as permanently failed.
 */
function dbIncrRetryOrFail(msgId, maxRetries = 3) {
  const msg = db.prepare('SELECT retry_count FROM messages WHERE id = ?').get(msgId);
  if (!msg) return;
  if (msg.retry_count + 1 >= maxRetries) {
    stmtMarkFailed.run(msgId);
    console.warn(`[gateway][db] Message ${msgId} permanently failed after ${maxRetries} retries`);
  } else {
    stmtIncrRetry.run(msgId);
  }
}

/**
 * Delete old processed/failed messages.
 */
function dbCleanupOld(olderThanMs) {
  const result = stmtCleanupOld.run(olderThanMs);
  return result.changes;
}

/**
 * Update last-seen timestamp for a JID.
 */
function dbUpdateLastSeen(jid, timestamp) {
  try {
    stmtUpsertLastSeen.run(jid, timestamp);
  } catch (err) {
    console.error(`[gateway][db] Failed to update last_seen for ${jid}: ${err.message}`);
  }
}

// ---------------------------------------------------------------------------
// Read config.toml — the gateway reads its own config directly
// ---------------------------------------------------------------------------
const CONFIG_PATH = process.env.LIBREFANG_CONFIG || path.join(os.homedir(), '.librefang', 'config.toml');

function readWhatsAppConfig(configPath) {
  const defaults = {
    default_agent: 'assistant',
    owner_numbers: [],
    conversation_ttl_hours: 24,
    // English-only by default keeps upstream deployments locale-neutral;
    // set `[relay_intent].languages = ["en", "it", …]` in config.toml
    // to enable extra language packs.
    relay_intent_languages: ['en'],
  };
  try {
    const content = fs.readFileSync(configPath, 'utf8');
    const parsed = toml.parse(content);
    const wa = parsed?.channels?.whatsapp || {};
    const relay = parsed?.relay_intent || {};
    const cfg = {
      default_agent: wa.default_agent || defaults.default_agent,
      owner_numbers: Array.isArray(wa.owner_numbers) ? wa.owner_numbers : defaults.owner_numbers,
      conversation_ttl_hours: parseInt(wa.conversation_ttl_hours, 10) || defaults.conversation_ttl_hours,
      relay_intent_languages:
        Array.isArray(relay.languages) && relay.languages.length > 0
          ? relay.languages
          : defaults.relay_intent_languages,
    };
    console.log(`[gateway] Read config from ${configPath}: default_agent="${cfg.default_agent}", owner_numbers=${JSON.stringify(cfg.owner_numbers)}, conversation_ttl_hours=${cfg.conversation_ttl_hours}, relay_intent_languages=${JSON.stringify(cfg.relay_intent_languages)}`);
    return cfg;
  } catch (err) {
    console.warn(`[gateway] Could not read ${configPath}: ${err.message} — using defaults/env vars`);
    return defaults;
  }
}

const tomlConfig = readWhatsAppConfig(CONFIG_PATH);

// ---------------------------------------------------------------------------
// Config: config.toml is the source of truth, env vars override if set
// ---------------------------------------------------------------------------
const PORT = parseInt(process.env.WHATSAPP_GATEWAY_PORT || '3009', 10);
const LIBREFANG_URL = (process.env.LIBREFANG_URL || 'http://127.0.0.1:4545').replace(/\/+$/, '');
const DEFAULT_AGENT = process.env.LIBREFANG_DEFAULT_AGENT || tomlConfig.default_agent;
const AGENT_NAME = DEFAULT_AGENT;

// Owner routing: build OWNER_JIDs set from config.toml owner_numbers
const ownerNumbersFromEnv = process.env.WHATSAPP_OWNER_JID ? [process.env.WHATSAPP_OWNER_JID] : [];
const OWNER_NUMBERS = ownerNumbersFromEnv.length > 0 ? ownerNumbersFromEnv : tomlConfig.owner_numbers;
const OWNER_JIDS = deriveOwnerJids(OWNER_NUMBERS);
// Primary owner JID for unsolicited/scheduled messages only
const OWNER_JID = OWNER_JIDS.size > 0 ? [...OWNER_JIDS][0] : '';

// §A — Feature flag: when set to "off" the gateway ignores the typed
// owner_notice channel introduced by the notify_owner LLM tool and falls
// back to the legacy NOTIFY_OWNER text-tag path. Lets ops roll back the
// new behaviour without a rebuild. Defaults to "on".
const OWNER_CHANNEL_MODE = (process.env.LIBREFANG_OWNER_CHANNEL || 'on').toLowerCase();
const OWNER_CHANNEL_ENABLED = OWNER_CHANNEL_MODE !== 'off';

// Conversation TTL from config.toml (default 24 hours)
const CONVERSATION_TTL_HOURS = parseInt(process.env.CONVERSATION_TTL_HOURS || String(tomlConfig.conversation_ttl_hours), 10);
const CONVERSATION_TTL_MS = CONVERSATION_TTL_HOURS * 3600 * 1000;

// Validate owner numbers at startup
if (OWNER_NUMBERS.length > 0) {
  for (const num of OWNER_NUMBERS) {
    const digits = num.replace(/^\+/, '');
    if (!/^\d{7,15}$/.test(digits)) {
      console.error(`[gateway] WARNING: owner number "${num}" looks invalid (expected 7-15 digits). Owner routing may not work.`);
    }
  }
  console.log(`[gateway] Owner routing enabled → ${[...OWNER_JIDS].join(', ')}`);
} else {
  console.log('[gateway] Owner routing disabled (no owner_numbers configured)');
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
let sock = null;          // Baileys socket
let sessionId = '';       // current session identifier
let qrDataUrl = '';       // latest QR code as data:image/png;base64,...
let connStatus = 'disconnected'; // disconnected | qr_ready | connected
let qrExpired = false;
let statusMessage = 'Not started';
let reconnectAttempts = 0;
let isConnecting = false;
// ST-02: legacy reconnect-delay/attempt constants removed in favour of the
// jittered backoff (computeBackoffDelay) with a 30s cap and no hard stop.

// ST-01 heartbeat watchdog: if no inbound messages.upsert event arrives for
// HEARTBEAT_MS, force-close the socket so the existing reconnect path takes
// over. The 180s default matches the openclaw reference.
const HEARTBEAT_MS = parseInt(process.env.WA_HEARTBEAT_MS || '180000', 10);
const HEARTBEAT_CHECK_INTERVAL_MS = parseInt(process.env.WA_HEARTBEAT_CHECK_MS || '30000', 10);
let lastInboundAt = Date.now();
let heartbeatInterval = null;

// Pure predicate — true when we've been silent longer than thresholdMs.
function checkHeartbeat(now, lastInboundAt, thresholdMs) {
  return (now - lastInboundAt) > thresholdMs;
}

// ST-02: exponential backoff with ±25% jitter, cap 30s, factor 1.8, NO hard
// stop. `rng` is injected for deterministic tests (defaults to Math.random).
// Matches openclaw/extensions/whatsapp/src/reconnect.ts semantics.
function computeBackoffDelay(attempts, rng = Math.random) {
  const base = Math.min(2000 * Math.pow(1.8, Math.max(0, attempts - 1)), 30_000);
  const jitter = 0.75 + rng() * 0.5; // ±25%
  return Math.round(base * jitter);
}

// Cached agent UUID — resolved from DEFAULT_AGENT name on first use
let cachedAgentId = null;

// The user's own JID (set after connection opens) for self-chat detection
let ownJid = null;

// ---------------------------------------------------------------------------
// LID ↔ phone-number JID mapping
// ---------------------------------------------------------------------------
// WhatsApp assigns every account an opaque `<digits>@lid` identifier that is
// unrelated to the phone number. The `remoteJid` of an inbound message may be
// a LID rather than an `@s.whatsapp.net` JID, and in that case we can only
// recognise the sender (owner vs. stranger, routing key, logging) once we have
// resolved the LID back to a phone-number JID.
//
// We maintain two caches:
//   - `lidToPnJid`    — populated from `msg.key.senderPn` whenever a message
//                       arrives that carries both the LID and the real PN JID.
//   - `ownerLidJids`  — LIDs known to belong to an OWNER_NUMBERS entry. We
//                       resolve these once at connect via `sock.onWhatsApp()`
//                       so that the very first LID-addressed message from the
//                       owner is recognised, even before any senderPn event.
const lidToPnJid = new Map();    // '<digits>@lid' → '<digits>@s.whatsapp.net'
const ownerLidJids = new Set();  // '<digits>@lid'

// Phase 4 §B (ID-02): boot-time rehydrate from SQLite. Keeps the 10000 most
// recently updated entries (prune runs before load so the eviction budget is
// enforced immediately, independent of how large the on-disk table grew
// between restarts).
if (LID_PERSIST_ENABLED) {
  try {
    lidCache.prune(db, lidCache.DEFAULT_KEEP);
    const persisted = lidCache.loadAll(db);
    for (const [lid, pn] of persisted) lidToPnJid.set(lid, pn);
    if (persisted.size > 0) {
      console.log(`[gateway] LID cache hydrated from SQLite: ${persisted.size} entries`);
    }
  } catch (err) {
    console.warn(JSON.stringify({
      event: 'lid_cache_load_failed',
      error: err.message,
    }));
  }
}

// Write-through helper. Updates the in-memory Map first (authoritative for
// the hot path) then mirrors to SQLite best-effort. SQL failures are logged
// but NEVER thrown — identity resolution must keep working even if the DB
// becomes read-only mid-session.
function lidMapSet(lid, pnJid) {
  if (!lid || !pnJid) return;
  lidToPnJid.set(lid, pnJid);
  if (!LID_PERSIST_ENABLED) return;
  try {
    lidCache.upsert(db, lid, pnJid);
  } catch (err) {
    console.warn(JSON.stringify({
      event: 'lid_cache_write_failed',
      lid,
      error: err.message,
    }));
  }
}

// ---------------------------------------------------------------------------
// Phase 2 §C — Group participant roster cache (GS-01 minimal)
// ---------------------------------------------------------------------------
// Populated lazily by `getGroupParticipants(sock, groupJid)` on first inbound
// from a group; subsequent inbounds within GROUP_METADATA_TTL_MS reuse the
// cached roster (no Baileys network call). The Baileys
// `group-participants.update` event invalidates the matching entry so adds /
// removes / promotions become visible at the next message.
//
// The roster is forwarded to the kernel inside the inbound payload's
// `sender_context.metadata.group_participants` field; librefang-channels'
// `should_process_group_message` consults it to decide whether the turn is
// addressed to the bot or to another participant (OB-04, OB-05).
const GROUP_METADATA_TTL_MS = 5 * 60 * 1000;
const groupMetadataCache = new Map(); // groupJid -> { participants: [...], fetchedAt }

async function getGroupParticipants(sock, groupJid) {
  if (!isGroupJid(groupJid)) return [];
  const cached = groupMetadataCache.get(groupJid);
  if (cached && (Date.now() - cached.fetchedAt) < GROUP_METADATA_TTL_MS) {
    console.log(JSON.stringify({ event: 'group_roster_cache_hit', groupJid, size: cached.participants.length }));
    return cached.participants;
  }
  try {
    const meta = await sock.groupMetadata(groupJid);
    const participants = (meta && Array.isArray(meta.participants) ? meta.participants : []).map(p => ({
      jid: p.id || p.jid || '',
      display_name: p.notify || p.name || (p.id ? String(p.id).split('@')[0] : ''),
    }));
    groupMetadataCache.set(groupJid, { participants, fetchedAt: Date.now() });
    console.log(JSON.stringify({ event: 'group_roster_fetched', groupJid, size: participants.length }));
    return participants;
  } catch (err) {
    console.log(JSON.stringify({ event: 'group_roster_fetch_failed', groupJid, error: String(err && err.message || err) }));
    return [];
  }
}

function invalidateGroupRoster(groupJid) {
  if (!groupJid) return;
  if (groupMetadataCache.delete(groupJid)) {
    console.log(JSON.stringify({ event: 'group_roster_invalidated', groupJid }));
  }
}

// CS-02: proactive LID→PN resolution for first-seen LIDs. Races
// sock.onWhatsApp([lid]) against a timeout; on success, populates the cache
// so subsequent messages in the same burst find it synchronously. On
// timeout or empty response, falls back to degraded-but-no-block behaviour
// (the caller proceeds with the LID as-is; a later senderPn event may
// still populate the cache naturally).
//
// Returns a string tag for observability: 'resolved' | 'empty' | 'timeout'
// | 'skipped' | 'error'. Side-effect: writes to `cache` on 'resolved'.
async function resolveLidProactively(sock, lid, cache, timeoutMs = 5000) {
  if (!sock || !lid || !cache || typeof sock.onWhatsApp !== 'function') return 'skipped';
  if (cache.has(lid)) return 'skipped';
  let timer;
  try {
    const lookup = await Promise.race([
      Promise.resolve(sock.onWhatsApp([lid])),
      new Promise((_, r) => { timer = setTimeout(() => r(new Error('timeout')), timeoutMs); }),
    ]);
    if (Array.isArray(lookup) && lookup[0] && lookup[0].jid) {
      cache.set(lid, lookup[0].jid);
      console.log(`[gateway] lid_resolved lid=${lid} pn=${lookup[0].jid}`);
      return 'resolved';
    }
    console.warn(`[gateway] lid_resolve_empty lid=${lid}`);
    return 'empty';
  } catch (e) {
    if (e && e.message === 'timeout') {
      console.warn(`[gateway] lid_resolve_timeout lid=${lid}`);
      return 'timeout';
    }
    console.warn(`[gateway] lid_resolve_error lid=${lid} err=${e && e.message}`);
    return 'error';
  } finally {
    if (timer) clearTimeout(timer);
  }
}

// ---------------------------------------------------------------------------
// Message store for Baileys retry mechanism
// ---------------------------------------------------------------------------
// Baileys needs getMessage() to re-decrypt messages on retry.  We keep a
// bounded in-memory store of recently received raw messages.
const MESSAGE_STORE_MAX = 500;
const MESSAGE_STORE_TTL_MS = 10 * 60 * 1000; // 10 min
const messageStore = new Map(); // key: msgId → { message, ts }

function messageStoreSet(msgId, message) {
  if (!msgId || !message) return;
  messageStore.set(msgId, { message, ts: Date.now() });
  // Evict oldest entries if over limit
  if (messageStore.size > MESSAGE_STORE_MAX) {
    const oldest = messageStore.keys().next().value;
    messageStore.delete(oldest);
  }
}

function messageStoreGet(msgId) {
  const entry = messageStore.get(msgId);
  if (!entry) return undefined;
  if (Date.now() - entry.ts > MESSAGE_STORE_TTL_MS) {
    messageStore.delete(msgId);
    return undefined;
  }
  return entry.message;
}

// ---------------------------------------------------------------------------
// Decryption retry tracking & fallback notification
// ---------------------------------------------------------------------------
const DECRYPT_RETRY_MAX = 3;
const DECRYPT_RETRY_EXPIRE_MS = 5 * 60 * 1000; // 5 min
const decryptRetryMap = new Map(); // key: "jid:msgId" → { count, expireTimer, firstSeen }

function getDecryptRetryKey(jid, msgId) { return `${jid}:${msgId}`; }

function cleanupDecryptRetry(key) {
  const entry = decryptRetryMap.get(key);
  if (entry?.expireTimer) clearTimeout(entry.expireTimer);
  decryptRetryMap.delete(key);
}

// ---------------------------------------------------------------------------
// Signal session renegotiation tracking (upsert path)
//
// The existing messages.update stub-39 hook handles decryption failures that
// Baileys surfaces via the update channel. A different class of failures —
// libsignal throwing from session_cipher.js before any stub is emitted —
// never reaches that hook. Those messages arrive in messages.upsert with
// msg.message = null/undefined and are silently skipped. The fix is to
// detect the null-content case in upsert and force a fresh Signal session
// via assertSessions(..., true) so the peer re-keys on their next send.
//   baseJid → { attempts, lastAttemptAt, lastMsgId, notified }
const sessionRecoveryMap = new Map();
const SESSION_RECOVERY_COOLDOWN_MS = 20_000;
const SESSION_RECOVERY_MAX_ATTEMPTS = 3;
const SESSION_RECOVERY_EXPIRE_MS = 30 * 60 * 1000; // 30 min

function normalizeBaseJid(jid) {
  if (!jid) return '';
  // Strip device suffix "<id>:<device>@<server>" → "<id>@<server>"
  return jid.replace(/:\d+@/, '@');
}

async function handleSessionRecovery(deviceJid, baseJid, msgId) {
  if (!baseJid) return;
  if (!sock || typeof sock.assertSessions !== 'function') {
    console.debug(`[gateway][session-recovery] skipped for ${deviceJid}: socket not ready or assertSessions unavailable`);
    return;
  }
  const now = Date.now();
  const entry = sessionRecoveryMap.get(baseJid) || {
    attempts: 0,
    lastAttemptAt: 0,
    lastMsgId: null,
    notified: false,
  };
  if (now - entry.lastAttemptAt < SESSION_RECOVERY_COOLDOWN_MS) return;
  if (entry.attempts >= SESSION_RECOVERY_MAX_ATTEMPTS) {
    if (!entry.notified && OWNER_JIDS && OWNER_JIDS.length > 0) {
      entry.notified = true;
      sessionRecoveryMap.set(baseJid, entry);
      const peer = baseJid.replace(/@.*/, '');
      const body = [
        `⚠️ Unable to decrypt messages from ${peer}`,
        `Tried ${entry.attempts} Signal session renegotiations — peer hasn't re-keyed.`,
        `Last failed message id: ${entry.lastMsgId || 'unknown'}`,
        `Hint: ask the contact to send a new message, or unlink/relink this device if it is you.`,
      ].join('\n');
      for (const ownerJid of OWNER_JIDS) {
        try {
          await sock.sendMessage(ownerJid, { text: body });
        } catch (e) {
          console.warn(`[gateway][session-recovery] notify ${ownerJid} failed: ${e?.message || e}`);
        }
      }
      console.warn(`[gateway][session-recovery] exhausted, owner notified about ${baseJid}`);
    }
    return;
  }
  entry.attempts += 1;
  entry.lastAttemptAt = now;
  entry.lastMsgId = msgId || entry.lastMsgId;
  sessionRecoveryMap.set(baseJid, entry);
  console.warn(`[gateway][session-recovery] SessionError for ${deviceJid} (msgId=${msgId || 'n/a'}) — forcing renegotiation ${entry.attempts}/${SESSION_RECOVERY_MAX_ATTEMPTS}`);
  try {
    // Target the specific device so libsignal re-keys the exact chain that
    // failed. A group/base JID would skip per-device handshake.
    await sock.assertSessions([deviceJid], true);
  } catch (e) {
    console.warn(`[gateway][session-recovery] assertSessions(${deviceJid}) failed: ${e?.message || e}`);
  }
}

// Single periodic cleanup for both stores
setInterval(() => {
  const now = Date.now();
  for (const [id, entry] of messageStore) {
    if (now - entry.ts > MESSAGE_STORE_TTL_MS) messageStore.delete(id);
  }
  for (const [key, entry] of decryptRetryMap) {
    if (now - entry.firstSeen > DECRYPT_RETRY_EXPIRE_MS) cleanupDecryptRetry(key);
  }
  for (const [key, entry] of sessionRecoveryMap) {
    if (now - entry.lastAttemptAt > SESSION_RECOVERY_EXPIRE_MS) sessionRecoveryMap.delete(key);
  }
}, 60_000).unref();

// ---------------------------------------------------------------------------
// Markdown → WhatsApp formatting conversion
// ---------------------------------------------------------------------------
// LLM responses use standard Markdown but WhatsApp has its own formatting
// syntax. Convert the most common patterns so messages render correctly.
function markdownToWhatsApp(text) {
  if (!text) return text;

  // Step 1: Protect inline code from formatting — replace with placeholders.
  // Must run BEFORE bold/italic so `**bold**` inside backticks is untouched.
  const codeSlots = [];
  text = text.replace(/(?<!`)(`{1})(?!`)(.+?)(?<!`)\1(?!`)/g, (_, _tick, content) => {
    const idx = codeSlots.length;
    codeSlots.push(content);
    return '\x01CODE' + idx + 'CODE\x01';
  });

  // Step 2: Protect backslash-escaped stars — \* should stay literal.
  text = text.replace(/\\\*/g, '\x01ESCAPED_STAR\x01');

  // Step 3: Bold — **text** or __text__ → placeholder.
  // Only **text** is treated as bold. The __text__ form is intentionally
  // skipped because it's ambiguous with Python dunders (__init__, __main__).
  // LLM responses almost always use ** for bold.
  // Escape any `*` inside bold content to \x02 to prevent italic regex collision.
  text = text.replace(/\*\*(.+?)\*\*/g, (_, inner) => '\x01BOLD' + inner.replace(/\*/g, '\x02') + 'BOLD\x01');

  // Step 4: Italic — *text* → _text_ (WhatsApp italic).
  // Exclude bullet-list items: lines starting with `* ` (star + space).
  text = text.replace(/(?<!\*)\*(?!\*)(?!\s)(.+?)(?<!\s|\*)\*(?!\*)/g, (match, inner, offset) => {
    // Check if this is a bullet list item (star at line start followed by space)
    const lineStart = text.lastIndexOf('\n', offset - 1) + 1;
    if (offset === lineStart && text[offset + 1] === ' ') return match;
    return '_' + inner + '_';
  });

  // Step 5: Restore bold placeholders → *text* (WhatsApp bold)
  text = text.replace(/\x01BOLD(.+?)BOLD\x01/g, (_, inner) => '*' + inner.replace(/\x02/g, '*') + '*');

  // Step 6: Strikethrough — ~~text~~ → ~text~
  text = text.replace(/~~(.+?)~~/g, '~$1~');

  // Step 7: Restore inline code placeholders → ```text``` (WhatsApp monospace)
  text = text.replace(/\x01CODE(\d+)CODE\x01/g, (_, idx) => '```' + codeSlots[Number(idx)] + '```');

  // Step 8: Restore escaped stars → literal *
  text = text.replace(/\x01ESCAPED_STAR\x01/g, '*');

  return text;
}

// ---------------------------------------------------------------------------
// Step B: Conversation Tracker — in-memory Map with TTL
// ---------------------------------------------------------------------------
// Map<stranger_jid, ConversationState>
const activeConversations = new Map();

// Max messages to keep per conversation
const MAX_CONVERSATION_MESSAGES = 20;

/**
 * Record an inbound or outbound message in the conversation tracker.
 * Creates the conversation entry if it doesn't exist.
 */
function trackMessage(strangerJid, pushName, phone, text, direction) {
  let convo = activeConversations.get(strangerJid);
  if (!convo) {
    convo = {
      pushName,
      phone,
      messages: [],
      lastActivity: Date.now(),
      messageCount: 0,
      escalated: false,
    };
    activeConversations.set(strangerJid, convo);
  }
  convo.pushName = pushName || convo.pushName;
  convo.lastActivity = Date.now();
  convo.messageCount += 1;
  convo.messages.push({
    text: (text || '').substring(0, 500),
    timestamp: Date.now(),
    direction, // 'inbound' | 'outbound'
  });
  // Cap message history
  if (convo.messages.length > MAX_CONVERSATION_MESSAGES) {
    convo.messages = convo.messages.slice(-MAX_CONVERSATION_MESSAGES);
  }
}

/**
 * Evict expired conversations based on TTL.
 */
function evictExpiredConversations() {
  const now = Date.now();
  for (const [jid, convo] of activeConversations) {
    if (now - convo.lastActivity > CONVERSATION_TTL_MS) {
      console.log(`[gateway] Evicting expired conversation: ${convo.pushName} (${convo.phone})`);
      activeConversations.delete(jid);
    }
  }
}

// Periodic sweep every 15 minutes
setInterval(evictExpiredConversations, 15 * 60 * 1000);

// ---------------------------------------------------------------------------
// Step F: Rate limiting — per-JID for strangers
// ---------------------------------------------------------------------------
const rateLimitMap = new Map(); // Map<jid, { timestamps: number[] }>
const RATE_LIMIT_MAX = 3;       // max messages per window
const RATE_LIMIT_WINDOW_MS = 60_000; // 1 minute window

function isRateLimited(jid) {
  const now = Date.now();
  let entry = rateLimitMap.get(jid);
  if (!entry) {
    entry = { timestamps: [] };
    rateLimitMap.set(jid, entry);
  }
  // Remove timestamps outside the window
  entry.timestamps = entry.timestamps.filter(t => now - t < RATE_LIMIT_WINDOW_MS);
  if (entry.timestamps.length >= RATE_LIMIT_MAX) {
    return true;
  }
  entry.timestamps.push(now);
  return false;
}

// Cleanup rate limit entries every 5 minutes
setInterval(() => {
  const now = Date.now();
  for (const [jid, entry] of rateLimitMap) {
    entry.timestamps = entry.timestamps.filter(t => now - t < RATE_LIMIT_WINDOW_MS);
    if (entry.timestamps.length === 0) rateLimitMap.delete(jid);
  }
}, 5 * 60 * 1000);

// ---------------------------------------------------------------------------
// Message deduplication — Baileys can deliver the same message multiple times
// ---------------------------------------------------------------------------
// Two-phase (wasProcessed / markProcessed): Baileys re-emits `messages.upsert`
// for a msgId whose previous handling ended in decrypt failure (null payload /
// SessionError / PreKeyError). That retransmit is the ONLY window for
// `assertSessions` to recover the Signal session. A mark-on-sight dedup
// blocks the retransmit and strands the sender — 2026-04-16 outage, see
// lib/dedup-tracker.js docstring.
const dedupTracker = createDedupTracker({ windowMs: 60_000 });

// ---------------------------------------------------------------------------
// Step F: Escalation deduplication — debounce NOTIFY_OWNER per stranger
// ---------------------------------------------------------------------------
const lastEscalationTime = new Map(); // Map<stranger_jid, timestamp>
const ESCALATION_DEBOUNCE_MS = 5 * 60 * 1000; // 5 minutes

function shouldDebounceEscalation(strangerJid) {
  const last = lastEscalationTime.get(strangerJid);
  if (last && Date.now() - last < ESCALATION_DEBOUNCE_MS) {
    return true;
  }
  lastEscalationTime.set(strangerJid, Date.now());
  return false;
}

// Cleanup stale escalation entries every 10 minutes
setInterval(() => {
  const now = Date.now();
  for (const [jid, ts] of lastEscalationTime) {
    if (now - ts > ESCALATION_DEBOUNCE_MS) lastEscalationTime.delete(jid);
  }
}, 10 * 60 * 1000);

// ---------------------------------------------------------------------------
// Step D: Build active conversations context block for owner messages
// ---------------------------------------------------------------------------
function buildConversationsContext() {
  if (activeConversations.size === 0) return '';

  const lines = ['[ACTIVE_STRANGER_CONVERSATIONS]'];
  let idx = 1;
  for (const [jid, convo] of activeConversations) {
    const lastMsg = convo.messages[convo.messages.length - 1];
    const agoMs = Date.now() - (lastMsg?.timestamp || convo.lastActivity);
    const agoStr = formatTimeAgo(agoMs);
    const lastText = lastMsg ? `"${lastMsg.text.substring(0, 100)}"` : '(no messages)';
    const escalatedTag = convo.escalated ? ' [ESCALATED]' : '';
    lines.push(`${idx}. ${convo.pushName} (${convo.phone}) [JID: ${jid}] — last: ${lastText} (${agoStr})${escalatedTag}`);
    idx++;
  }
  lines.push('[/ACTIVE_STRANGER_CONVERSATIONS]');
  return lines.join('\n');
}

function formatTimeAgo(ms) {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

// ---------------------------------------------------------------------------
// Step C: Build stranger context prefix (factual only, no personality)
// ---------------------------------------------------------------------------
function buildStrangerContext(pushName, phone, strangerJid) {
  const convo = activeConversations.get(strangerJid);
  const messageCount = convo ? convo.messageCount : 1;
  const firstMessageAt = convo && convo.messages.length > 0
    ? new Date(convo.messages[0].timestamp).toISOString()
    : new Date().toISOString();

  return [
    '[WHATSAPP_STRANGER_CONTEXT]',
    `Incoming WhatsApp message from: ${pushName} (${phone})`,
    `This person is NOT the owner. They are an external contact.`,
    `Active conversation: ${messageCount} messages, started ${firstMessageAt}`,
    '',
    'Available routing tags:',
    '- [NOTIFY_OWNER]{"reason": "...", "summary": "..."}[/NOTIFY_OWNER] — sends a notification to the owner',
    '[/WHATSAPP_STRANGER_CONTEXT]',
    '',
  ].join('\n');
}

// ---------------------------------------------------------------------------
// Step C: Parse NOTIFY_OWNER tags from agent response
// ---------------------------------------------------------------------------
const NOTIFY_OWNER_RE = /\[NOTIFY_OWNER\]\s*(\{[\s\S]*?\})\s*\[\/NOTIFY_OWNER\]/g;

function extractNotifyOwner(responseText) {
  const notifications = [];
  for (const match of responseText.matchAll(NOTIFY_OWNER_RE)) {
    try {
      const parsed = JSON.parse(match[1]);
      notifications.push({
        reason: parsed.reason || 'unknown',
        summary: parsed.summary || '',
      });
    } catch {
      console.error('[gateway] Failed to parse NOTIFY_OWNER JSON:', match[1]);
    }
  }
  // §A — legacy text-tag path is kept one release for compatibility, but
  // every hit is loud-logged so callers can migrate to the typed
  // `notify_owner` tool. Suppressed when the new envelope already routed
  // the same payload (caller checks collectedOwnerNotices first).
  if (notifications.length > 0) {
    console.warn('[gateway][deprecated] NOTIFY_OWNER text tag detected; migrate to the notify_owner LLM tool. Hits:', notifications.length);
  }
  const cleanedText = responseText.replace(NOTIFY_OWNER_RE, '').trim();
  return { notifications, cleanedText };
}

// ---------------------------------------------------------------------------
// Silent-response sentinel — gateway-side mirror of the canonical Rust
// detector at crates/librefang-runtime/src/silent_response.rs.
// ---------------------------------------------------------------------------
// Two-layer protection (Phase 2 §B, OB-02 / OB-03 / OB-07):
//
//   1. `isSilentResponse(text)` classifies a complete (or accumulated)
//      response as silent — case-insensitive, with trailing punctuation /
//      whitespace / emoji tolerance, and proper word boundaries. Mirrors
//      the Rust canonical detector so both layers of the stack agree.
//
//   2. `createHoldbackAccumulator({onFlush, onSilent})` is the streaming
//      gate: it BUFFERS deltas instead of forwarding them and only
//      releases a flush once it can prove the cumulative text is NOT
//      sentinel-shaped (length threshold + classifier check). If the
//      stream ends silent, NO partial chunk ever escaped. This replaces
//      the post-hoc `stripNoReply` scrub which had a window where a
//      mid-stream "[no reply" prefix would already have been forwarded
//      as a WhatsApp message edit before the trailing " needed]" arrived.
//
// Toggle: `LIBREFANG_SILENT_V2=off` reverts to the legacy regex-scrub
// path, kept for one release as the rollback hatch.
const SILENT_V2_ENABLED = !['off', '0', 'false', 'no'].includes(
  String(process.env.LIBREFANG_SILENT_V2 || '').toLowerCase(),
);

// Hold-back window: the accumulator will not flush until the buffer has
// grown past this many chars OR the stream has ended. 32 is empirically
// long enough to cover the longest sentinel form ("[no reply needed]" =
// 18 chars) plus comfortable headroom for emoji/punctuation tolerance.
const SILENT_HOLDBACK_MIN_CHARS = 32;

// Match a buffer that LOOKS like it is becoming a silent sentinel.
// Used by the hold-back accumulator to keep buffering when the prefix
// is still ambiguous (e.g. the model has streamed `"Ok [no reply"` and
// we don't yet know whether the next delta closes it or extends into
// real text).
const SILENT_PREFIX_RE = /^\s*(\[?\s*no[_\s]?reply(?:\s*needed)?\s*\]?\s*[\s.!?]*)$/i;

function isSilentResponse(text) {
  if (typeof text !== 'string' || text.length === 0) return false;
  const trimmed = text.trim();
  if (!trimmed) return false;

  if (!SILENT_V2_ENABLED) {
    // Legacy detector — bit-for-bit equivalent to pre-Phase-2 behaviour.
    return (
      trimmed === 'NO_REPLY' ||
      trimmed.endsWith('NO_REPLY') ||
      trimmed === '[no reply needed]' ||
      trimmed.endsWith('[no reply needed]') ||
      trimmed === 'no reply needed' ||
      trimmed.endsWith('no reply needed')
    );
  }

  // Strip trailing punctuation, whitespace, and any non-ASCII (emoji).
  // Walk by code points (handles surrogate pairs for emojis correctly).
  let end = trimmed.length;
  while (end > 0) {
    let unitStart = end - 1;
    // If we're on a low surrogate, the actual code point starts one earlier.
    const lowUnit = trimmed.charCodeAt(unitStart);
    if (lowUnit >= 0xDC00 && lowUnit <= 0xDFFF && unitStart > 0) {
      const highUnit = trimmed.charCodeAt(unitStart - 1);
      if (highUnit >= 0xD800 && highUnit <= 0xDBFF) unitStart -= 1;
    }
    const c = trimmed.codePointAt(unitStart);
    const ch = String.fromCodePoint(c);
    const isStrippable =
      /\s/.test(ch) ||
      ch === '.' || ch === ',' || ch === ';' || ch === ':' || ch === '!' || ch === '?' ||
      c > 0x7F;
    if (!isStrippable) break;
    end = unitStart;
  }
  const stripped = trimmed.slice(0, end);
  const lower = stripped.toLowerCase();

  if (lower === 'no_reply' || lower === '[no reply needed]' || lower === 'no reply needed') {
    return true;
  }
  for (const needle of ['no_reply', '[no reply needed]', 'no reply needed']) {
    if (lower.endsWith(needle)) {
      const cut = lower.length - needle.length;
      if (cut === 0) return true;
      const prev = lower[cut - 1];
      // Word-boundary check: prev char must NOT be alphanumeric or _.
      if (!/[a-z0-9_]/.test(prev)) return true;
    }
  }
  return false;
}

// Legacy entry point preserved for the non-streaming and final-response
// scrub call sites (lines that historically called stripNoReply on a
// fully-formed response). When the response is a sentinel, returns ''
// so callers can short-circuit delivery; otherwise returns the text
// unchanged (the canonical V2 detector decides whole-message silence,
// no partial scrubbing — partial scrubbing was the bug).
function stripNoReply(text) {
  if (typeof text !== 'string' || !text) return text || '';
  if (isSilentResponse(text)) return '';
  if (!SILENT_V2_ENABLED) {
    // Legacy in-text scrub kept under the rollback flag.
    const stripped = text
      .replace(/\bNO_REPLY\b/g, '')
      .replace(/[ \t]+\n/g, '\n')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
    return stripped === 'NO_REPLY' ? '' : stripped;
  }
  return text;
}

/**
 * Streaming hold-back accumulator (OB-07).
 *
 * Buffers incoming deltas and decides — at every delta — whether the
 * cumulative text has clearly diverged from any silent-response shape.
 * Only then does it call `onFlush(buf)` once. After the first flush, all
 * subsequent deltas are forwarded immediately via `onFlush(delta)` so
 * progressive WhatsApp edits keep working unchanged.
 *
 * If the stream concludes silent, `onSilent(buf)` fires and `onFlush`
 * is NEVER called — guaranteeing zero partial sentinel leaks.
 *
 * @param {object} cfg
 * @param {(text: string) => (Promise<void> | void)} cfg.onFlush  forward
 *   text to the channel (e.g. sock.sendMessage / WhatsApp edit)
 * @param {(text: string) => (Promise<void> | void)} [cfg.onSilent] optional
 *   notification for diagnostics / OBS-01 logging
 * @param {number} [cfg.minChars=32] hold-back threshold
 */
function createHoldbackAccumulator({ onFlush, onSilent, minChars = SILENT_HOLDBACK_MIN_CHARS } = {}) {
  if (typeof onFlush !== 'function') {
    throw new TypeError('createHoldbackAccumulator requires onFlush callback');
  }
  let buf = '';
  let flushed = false;

  async function push(delta) {
    if (typeof delta !== 'string' || delta.length === 0) return;
    if (flushed) {
      await onFlush(delta);
      return;
    }
    buf += delta;
    const looksLikeSentinel = SILENT_PREFIX_RE.test(buf);
    if (buf.trim().length >= minChars && !looksLikeSentinel && !isSilentResponse(buf)) {
      flushed = true;
      await onFlush(buf);
    }
  }

  async function end() {
    if (flushed) return { silent: false, flushed: true };
    if (buf.trim().length === 0 || isSilentResponse(buf)) {
      if (typeof onSilent === 'function') await onSilent(buf);
      try {
        console.log(JSON.stringify({
          event: 'silent_response_gateway',
          final: true,
          silent: true,
          preview: buf.slice(0, 40),
        }));
      } catch { /* noop */ }
      return { silent: true, flushed: false };
    }
    flushed = true;
    await onFlush(buf);
    return { silent: false, flushed: true };
  }

  return { push, end, get buffered() { return buf; }, get hasFlushed() { return flushed; } };
}

// ---------------------------------------------------------------------------
// Step E: Parse relay commands from agent response
// ---------------------------------------------------------------------------

// The agent can embed a relay command in its response using this JSON format:
// [RELAY_TO_STRANGER]{"jid":"...@s.whatsapp.net","message":"..."}[/RELAY_TO_STRANGER]
const RELAY_RE = /\[RELAY_TO_STRANGER\]\s*(\{[\s\S]*?\})\s*\[\/RELAY_TO_STRANGER\]/g;

function extractRelayCommands(responseText) {
  const relays = [];
  for (const match of responseText.matchAll(RELAY_RE)) {
    try {
      const parsed = JSON.parse(match[1]);
      if (parsed.jid && parsed.message) {
        relays.push({ jid: parsed.jid, message: parsed.message });
      }
    } catch {
      console.error('[gateway] Failed to parse relay command JSON:', match[1]);
    }
  }
  const cleanedText = responseText.replace(RELAY_RE, '').trim();
  return { relays, cleanedText };
}

// ---------------------------------------------------------------------------
// Step F: Anti-confusion safeguards — relay validation + audit logging
// ---------------------------------------------------------------------------

/**
 * Validate and execute a relay to a stranger.
 * Returns a status string for the owner confirmation.
 */
async function executeRelay(relay) {
  const { jid, message } = relay;

  // F1: JID must exist in active conversations
  const convo = activeConversations.get(jid);
  if (!convo) {
    const errorMsg = `Relay rejected: no active conversation for JID ${jid}. The conversation may have expired.`;
    console.warn(`[gateway] ${errorMsg}`);
    return { success: false, error: errorMsg };
  }

  // F2: Socket must be connected
  if (!sock || connStatus !== 'connected') {
    return { success: false, error: 'WhatsApp not connected' };
  }

  try {
    const sentRelay = await sock.sendMessage(jid, { text: markdownToWhatsApp(message) });
    if (ECHO_TRACKER_ENABLED) echoTracker.track(message);

    // F4: Audit log
    console.log(`[gateway] RELAY SENT | to: ${convo.pushName} (${convo.phone}) [${jid}] | message: "${message.substring(0, 100)}" | timestamp: ${new Date().toISOString()}`);

    // Update conversation tracker with outbound message
    trackMessage(jid, convo.pushName, convo.phone, message, 'outbound');
    // Save relay outbound to DB
    dbSaveMessage({ id: sentRelay?.key?.id || randomUUID(), jid, senderJid: ownJid, pushName: null, phone: convo.phone, text: message, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });

    return { success: true, recipient: convo.pushName, phone: convo.phone };
  } catch (err) {
    console.error(`[gateway] Relay send failed to ${jid}:`, err.message);
    return { success: false, error: err.message };
  }
}

// ---------------------------------------------------------------------------
// Resolve agent name → UUID via LibreFang API
// ---------------------------------------------------------------------------
function resolveAgentId() {
  return new Promise((resolve, reject) => {
    // If DEFAULT_AGENT is already a UUID, use it directly
    if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(DEFAULT_AGENT)) {
      cachedAgentId = DEFAULT_AGENT;
      return resolve(DEFAULT_AGENT);
    }

    const url = new URL(`${LIBREFANG_URL}/api/agents`);

    const req = http.request(
      {
        hostname: url.hostname,
        port: url.port || 4545,
        path: url.pathname,
        method: 'GET',
        headers: { 'Accept': 'application/json' },
        timeout: 10_000,
      },
      (res) => {
        let body = '';
        res.on('data', (chunk) => (body += chunk));
        res.on('end', () => {
          try {
            const parsed = JSON.parse(body);
            const agents = Array.isArray(parsed) ? parsed : (parsed.items || []);
            if (!agents.length) {
              return reject(new Error('No agents returned from /api/agents'));
            }
            // Match by name (case-insensitive)
            const match = agents.find(
              (a) => (a.name || '').toLowerCase() === DEFAULT_AGENT.toLowerCase()
            );
            if (match && match.id) {
              cachedAgentId = match.id;
              console.log(`[gateway] Resolved agent "${DEFAULT_AGENT}" → ${cachedAgentId}`);
              resolve(cachedAgentId);
            } else if (agents.length > 0) {
              // Fallback: use first available agent
              cachedAgentId = agents[0].id;
              console.log(`[gateway] Agent "${DEFAULT_AGENT}" not found, using first agent: ${cachedAgentId}`);
              resolve(cachedAgentId);
            } else {
              reject(new Error('No agents available on LibreFang'));
            }
          } catch (e) {
            reject(new Error(`Failed to parse /api/agents: ${e.message}`));
          }
        });
      },
    );

    req.on('error', reject);
    req.on('timeout', () => {
      req.destroy();
      reject(new Error('LibreFang /api/agents timeout'));
    });
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Baileys connection
// ---------------------------------------------------------------------------
async function cleanupSocket() {
  // ST-01: stop heartbeat watchdog whenever we tear down the socket — both
  // planned reconnect and gracefulShutdown go through here.
  if (heartbeatInterval) {
    clearInterval(heartbeatInterval);
    heartbeatInterval = null;
  }
  if (!sock) return;
  const previousSock = sock;
  sock = null;
  ownJid = null;
  try { previousSock.ev?.removeAllListeners?.(); } catch {}
  try { previousSock.ws?.close?.(); } catch {}
  try { previousSock.end?.(); } catch {}
}

async function startConnection() {
  if (isConnecting) {
    console.log('[gateway] Connection attempt already in progress, skipping');
    return;
  }
  isConnecting = true;
  try {

  // Dynamic imports — Baileys is ESM-only in v6+
  const { default: makeWASocket, useMultiFileAuthState, DisconnectReason, fetchLatestBaileysVersion } =
    await import('@whiskeysockets/baileys');
  const QRCode = (await import('qrcode')).default || await import('qrcode');
  const pino = (await import('pino')).default || await import('pino');

  const logger = pino({ level: 'warn' });

  const { state, saveCreds } = await useMultiFileAuthState(
    require('node:path').join(__dirname, 'auth_store')
  );
  const { version } = await fetchLatestBaileysVersion();

  sessionId = randomUUID();
  qrDataUrl = '';
  qrExpired = false;
  connStatus = 'disconnected';
  statusMessage = 'Connecting...';

  sock = makeWASocket({
    version,
    auth: state,
    logger,
    browser: ['LibreFang', 'Desktop', '1.0.0'],
    // getMessage enables Baileys' built-in retry mechanism for decryption failures.
    // When a message cannot be decrypted, Baileys sends a retry receipt to the sender
    // and needs getMessage() to return the raw message for re-decryption.
    getMessage: async (key) => messageStoreGet(key.id),
  });

  // Save credentials whenever they update
  sock.ev.on('creds.update', saveCreds);

  // Phase 2 §C / GS-01 — invalidate cached group roster on membership change
  // so adds/removes/promotions become visible at the next inbound message.
  sock.ev.on('group-participants.update', (update) => {
    const id = update && update.id;
    if (id) invalidateGroupRoster(id);
  });

  // Connection state changes (QR code, connected, disconnected)
  sock.ev.on('connection.update', async (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) {
      // New QR code generated — convert to data URL
      try {
        qrDataUrl = await QRCode.toDataURL(qr, { width: 256, margin: 2 });
        connStatus = 'qr_ready';
        qrExpired = false;
        statusMessage = 'Scan this QR code with WhatsApp → Linked Devices';
        console.log('[gateway] QR code ready — waiting for scan');
      } catch (err) {
        console.error('[gateway] QR generation failed:', err.message);
      }
    }

    if (connection === 'close') {
      const statusCode = lastDisconnect?.error?.output?.statusCode;
      const reason = lastDisconnect?.error?.output?.payload?.message || 'unknown';
      console.log(`[gateway] Connection closed: ${reason} (${statusCode})`);

      if (statusCode === DisconnectReason.loggedOut) {
        // User logged out from phone — clear auth and stop
        connStatus = 'disconnected';
        statusMessage = 'Logged out. Generate a new QR code to reconnect.';
        qrDataUrl = '';
        await cleanupSocket();
        reconnectAttempts = 0;
        cachedAgentId = null;
        // Remove auth store so next connect gets a fresh QR
        const fs = require('node:fs');
        const path = require('node:path');
        const authPath = path.join(__dirname, 'auth_store');
        if (fs.existsSync(authPath)) {
          fs.rmSync(authPath, { recursive: true, force: true });
        }
      } else if (statusCode === DisconnectReason.forbidden) {
        // Non-recoverable — don't auto-reconnect
        connStatus = 'disconnected';
        statusMessage = `Disconnected: ${reason}. Use POST /login/start to reconnect.`;
        qrDataUrl = '';
        await cleanupSocket();
      } else {
        // ST-02: all other disconnect reasons are recoverable. Exponential
        // backoff 2s → 30s, factor 1.8, ±25% jitter, NO hard stop — a
        // transient outage longer than 5 attempts (the previous cap) used
        // to leave the gateway permanently disconnected until manual
        // restart. We now keep retrying at the capped interval.
        reconnectAttempts += 1;
        const delay = computeBackoffDelay(reconnectAttempts);
        console.log(
          `[gateway] Reconnecting in ${Math.round(delay / 1000)}s (attempt ${reconnectAttempts}, jittered)`,
        );
        connStatus = 'disconnected';
        statusMessage = `Reconnecting (attempt ${reconnectAttempts})...`;
        setTimeout(() => startConnection(), delay);
      }
    }

    if (connection === 'open') {
      connStatus = 'connected';
      qrExpired = false;
      qrDataUrl = '';
      reconnectAttempts = 0;
      statusMessage = 'Connected to WhatsApp';
      console.log('[gateway] Connected to WhatsApp!');

      // ST-01: (re)start heartbeat watchdog. Paused while sock is null or
      // status is not 'connected' (initial connect + planned reconnect gap).
      lastInboundAt = Date.now();
      if (heartbeatInterval) clearInterval(heartbeatInterval);
      heartbeatInterval = setInterval(() => {
        if (!sock || connStatus !== 'connected') return;
        const now = Date.now();
        if (checkHeartbeat(now, lastInboundAt, HEARTBEAT_MS)) {
          console.log(JSON.stringify({
            event: 'heartbeat_timeout',
            last_inbound_ms: now - lastInboundAt,
            threshold_ms: HEARTBEAT_MS,
          }));
          try { sock.end(undefined); } catch { /* best-effort */ }
        }
      }, HEARTBEAT_CHECK_INTERVAL_MS);

      // Capture own JID for self-chat detection
      if (sock?.user?.id) {
        // Baileys user.id is like "1234567890:42@s.whatsapp.net" — normalize
        ownJid = sock.user.id.replace(/:.*@/, '@');
        console.log(`[gateway] Own JID: ${ownJid}`);
      }

      // Invalidate cached agent UUID on reconnect — the daemon may have
      // restarted and agents may have new UUIDs.
      cachedAgentId = null;

      // Resolve LIDs for every OWNER_NUMBERS entry so that LID-addressed
      // messages from the owner are recognised without waiting for the first
      // senderPn event. Best-effort: if the call fails (old Baileys, no
      // network, number not on WhatsApp) we log and continue — subsequent
      // senderPn events will still populate `lidToPnJid`.
      if (OWNER_JIDS.size > 0 && typeof sock.onWhatsApp === 'function') {
        try {
          const results = await sock.onWhatsApp(...[...OWNER_JIDS]);
          for (const r of results || []) {
            if (r && r.exists && r.lid) {
              ownerLidJids.add(r.lid);
              if (r.jid) lidMapSet(r.lid, r.jid);
            }
          }
          if (ownerLidJids.size > 0) {
            console.log(`[gateway] Owner LIDs resolved → ${[...ownerLidJids].join(', ')}`);
          }
        } catch (err) {
          console.warn(`[gateway] Failed to resolve owner LIDs: ${err.message}`);
        }
      }
    }
  });

  // Incoming messages → forward to LibreFang
  sock.ev.on('messages.upsert', async ({ messages, type }) => {
    // ST-01: any inbound activity refreshes the heartbeat timestamp, even
    // for non-notify events (history syncs, retries) — they still prove the
    // socket is live.
    lastInboundAt = Date.now();
    if (type !== 'notify') return;

    for (const msg of messages) {
      // Store raw message for Baileys retry mechanism and resolve successful retries
      if (msg.key?.id && msg.message) {
        messageStoreSet(msg.key.id, msg.message);
        const retryKey = getDecryptRetryKey(msg.key.remoteJid || '', msg.key.id);
        if (decryptRetryMap.has(retryKey)) {
          console.log(`[gateway][retry] Decryption retry succeeded for ${msg.key.id}`);
          cleanupDecryptRetry(retryKey);
          try { stmtMarkProcessed.run(1, msg.key.id); } catch (_) { /* best-effort */ }
        }
      }

      // Skip status broadcasts
      if (msg.key.remoteJid === 'status@broadcast') continue;

      // Read-only dedup check: do NOT mark here. Marking happens after the
      // decrypt-failure branch below so WA's retransmit of a failed-decrypt
      // msgId can reach the session-recovery path instead of being stranded.
      if (dedupTracker.wasProcessed(msg.key.id)) {
        console.log(`[gateway] Skipping duplicate message: ${msg.key.id}`);
        continue;
      }

      // Handle self-chat ("Notes to Self"): fromMe messages to own JID.
      if (msg.key.fromMe) {
        const isSelfChat = ownJid && msg.key.remoteJid === ownJid;
        if (!isSelfChat) continue; // Skip regular outgoing messages
      }

      const sender = msg.key.remoteJid || '';
      const innerMsg = msg.message || {};

      // Signal session recovery: inbound message with null payload ⇒ libsignal
      // rejected the ciphertext before stub 39 was emitted. Force a fresh
      // session with the actual device (keep the device suffix when
      // assertSessions is called — the session is per-device, not per-user).
      if (!msg.message && !msg.key.fromMe && sender) {
        const recoveryJid = msg.key.participant || sender;
        const baseJid = normalizeBaseJid(recoveryJid);
        handleSessionRecovery(recoveryJid, baseJid, msg.key.id).catch(err => {
          console.warn(`[gateway][session-recovery] handler error: ${err?.message || err}`);
        });
        continue;
      }

      // Decrypt succeeded. Mark so subsequent retransmits of this msgId
      // are deduped.
      dedupTracker.markProcessed(msg.key.id);

      // --- FASE 4: Handle reactions ---
      if (innerMsg.reactionMessage) {
        const emoji = innerMsg.reactionMessage.text;
        const reactedMsgId = innerMsg.reactionMessage.key?.id || '';
        if (emoji) {
          console.log(`[gateway] Reaction ${emoji} from ${msg.pushName || sender} on msg ${reactedMsgId}`);
          // Only forward non-empty reactions (empty = reaction removed)
          // For now, skip reactions — they don't need agent processing
        }
        continue;
      }

      // --- Extract text from various message types ---
      const text = innerMsg.conversation
        || innerMsg.extendedTextMessage?.text
        || innerMsg.imageMessage?.caption
        || innerMsg.videoMessage?.caption
        || innerMsg.documentWithCaptionMessage?.message?.documentMessage?.caption
        || '';

      // Check for downloadable media
      const downloadableMedia = getDownloadableMedia(innerMsg);
      // Legacy fallback descriptor for non-downloadable media or download failures
      const mediaDescriptor = getMediaDescriptor(innerMsg, msg.pushName || sender);

      // --- FASE 4: Improved location handling ---
      if (innerMsg.locationMessage || innerMsg.liveLocationMessage) {
        const loc = innerMsg.locationMessage || innerMsg.liveLocationMessage;
        const lat = loc.degreesLatitude;
        const lon = loc.degreesLongitude;
        const locName = loc.name || loc.address || '';
        const locLabel = locName ? `${locName} — ` : '';
        // Override mediaDescriptor with enriched location text
        const locationText = `[Location: ${locLabel}${lat}, ${lon} — https://maps.google.com/?q=${lat},${lon}]`;
        // Fall through to normal message processing with this text
        innerMsg._overrideMediaText = locationText;
      }

      // --- FASE 4: Improved contact handling ---
      if (innerMsg.contactMessage) {
        const vcard = innerMsg.contactMessage.vcard || '';
        let contactName = innerMsg.contactMessage.displayName || '';
        let contactPhone = '';
        // Parse vCard for phone number
        const telMatch = vcard.match(/TEL[^:]*:([+\d\s-]+)/i);
        if (telMatch) contactPhone = telMatch[1].trim();
        const fnMatch = vcard.match(/FN:(.+)/i);
        if (fnMatch && !contactName) contactName = fnMatch[1].trim();
        innerMsg._overrideMediaText = `[Shared contact: ${contactName}${contactPhone ? ' ' + contactPhone : ''}]`;
      }
      if (innerMsg.contactsArrayMessage) {
        const contacts = innerMsg.contactsArrayMessage.contacts || [];
        const parsed = contacts.map(c => {
          const vcard = c.vcard || '';
          const name = c.displayName || '';
          const telMatch = vcard.match(/TEL[^:]*:([+\d\s-]+)/i);
          const phone = telMatch ? telMatch[1].trim() : '';
          return `${name}${phone ? ' ' + phone : ''}`;
        });
        innerMsg._overrideMediaText = `[Shared contacts: ${parsed.join(', ')}]`;
      }

      // Skip if there's nothing to process
      if (!text && !downloadableMedia && !mediaDescriptor && !innerMsg._overrideMediaText) continue;

      // Extract real phone number
      //
      // `sender` (= msg.key.remoteJid) may be:
      //   - '<digits>@s.whatsapp.net' — standard phone-number JID
      //   - '<digits>@lid'            — WhatsApp anonymous LID (opaque)
      //   - '<digits>@g.us'           — group JID (we handle separately below)
      //
      // A LID by itself is NOT a phone number — using it as such produces
      // bogus 15-digit phone strings and causes every LID-addressed message
      // to be mis-classified as from a stranger. Resolve via, in order:
      //   1. `msg.key.senderPn` (sometimes provided by Baileys directly)
      //   2. `lidToPnJid` cache populated by previous (1)s or by onWhatsApp()
      //   3. `msg.key.participant` (groups; the actual sender inside)
      //   4. `sender` itself when it's already an `@s.whatsapp.net` JID
      // If none of the above yields a phone-number JID, `phone` is left as
      // a placeholder and we flag the sender as unresolved.
      const isGroup = isGroupJid(sender);
      const isLid = isLidJid(sender);
      const senderPnRaw = msg.key.senderPn || '';

      // Cache LID → phone-number JID when we see both on the same message.
      // Side effect lives OUTSIDE resolvePeerId — Plan 01 §Concerns #1.
      if (isLid && senderPnRaw) {
        lidMapSet(sender, senderPnRaw);
      }

      // CS-02: first-seen LID without senderPn AND not in cache — proactively
      // ask Baileys for the PN mapping with a 5s timeout. Populates cache so
      // the next message in the burst resolves synchronously.
      // Side effect lives OUTSIDE resolvePeerId — Plan 01 §Concerns #1.
      if (isLid && !senderPnRaw && !lidToPnJid.has(sender)) {
        const tag = await resolveLidProactively(sock, sender, lidToPnJid, 5000);
        // On 'resolved' the function already wrote into the Map; mirror that
        // into SQLite via the write-through helper. The double-set into the
        // Map is a no-op (same key, same value).
        if (tag === 'resolved') {
          const pn = lidToPnJid.get(sender);
          if (pn) lidMapSet(sender, pn);
        }
      }

      // Centralized resolution — Phase 4 §A (ID-01).
      const { peer: senderPnJid, confidence } = resolvePeerId(sender, {
        lidToPnCache: lidToPnJid,
        senderPn: senderPnRaw,
        participant: msg.key.participant || '',
      });

      const phone = extractE164(senderPnJid);
      const phoneResolved = phone !== '';
      const pushName = msg.pushName || phone || sender;

      if (!phoneResolved) {
        // ID-03 structured log — every lid_unresolved outcome.
        const reason = senderPnRaw ? 'senderPn_present_but_unextractable'
          : (isLid && lidToPnJid.has(sender)) ? 'cache_hit_but_unextractable'
          : msg.key.participant ? 'participant_was_lid'
          : 'no_mapping_available';
        console.warn(JSON.stringify({
          event: 'identity_unresolved',
          jid: sender,
          reason,
          lid_cache_size: lidToPnJid.size,
          confidence,
        }));
      }

      // Determine sender type. Owner check accepts either the resolved
      // phone-number JID or a LID previously bound to an owner number.
      const isOwner = OWNER_JIDS.size > 0 && (
        (senderPnJid && OWNER_JIDS.has(senderPnJid)) ||
        (isLid && ownerLidJids.has(sender))
      );
      const isStranger = !isGroup && OWNER_JIDS.size > 0 && !isOwner;

      // Detect @mention: check if our JID is in the mentionedJid list
      let wasMentioned = false;
      if (isGroup && ownJid) {
        const mentionedJids = innerMsg.extendedTextMessage?.contextInfo?.mentionedJid
          || innerMsg.imageMessage?.contextInfo?.mentionedJid
          || innerMsg.videoMessage?.contextInfo?.mentionedJid
          || [];
        // ownJid is normalized like "1234567890@s.whatsapp.net"
        const ownNumber = ownJid.replace(/@.*$/, '');
        wasMentioned = mentionedJids.some(jid => jid.replace(/@.*$/, '') === ownNumber);
      }

      // Rate limiting for strangers and group messages
      if ((isStranger || isGroup) && isRateLimited(sender)) {
        console.log(`[gateway] Rate limited: ${pushName} (${phone}) — dropping message`);
        continue;
      }

      // --- Resolve agent ID early (needed for media upload) ---
      if (!cachedAgentId) {
        try {
          await resolveAgentId();
        } catch (err) {
          console.error(`[gateway] Agent resolution failed: ${err.message}`);
          continue;
        }
      }

      // --- FASE 1: Process media (download + upload to LibreFang) ---
      let attachments = [];
      let messageText = text;
      let transcriptionText = '';

      if (downloadableMedia) {
        const result = await processMediaMessage(msg, innerMsg, cachedAgentId);
        if (result && result.attachment) {
          attachments.push(result.attachment);
          if (result.transcription) {
            transcriptionText = result.transcription;
          }
          // If no text caption, generate a default message
          if (!messageText) {
            if (transcriptionText) {
              // Audio with transcription: use transcription as message text
              const ptt = innerMsg.audioMessage?.ptt;
              messageText = `[${ptt ? 'Voice' : 'Audio'} transcription]: ${transcriptionText}`;
            } else {
              messageText = innerMsg._overrideMediaText || getMediaFilename(downloadableMedia.type, downloadableMedia.msg);
            }
          }
        } else if (result && result.fallbackText) {
          // File too large
          messageText = result.fallbackText;
        } else {
          // Download/upload failed — fall back to text descriptor
          console.warn(`[gateway] Media processing failed, falling back to text descriptor`);
          messageText = messageText || innerMsg._overrideMediaText || mediaDescriptor || '[Unprocessable media]';
        }
      } else if (innerMsg._overrideMediaText) {
        // Location or contact — no downloadable media, just enriched text
        messageText = innerMsg._overrideMediaText;
      } else if (!messageText && mediaDescriptor) {
        // Fallback for unknown media types
        messageText = mediaDescriptor;
      }

      if (!messageText && attachments.length === 0) continue;

      // --- Phase 3 §A: Echo tracker gate (EB-01) ---
      // Drop messages whose body matches a recently-sent outbound text
      // (self-loop prevention when WhatsApp reflects our own message back
      // via sync/cross-device mirror). Flag `LIBREFANG_ECHO_TRACKER=off`
      // disables this gate. Only checks text bodies (never attachments).
      if (ECHO_TRACKER_ENABLED && messageText && echoTracker.isEcho(messageText)) {
        console.log(JSON.stringify({
          event: 'echo_drop',
          body_excerpt: EchoTracker.normalize(messageText).slice(0, 80),
          tracker_size: echoTracker.size(),
          elapsed_ms_since_last_sent: echoTracker.elapsedSinceLastSent(),
        }));
        continue;
      }

      // --- FASE 2: Reply context (quotedMessage) ---
      const contextSources = [
        innerMsg.extendedTextMessage?.contextInfo,
        innerMsg.imageMessage?.contextInfo,
        innerMsg.videoMessage?.contextInfo,
        innerMsg.audioMessage?.contextInfo,
        innerMsg.documentMessage?.contextInfo,
        innerMsg.stickerMessage?.contextInfo,
      ];
      const contextInfo = contextSources.find(c => c) || null;

      if (contextInfo?.quotedMessage) {
        const quoted = contextInfo.quotedMessage;
        const quotedText = quoted.conversation
          || quoted.extendedTextMessage?.text
          || quoted.imageMessage?.caption
          || quoted.videoMessage?.caption
          || '';
        if (quotedText) {
          messageText = `[In risposta a: "${quotedText.substring(0, 200)}"]\n${messageText}`;
        }
      }

      // --- FASE 2: Forwarded message context ---
      if (contextInfo?.isForwarded) {
        messageText = `[Forwarded message]\n${messageText}`;
      }

      console.log(`[gateway] Incoming from ${pushName} (${phone}): ${messageText.substring(0, 80)}${attachments.length ? ` [+${attachments.length} attachment(s)]` : ''}`);

      // --- Message Store: determine raw type ---
      const rawType = downloadableMedia ? downloadableMedia.type.replace('Message', '')
        : innerMsg.locationMessage ? 'location'
        : innerMsg.contactMessage ? 'contact'
        : innerMsg.contactsArrayMessage ? 'contacts'
        : innerMsg.reactionMessage ? 'reaction'
        : 'text';

      // --- Message Store: save inbound message BEFORE processing (processed=0) ---
      const msgTimestamp = (msg.messageTimestamp
        ? (typeof msg.messageTimestamp === 'number' ? msg.messageTimestamp * 1000 : Number(msg.messageTimestamp) * 1000)
        : Date.now());
      dbSaveMessage({
        id: msg.key.id,
        jid: sender,
        senderJid: msg.key.participant || sender,
        pushName,
        phone,
        text: messageText,
        direction: 'inbound',
        timestamp: msgTimestamp,
        processed: 0,
        rawType,
      });
      dbUpdateLastSeen(sender, msgTimestamp);

      // Send read receipt (blue ticks) immediately
      await sock.readMessages([msg.key]);

      // Forward to LibreFang agent
      try {
        // Track stranger messages
        if (isStranger) {
          trackMessage(sender, pushName, phone, messageText, 'inbound');
        }

        // Build the message to send to the agent
        let messageToSend;
        let systemPrefix = '';

        if (isGroup) {
          // Include sender identity so the LLM knows who is talking in the group
          messageToSend = `[Group message from ${pushName || phone}]\n${messageText}`;
        } else if (isStranger) {
          const strangerContext = buildStrangerContext(pushName, phone, sender);
          messageToSend = strangerContext + messageText;
        } else if (isOwner && activeConversations.size > 0 && ownerIntentsRelay(messageText)) {
          // Only inject the relay system instruction when the owner's text
          // expresses an explicit delegated-speech intent. A neutral greeting
          // from the owner to the agent during an active stranger conversation
          // must NOT be forced into relay mode.
          const context = buildConversationsContext();
          systemPrefix = buildRelaySystemInstruction();
          messageToSend = context + '\n\n[OWNER_MESSAGE]\n' + messageText;
        } else {
          messageToSend = messageText;
        }

        // --- Streaming: progressive message edits while LLM generates ---
        let streamMsgKey = null; // key of the initial WhatsApp message we'll edit
        const onProgress = async (partialText) => {
          if (!sock) return;
          // Strip internal tags before sending partial text to WhatsApp.
          // Bail early if no brackets — most chunks won't contain tags.
          let cleaned = partialText;
          if (cleaned.includes('[NOTIFY_OWNER]') || cleaned.includes('[RELAY_TO_STRANGER]') || cleaned.includes('[no reply needed]')) {
            cleaned = cleaned
              .replace(NOTIFY_OWNER_RE, '')
              .replace(RELAY_RE, '')
              .replace(/\[no reply needed\]/gi, '');
          }
          // OB-07 hold-back gate: until we have already established a
          // visible WhatsApp message (streamMsgKey != null), refuse to
          // flush a chunk that is shorter than the hold-back threshold OR
          // whose accumulated text still LOOKS like a silent sentinel.
          // Once committed, subsequent edits are unconditional — the
          // model has clearly diverged from any sentinel shape.
          if (SILENT_V2_ENABLED && !streamMsgKey) {
            const trimmedCum = cleaned.trim();
            if (
              trimmedCum.length < SILENT_HOLDBACK_MIN_CHARS ||
              SILENT_PREFIX_RE.test(trimmedCum) ||
              isSilentResponse(cleaned)
            ) {
              try {
                console.log(JSON.stringify({
                  event: 'silent_response_gateway',
                  final: false,
                  silent: true,
                  preview: trimmedCum.slice(0, 40),
                }));
              } catch { /* noop */ }
              return;
            }
          }
          // Final scrub for trailing-sentinel residue (e.g. ".... NO_REPLY"
          // arriving after the hold-back window has elapsed).
          cleaned = stripNoReply(cleaned);
          cleaned = cleaned.trim();
          if (!cleaned) return;
          const formatted = markdownToWhatsApp(cleaned);
          if (!streamMsgKey) {
            const sent = await sock.sendMessage(sender, { text: formatted });
            streamMsgKey = sent?.key;
          } else {
            await sock.sendMessage(sender, { text: formatted, edit: streamMsgKey });
          }
          if (ECHO_TRACKER_ENABLED) echoTracker.track(cleaned);
        };

        // Phase 2 §C — fetch participant roster for groups (cached 5min).
        const groupParticipants = isGroup ? await getGroupParticipants(sock, sender) : [];

        // §A — collect typed owner notices emitted via the notify_owner tool.
        const collectedOwnerNotices = [];
        const onOwnerNotice = (text) => {
          if (!text) return;
          collectedOwnerNotices.push(text);
        };
        const rawResponse = await forwardToLibreFangStreaming(
          messageToSend, systemPrefix, phone, pushName, isOwner, attachments, onProgress, sender, { isGroup, wasMentioned, groupParticipants, onOwnerNotice },
        );

        // §A — fan out collected owner notices to every configured OWNER_JID.
        // OB-01: happens regardless of whether a public reply will be sent
        // below; the owner receives the private payload even when the model
        // elects to stay silent in the source chat.
        if (OWNER_CHANNEL_ENABLED && collectedOwnerNotices.length > 0) {
          if (OWNER_JIDS.size === 0) {
            console.log(JSON.stringify({
              event: 'owner_notify_skip',
              reason: 'no_owner_configured',
              source_chat: sender,
              count: collectedOwnerNotices.length,
            }));
          } else {
            for (const noticeText of collectedOwnerNotices) {
              for (const ownerJid of OWNER_JIDS) {
                try {
                  await sock.sendMessage(ownerJid, { text: noticeText });
                } catch (e) {
                  console.error(`[gateway] owner_notify send failed to ${ownerJid}: ${e.message}`);
                }
              }
              console.log(JSON.stringify({
                event: 'owner_notify',
                target_jids: [...OWNER_JIDS],
                source_chat: sender,
                bytes: noticeText.length,
              }));
            }
          }
        }

        // Scrub NO_REPLY before markdown conversion — if the model emitted it
        // trailing or glued to an emoji it would otherwise reach WhatsApp.
        const response = markdownToWhatsApp(stripNoReply(rawResponse));

        // Helper: send a new message or edit the streamed one for final delivery
        const sendOrEdit = async (jid, finalText) => {
          if (streamMsgKey && jid === sender) {
            // Edit the message we've been streaming
            await sock.sendMessage(jid, { text: finalText, edit: streamMsgKey });
            if (ECHO_TRACKER_ENABLED) echoTracker.track(finalText);
            return streamMsgKey;
          }
          // No streaming happened (fallback path) — send new message
          const sentKey = (await sock.sendMessage(jid, { text: finalText }))?.key;
          if (ECHO_TRACKER_ENABLED) echoTracker.track(finalText);
          return sentKey;
        };

        if (response && sock) {
          if (isStranger) {
            // Step C: Agent response goes to STRANGER, not owner
            const { notifications, cleanedText } = extractNotifyOwner(response);

            // Send cleaned response to the stranger (format after tag extraction)
            if (cleanedText) {
              const formattedText = markdownToWhatsApp(cleanedText);
              const sentKey = await sendOrEdit(sender, formattedText);
              console.log(`[gateway] Replied to stranger ${pushName} (${phone})${streamMsgKey ? ' (streamed)' : ''}`);

              // Track outbound message
              trackMessage(sender, pushName, phone, cleanedText, 'outbound');
              // Save outbound to DB
              dbSaveMessage({ id: sentKey?.id || randomUUID(), jid: sender, senderJid: ownJid, pushName: null, phone, text: cleanedText, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });
            }

            // Step C + F: If NOTIFY_OWNER tags found, send notification to owner
            for (const notif of notifications) {
              const convo = activeConversations.get(sender);
              // F: Escalation deduplication
              if (shouldDebounceEscalation(sender)) {
                console.log(`[gateway] Debounced escalation for ${pushName} — skipping duplicate notification`);
                continue;
              }

              // Mark conversation as escalated
              if (convo) convo.escalated = true;

              const ownerNotif = notif.summary || `[${pushName}] ${notif.reason}`;

              // Send notification to primary owner
              await sock.sendMessage(OWNER_JID, { text: ownerNotif });
              if (ECHO_TRACKER_ENABLED) echoTracker.track(ownerNotif);
              console.log(`[gateway] NOTIFY_OWNER sent for ${pushName}: ${notif.reason}`);
            }

          } else if (isOwner && !isGroup) {
            // Step E: Check for relay commands in the agent response (DMs only, never groups)
            const { relays, cleanedText } = extractRelayCommands(response);

            // Execute any relay commands
            const relayResults = [];
            for (const relay of relays) {
              const result = await executeRelay(relay);
              relayResults.push(result);
            }

            // Build owner confirmation message
            let ownerReply = cleanedText;

            // Log relay results (don't append technical details to owner message)
            for (let i = 0; i < relayResults.length; i++) {
              const r = relayResults[i];
              if (r.success) {
                console.log(`[gateway] Relay delivered to ${r.recipient} (${r.phone})`);
              } else {
                console.error(`[gateway] Relay failed: ${r.error}`);
                const failLine = `\n✗ Relay failed: ${r.error}`;
                ownerReply = ownerReply ? ownerReply + failLine : failLine.trim();
              }
            }

            if (ownerReply) {
              ownerReply = markdownToWhatsApp(ownerReply);
              const sentKey = await sendOrEdit(sender, ownerReply);
              console.log(`[gateway] Replied to owner (${sender})${streamMsgKey ? ' (streamed)' : ''}`);
              dbSaveMessage({ id: sentKey?.id || randomUUID(), jid: sender, senderJid: ownJid, pushName: null, phone, text: ownerReply, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });
            }

          } else {
            // Groups or no owner routing — reply directly
            const finalText = markdownToWhatsApp(response);
            const sentKey = await sendOrEdit(sender, finalText);
            console.log(`[gateway] Replied to ${pushName}`);
            dbSaveMessage({ id: sentKey?.id || randomUUID(), jid: sender, senderJid: ownJid, pushName: null, phone, text: response, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });
          }
        }

        // --- Message Store: mark inbound message as processed ---
        dbMarkProcessed(msg.key.id, 1);

      } catch (err) {
        console.error(`[gateway] Forward/reply failed:`, err.message);
        // Message stays processed=0 in DB — catch-up sweep will retry later
      }
    }
  });

  // -------------------------------------------------------------------------
  // Fase 3.2 — Option A: Hook messages.update for failed decryptions
  // -------------------------------------------------------------------------
  sock.ev.on('messages.update', (updates) => {
    for (const update of updates) {
      const key = update.key;
      const updateData = update.update || {};

      // stub 39 = CIPHERTEXT in Baileys' numeric enum (failed to decrypt)
      const stub = updateData.messageStubType;
      const isDecryptionError = stub === 39
        || updateData.status === 'ERROR' || updateData.status === 5;

      if (isDecryptionError) {
        const jid = key?.remoteJid || 'unknown';
        const msgId = key?.id || 'unknown';
        const retryKey = getDecryptRetryKey(jid, msgId);

        let entry = decryptRetryMap.get(retryKey);
        if (!entry) {
          entry = { count: 0, expireTimer: null, firstSeen: Date.now() };
          decryptRetryMap.set(retryKey, entry);

          // Save placeholder in DB on first occurrence
          dbSaveMessage({
            id: msgId,
            jid,
            senderJid: key?.participant || null,
            pushName: null,
            phone: null,
            text: '[DECRYPTION_FAILED — message could not be read]',
            direction: 'inbound',
            timestamp: Date.now(),
            processed: 0,
            rawType: 'decryption_error',
          });
        }

        entry.count += 1;
        console.warn(`[gateway][decrypt-retry] Decryption failure #${entry.count}/${DECRYPT_RETRY_MAX} — jid: ${jid}, msgId: ${msgId}, stub: ${stub || 'none'}`);

        if (entry.count >= DECRYPT_RETRY_MAX) {
          console.error(`[gateway][decrypt-retry] All ${DECRYPT_RETRY_MAX} retries exhausted for ${msgId} from ${jid}`);
          dbIncrRetryOrFail(msgId, DECRYPT_RETRY_MAX);

          const contactName = jid.replace(/@.*/, '');
          const timestamp = new Date().toISOString();
          const notifyText = [
            `⚠️ Unreadable message from ${contactName}`,
            `Time: ${timestamp}`,
            `ID: ${msgId}`,
            ``,
            `Message could not be decrypted after ${DECRYPT_RETRY_MAX} attempts.`,
            `Hint: ask the contact to resend the message.`,
          ].join('\n');

          forwardToLibreFang(
            notifyText,
            '[SYSTEM:decryption_failure]',
            contactName,
            'System',
            true,
            [],
          ).catch(err => console.error(`[gateway][decrypt-retry] Failed to send fallback notification:`, err.message));

          cleanupDecryptRetry(retryKey);
        } else {
          // Reset expire timer — clean up if no further updates arrive
          if (entry.expireTimer) clearTimeout(entry.expireTimer);
          entry.expireTimer = setTimeout(() => cleanupDecryptRetry(retryKey), DECRYPT_RETRY_EXPIRE_MS);
        }
      }
    }
  });

  // -------------------------------------------------------------------------
  // Fase 3.2 — Option C: Gap detection — warn if active chat goes silent
  // -------------------------------------------------------------------------
  const GAP_DETECTION_INTERVAL_MS = 10 * 60 * 1000;  // check every 10 min
  const GAP_THRESHOLD_MS = 30 * 60 * 1000;            // 30 min silence = warning

  const gapDetectionTimer = setInterval(() => {
    if (connStatus !== 'connected') return;
    const allLastSeen = stmtGetLastSeen.all();
    const now = Date.now();
    for (const row of allLastSeen) {
      // Only check JIDs that had recent activity (within last 2 hours)
      if (now - row.last_timestamp > 2 * 60 * 60 * 1000) continue;
      const gap = now - row.last_timestamp;
      if (gap > GAP_THRESHOLD_MS) {
        // Check if there's an active conversation for this JID (only warn for active ones)
        if (activeConversations.has(row.jid)) {
          console.warn(`[gateway][gap-detect] No messages from ${row.jid} for ${Math.round(gap / 60000)}min — possible message loss`);
        }
      }
    }
  }, GAP_DETECTION_INTERVAL_MS);

  // Clean up interval on socket close to prevent leaks on reconnect
  sock.ev.on('connection.update', (update) => {
    if (update.connection === 'close') {
      clearInterval(gapDetectionTimer);
    }
  });

  } finally {
    isConnecting = false;
  }
}

// ---------------------------------------------------------------------------
// Bug fix: Non-text media descriptor — don't silently drop media messages
// ---------------------------------------------------------------------------
function getMediaDescriptor(innerMsg, senderName) {
  if (innerMsg.imageMessage) {
    return `[Photo from ${senderName}]`;
  }
  if (innerMsg.videoMessage) {
    return `[Video from ${senderName}]`;
  }
  if (innerMsg.audioMessage) {
    const ptt = innerMsg.audioMessage.ptt;
    return ptt ? `[Voice message from ${senderName}]` : `[Audio from ${senderName}]`;
  }
  if (innerMsg.stickerMessage) {
    return `[Sticker from ${senderName}]`;
  }
  if (innerMsg.locationMessage || innerMsg.liveLocationMessage) {
    const loc = innerMsg.locationMessage || innerMsg.liveLocationMessage;
    return `[Location from ${senderName}: ${loc.degreesLatitude}, ${loc.degreesLongitude}]`;
  }
  if (innerMsg.contactMessage || innerMsg.contactsArrayMessage) {
    return `[Contact card from ${senderName}]`;
  }
  if (innerMsg.documentMessage) {
    const fileName = innerMsg.documentMessage.fileName || 'unknown';
    return `[Document from ${senderName}: ${fileName}]`;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Media processing: download from WhatsApp, upload to LibreFang
// ---------------------------------------------------------------------------
const MAX_MEDIA_SIZE = 50 * 1024 * 1024; // 50MB limit
const MEDIA_DOWNLOAD_TIMEOUT = 30_000;   // 30 seconds

// Cached Baileys downloadMediaMessage function (loaded on first use)
let _downloadMediaMessage = null;

async function getDownloadMediaFn() {
  if (!_downloadMediaMessage) {
    const baileys = await import('@whiskeysockets/baileys');
    _downloadMediaMessage = baileys.downloadMediaMessage;
  }
  return _downloadMediaMessage;
}

/**
 * Detect which media type key is present in the message.
 * Returns { type, msg } where msg is the inner media message object,
 * or null if no downloadable media is found.
 */
function getDownloadableMedia(innerMsg) {
  if (innerMsg.imageMessage)    return { type: 'imageMessage',    msg: innerMsg.imageMessage };
  if (innerMsg.videoMessage)    return { type: 'videoMessage',    msg: innerMsg.videoMessage };
  if (innerMsg.audioMessage)    return { type: 'audioMessage',    msg: innerMsg.audioMessage };
  if (innerMsg.stickerMessage)  return { type: 'stickerMessage',  msg: innerMsg.stickerMessage };
  if (innerMsg.documentMessage) return { type: 'documentMessage', msg: innerMsg.documentMessage };
  if (innerMsg.documentWithCaptionMessage?.message?.documentMessage) {
    return { type: 'documentMessage', msg: innerMsg.documentWithCaptionMessage.message.documentMessage };
  }
  return null;
}

/**
 * Determine MIME type for a media message.
 */
function getMediaMimeType(mediaType, mediaMsg) {
  // Most Baileys media objects carry a `mimetype` field
  if (mediaMsg.mimetype) return mediaMsg.mimetype;
  // Fallbacks by type
  const defaults = {
    imageMessage: 'image/jpeg',
    videoMessage: 'video/mp4',
    audioMessage: 'audio/ogg; codecs=opus',
    stickerMessage: 'image/webp',
    documentMessage: 'application/octet-stream',
  };
  return defaults[mediaType] || 'application/octet-stream';
}

/**
 * Determine a human-readable filename for a media message.
 */
function getMediaFilename(mediaType, mediaMsg) {
  if (mediaMsg.fileName) return mediaMsg.fileName;
  const extensions = {
    'image/jpeg': '.jpg', 'image/png': '.png', 'image/webp': '.webp',
    'video/mp4': '.mp4', 'audio/ogg; codecs=opus': '.ogg', 'audio/mpeg': '.mp3',
    'audio/ogg': '.ogg', 'application/pdf': '.pdf',
  };
  const mime = getMediaMimeType(mediaType, mediaMsg);
  const ext = extensions[mime] || '';
  const prefixes = {
    imageMessage: 'photo', videoMessage: 'video', audioMessage: 'audio',
    stickerMessage: 'sticker', documentMessage: 'document',
  };
  return (prefixes[mediaType] || 'file') + ext;
}

/**
 * Download media from a WhatsApp message with retry and timeout.
 * Returns a Buffer or throws on failure.
 */
async function downloadMedia(fullMsg) {
  const downloadFn = await getDownloadMediaFn();

  async function attempt() {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Media download timeout')), MEDIA_DOWNLOAD_TIMEOUT);
      downloadFn(fullMsg, 'buffer', {})
        .then(buf => { clearTimeout(timer); resolve(buf); })
        .catch(err => { clearTimeout(timer); reject(err); });
    });
  }

  try {
    return await attempt();
  } catch (firstErr) {
    // Retry once after 2 seconds
    console.warn(`[gateway] Media download failed (attempt 1): ${firstErr.message} — retrying in 2s`);
    await new Promise(r => setTimeout(r, 2000));
    return await attempt();
  }
}

/**
 * Upload a buffer to LibreFang via POST /api/agents/{id}/upload.
 * Returns { file_id, filename, content_type, size, transcription? } or throws.
 */
async function uploadToLibreFang(agentId, buffer, contentType, filename) {
  async function attempt() {
    return new Promise((resolve, reject) => {
      const url = new URL(`${LIBREFANG_URL}/api/agents/${encodeURIComponent(agentId)}/upload`);
      const req = http.request(
        {
          hostname: url.hostname,
          port: url.port || 4545,
          path: url.pathname,
          method: 'POST',
          headers: {
            'Content-Type': contentType,
            'X-Filename': filename,
            'Content-Length': buffer.length,
          },
          timeout: 60_000,
        },
        (res) => {
          let body = '';
          res.on('data', chunk => body += chunk);
          res.on('end', () => {
            if (res.statusCode >= 400) {
              return reject(new Error(`Upload failed (${res.statusCode}): ${body}`));
            }
            try {
              resolve(JSON.parse(body));
            } catch (e) {
              reject(new Error(`Upload response parse error: ${e.message}`));
            }
          });
        }
      );
      req.on('error', reject);
      req.on('timeout', () => { req.destroy(); reject(new Error('Upload timeout')); });
      req.write(buffer);
      req.end();
    });
  }

  try {
    return await attempt();
  } catch (firstErr) {
    // Retry once
    console.warn(`[gateway] Upload failed (attempt 1): ${firstErr.message} — retrying`);
    await new Promise(r => setTimeout(r, 1000));
    return await attempt();
  }
}

/**
 * Process a media message: download from WhatsApp, upload to LibreFang.
 * Returns { attachment, transcription? } on success, or null on failure.
 * On failure, logs the error (caller should fall back to text descriptor).
 */
async function processMediaMessage(fullMsg, innerMsg, agentId) {
  const media = getDownloadableMedia(innerMsg);
  if (!media) return null;

  const mimeType = getMediaMimeType(media.type, media.msg);
  const filename = getMediaFilename(media.type, media.msg);

  try {
    const buffer = await downloadMedia(fullMsg);

    // Size check
    if (buffer.length > MAX_MEDIA_SIZE) {
      console.warn(`[gateway] Media too large: ${(buffer.length / 1024 / 1024).toFixed(1)}MB > ${MAX_MEDIA_SIZE / 1024 / 1024}MB`);
      return { fallbackText: `[File too large: ${(buffer.length / 1024 / 1024).toFixed(0)}MB, limit ${MAX_MEDIA_SIZE / 1024 / 1024}MB]` };
    }

    const startTime = Date.now();
    const uploadResult = await uploadToLibreFang(agentId, buffer, mimeType, filename);
    const elapsed = Date.now() - startTime;

    console.log(`[gateway] Media processed: ${filename} (${mimeType}, ${(buffer.length / 1024).toFixed(0)}KB, upload ${elapsed}ms) → file_id=${uploadResult.file_id}`);

    return {
      attachment: {
        file_id: uploadResult.file_id,
        filename: uploadResult.filename || filename,
        content_type: uploadResult.content_type || mimeType,
      },
      transcription: uploadResult.transcription || null,
    };
  } catch (err) {
    console.error(`[gateway] Media processing failed for ${filename}: ${err.message}`);
    return null; // Caller will fall back to text descriptor
  }
}

// ---------------------------------------------------------------------------
// Detect whether an owner message expresses relay intent.
//
// Before this guard, `buildRelaySystemInstruction` was injected for every
// owner turn whenever any stranger conversation was active — which forced
// the model to interpret neutral owner-to-bot messages ("saludos", "hola",
// "come stai?") as requests to relay a reply to the last stranger. Result
// observed in production: owner writing "saludos" to the bot triggered a
// RELAY_TO_STRANGER to an unrelated namesake contact.
//
// Only inject the relay instruction when the owner's message expresses an
// explicit delegated-speech intent. Everything else is treated as owner
// talking directly to the agent.
// Regex compiled once at module load from the configured language
// packs in `lib/intent_patterns.js`. Adding a locale is a file-level
// change; adding the code to the config toggles it on.
const RELAY_INTENT_RE = require('./lib/intent_patterns').compileIntentRegex(
  tomlConfig.relay_intent_languages,
);

function ownerIntentsRelay(text) {
  const t = (text || '').trim().toLowerCase();
  if (!t) return false;
  if (t.startsWith('/relay') || t.startsWith('/reply')) return true;
  if (/(^|\s)@[\w.+-]+/.test(t)) return true;
  return RELAY_INTENT_RE.test(t);
}

// ---------------------------------------------------------------------------
// Build relay system instruction (Step E — separate from user text)
// ---------------------------------------------------------------------------
function buildRelaySystemInstruction() {
  return [
    '[SYSTEM_INSTRUCTION_WHATSAPP_RELAY]',
    'You are acting as a bridge between the owner and external contacts.',
    'When the owner wants to reply to a stranger, you MUST:',
    '1. Determine which stranger the owner is addressing (from the active conversations list above)',
    '2. Reformulate the message appropriately (never forward the raw owner message)',
    '3. Wrap the outgoing message in this exact format:',
    '[RELAY_TO_STRANGER]{"jid":"<stranger_jid>","message":"<your reformulated message>"}[/RELAY_TO_STRANGER]',
    '',
    'RULES:',
    '- The "jid" MUST be one from the [ACTIVE_STRANGER_CONVERSATIONS] list',
    '- The "message" MUST be a reformulated, polished version — never copy the owner\'s raw words',
    '- If the intended recipient is ambiguous, ask the owner to clarify instead of guessing',
    '- If the owner is talking to you (the agent) and NOT replying to a stranger, respond normally without any relay block',
    '- You can include both a relay block AND a confirmation message to the owner in the same response',
    '[/SYSTEM_INSTRUCTION_WHATSAPP_RELAY]',
    '',
  ].join('\n');
}

// ---------------------------------------------------------------------------
// Forward incoming message to LibreFang API, return agent response
// ---------------------------------------------------------------------------
const MAX_FORWARD_RETRIES = 1;

async function forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, attachments, { isGroup = false, wasMentioned = false, chatJid = '', groupParticipants = [], onOwnerNotice = null } = {}, retryCount = 0) {
  // CS-01: fail-fast — refuse to forward with an empty chatJid. A bare
  // `whatsapp` channel loses per-conversation session isolation; the kernel
  // would merge unrelated chats into the same session.
  if (!chatJid) {
    const err = new Error(`[gateway] chatJid empty — refusing to forward to bare whatsapp channel (phone=${phone} pushName=${pushName} isGroup=${isGroup})`);
    err.code = 'CHATJID_EMPTY';
    console.error(err.message);
    throw err;
  }

  // Resolve agent UUID if not cached (or if invalidated on reconnect)
  if (!cachedAgentId) {
    try {
      await resolveAgentId();
    } catch (err) {
      console.error(`[gateway] Agent resolution failed: ${err.message}`);
      throw err;
    }
  }

  const fullMessage = systemPrefix ? systemPrefix + text : text;

  // Per-conversation session isolation: include chat JID in channel_type
  // so the kernel creates separate sessions for each WhatsApp conversation.
  // CS-01: chatJid has already been validated non-empty at function entry.
  // Phase 3 §B — centralized in channelTypeForChat for single-sourcing.
  const channelType = channelTypeForChat(chatJid);

  // Phase 3 §B (EB-02) — single structured log per dispatch; allows
  // reconstructing (agent, peer, chat) tuple from logs alone. Retry recursion
  // re-enters this function, which is the desired diagnostic behavior.
  if (DISPATCH_LOG_VERBOSE) {
    console.log(JSON.stringify({
      event: 'forward_dispatch',
      session_key: buildSessionKey(cachedAgentId, phone, chatJid),
      channel_type: channelType,
      phone,
      push_name: pushName,
      is_group: !!isGroup,
      was_mentioned: !!wasMentioned,
    }));
  }

  const payload = {
    message: fullMessage,
    channel_type: channelType,
    sender_id: phone,
    sender_name: pushName,
    is_group: isGroup,
    was_mentioned: wasMentioned,
  };

  // Include attachments if present
  if (attachments && attachments.length > 0) {
    payload.attachments = attachments;
  }

  // Phase 2 §C — forward the group participant roster so the kernel's
  // addressee guard (`is_addressed_to_other_participant`) can fire.
  // Empty for DMs and for the catch-up path (no live `sock` to query).
  if (isGroup && Array.isArray(groupParticipants) && groupParticipants.length > 0) {
    payload.group_participants = groupParticipants;
  }

  const payloadStr = JSON.stringify(payload);

  return new Promise((resolve, reject) => {
    const url = new URL(`${LIBREFANG_URL}/api/agents/${encodeURIComponent(cachedAgentId)}/message`);

    const req = http.request(
      {
        hostname: url.hostname,
        port: url.port || 4545,
        path: url.pathname,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(payloadStr),
        },
        timeout: 120_000, // LLM calls can be slow
      },
      (res) => {
        let body = '';
        res.on('data', (chunk) => (body += chunk));
        res.on('end', () => {
          // If the agent UUID became stale (404), invalidate cache and retry once
          if (res.statusCode === 404) {
            if (retryCount < MAX_FORWARD_RETRIES) {
              console.log('[gateway] Agent UUID stale (404), re-resolving...');
              cachedAgentId = null;
              resolveAgentId()
                .then(() => forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, attachments, { isGroup, wasMentioned, chatJid }, retryCount + 1))
                .then(resolve)
                .catch(reject);
              return;
            }
            console.error('[gateway] Agent UUID still 404 after retry, giving up');
            return reject(new Error('Agent not found after retry'));
          }

          try {
            const data = JSON.parse(body);
            // §A — surface owner_notice envelope field (BC-02: absent on
            // older daemon builds, then we just behave like before).
            if (OWNER_CHANNEL_ENABLED && data.owner_notice && typeof onOwnerNotice === 'function') {
              try {
                onOwnerNotice(data.owner_notice);
              } catch (e) {
                console.warn(`[gateway] onOwnerNotice handler threw: ${e.message}`);
              }
            }
            // Silent completion — agent intentionally chose not to reply (NO_REPLY)
            if (data.silent) {
              resolve('');
              return;
            }
            // The /api/agents/{id}/message endpoint returns { response: "..." }
            const responseText = data.response || data.message || data.text || '';
            // Scrub NO_REPLY wherever it appears (isolated, trailing, or
            // glued to an emoji / punctuation without a separator).
            resolve(stripNoReply(responseText));
          } catch {
            // Non-JSON fallback — still scrub NO_REPLY for the same reason.
            resolve(stripNoReply(body || ''));
          }
        });
      },
    );

    req.on('error', reject);
    req.on('timeout', () => {
      req.destroy();
      reject(new Error('LibreFang API timeout'));
    });
    req.write(payloadStr);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Streaming forward — SSE version with progressive WhatsApp message edits
// ---------------------------------------------------------------------------

/** Minimum interval (ms) between WhatsApp message edits during streaming. */
const STREAMING_EDIT_INTERVAL_MS = 2000;

/**
 * Forward a message to LibreFang using the SSE streaming endpoint.
 * Calls `onProgress(accumulatedText)` periodically so the caller can
 * edit the WhatsApp message in-place.  Returns the complete response text.
 *
 * Falls back to the non-streaming `forwardToLibreFang()` on any SSE error.
 *
 * @param {string} text
 * @param {string} systemPrefix
 * @param {string} phone
 * @param {string} pushName
 * @param {boolean} isOwner
 * @param {object[]} attachments
 * @param {(text: string) => Promise<void>} onProgress
 * @returns {Promise<string>} complete response
 */
async function forwardToLibreFangStreaming(text, systemPrefix, phone, pushName, isOwner, attachments, onProgress, chatJid = '', { isGroup = false, wasMentioned = false, groupParticipants = [], onOwnerNotice = null } = {}) {
  // CS-01: fail-fast — refuse to forward with an empty chatJid (same
  // rationale as `forwardToLibreFang`). Keeps streaming parity.
  if (!chatJid) {
    const err = new Error(`[gateway] chatJid empty — refusing to forward to bare whatsapp channel (phone=${phone} pushName=${pushName} isGroup=${isGroup})`);
    err.code = 'CHATJID_EMPTY';
    console.error(err.message);
    throw err;
  }

  // Resolve agent UUID if not cached
  if (!cachedAgentId) {
    try {
      await resolveAgentId();
    } catch (err) {
      console.error(`[gateway] Agent resolution failed: ${err.message}`);
      throw err;
    }
  }

  const fullMessage = systemPrefix ? systemPrefix + text : text;

  // CS-01: chatJid has already been validated non-empty at function entry.
  // Phase 3 §B — centralized in channelTypeForChat for single-sourcing.
  const channelType = channelTypeForChat(chatJid);

  // Phase 3 §B (EB-02) — streaming-path dispatch log parity.
  if (DISPATCH_LOG_VERBOSE) {
    console.log(JSON.stringify({
      event: 'forward_dispatch',
      session_key: buildSessionKey(cachedAgentId, phone, chatJid),
      channel_type: channelType,
      phone,
      push_name: pushName,
      is_group: !!isGroup,
      was_mentioned: !!wasMentioned,
    }));
  }

  const payload = {
    message: fullMessage,
    channel_type: channelType,
    sender_id: phone,
    sender_name: pushName,
  };

  if (attachments && attachments.length > 0) {
    payload.attachments = attachments;
  }

  // Phase 2 §C — see forwardToLibreFang. Streaming path also forwards roster
  // for parity (kernel-side wiring still required to thread it into
  // ChannelMessage.metadata — tracked as a follow-up; gating tests at the
  // bridge layer cover the receive side).
  if (isGroup && Array.isArray(groupParticipants) && groupParticipants.length > 0) {
    payload.group_participants = groupParticipants;
  }

  const payloadStr = JSON.stringify(payload);

  return new Promise((resolve, reject) => {
    const url = new URL(`${LIBREFANG_URL}/api/agents/${encodeURIComponent(cachedAgentId)}/message/stream`);

    const req = http.request(
      {
        hostname: url.hostname,
        port: url.port || 4545,
        path: url.pathname,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(payloadStr),
          Accept: 'text/event-stream',
        },
        timeout: 180_000, // streaming can take longer
      },
      (res) => {
        // Non-200 or non-SSE → fall back to non-streaming
        const ct = res.headers['content-type'] || '';
        if (res.statusCode !== 200 || !ct.includes('text/event-stream')) {
          let body = '';
          res.on('data', (chunk) => (body += chunk));
          res.on('end', () => {
            console.warn(`[gateway] SSE endpoint returned ${res.statusCode}, falling back to non-streaming`);
            forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, attachments, { isGroup, wasMentioned, chatJid, onOwnerNotice })
              .then(resolve)
              .catch(reject);
          });
          return;
        }

        let accumulated = '';
        let lastEditTime = 0;
        let pendingEdit = null;
        let buf = '';

        res.setEncoding('utf8');
        res.on('data', (raw) => {
          buf += raw;
          // SSE frames are separated by double newlines
          const parts = buf.split('\n\n');
          buf = parts.pop(); // keep incomplete frame in buffer

          for (const frame of parts) {
            let eventType = 'message';
            let dataStr = '';
            for (const line of frame.split('\n')) {
              if (line.startsWith('event:')) eventType = line.slice(6).trim();
              else if (line.startsWith('data:')) dataStr += line.slice(5).trim();
            }
            if (!dataStr) continue;

            if (eventType === 'phase') {
              // Transient status (e.g. "still working..."). Show via onProgress
              // but DON'T add to accumulated — next real chunk overwrites it.
              try {
                const parsed = JSON.parse(dataStr);
                if (parsed.phase === 'long_running' && onProgress) {
                  // Don't send phase updates if accumulated text is NO_REPLY
                  const accTrim = accumulated.trim();
                  if (/(?:^|\n)\s*NO_REPLY\s*$/.test(accTrim) || accTrim === 'NO_REPLY') continue;
                  const status = parsed.detail || 'Still working...';
                  const display = accumulated ? accumulated + '\n\n[' + status + ']' : '[' + status + ']';
                  onProgress(display).catch(() => {});
                }
              } catch { /* ignore */ }
            } else if (eventType === 'owner_notice') {
              // §A — typed owner-side payload from notify_owner tool.
              // Forward to caller's onOwnerNotice handler unless flag disabled.
              if (OWNER_CHANNEL_ENABLED && typeof onOwnerNotice === 'function') {
                try {
                  const parsed = JSON.parse(dataStr);
                  if (parsed.text) onOwnerNotice(parsed.text);
                } catch (e) {
                  console.warn(`[gateway] owner_notice SSE parse failed: ${e.message}`);
                }
              }
            } else if (eventType === 'chunk') {
              try {
                const parsed = JSON.parse(dataStr);
                if (parsed.content) {
                  accumulated += parsed.content;

                  // Throttle edits
                  const now = Date.now();
                  if (onProgress && now - lastEditTime >= STREAMING_EDIT_INTERVAL_MS) {
                    lastEditTime = now;
                    // Fire-and-forget; don't block the stream
                    clearTimeout(pendingEdit);
                    pendingEdit = null;
                    onProgress(accumulated).catch((e) =>
                      console.warn(`[gateway] Streaming edit failed: ${e.message}`)
                    );
                  } else if (onProgress && !pendingEdit) {
                    // Schedule a trailing edit so the last chunk is always sent
                    const remaining = STREAMING_EDIT_INTERVAL_MS - (now - lastEditTime);
                    pendingEdit = setTimeout(() => {
                      pendingEdit = null;
                      lastEditTime = Date.now();
                      onProgress(accumulated).catch((e) =>
                        console.warn(`[gateway] Streaming trailing edit failed: ${e.message}`)
                      );
                    }, remaining);
                  }
                }
              } catch { /* ignore malformed JSON */ }
            }
            // 'done' event → stream is over, handled by res.on('end')
          }
        });

        res.on('end', () => {
          clearTimeout(pendingEdit);
          resolve(accumulated);
        });

        res.on('error', (err) => {
          clearTimeout(pendingEdit);
          console.warn(`[gateway] SSE stream error: ${err.message}, falling back`);
          forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, attachments, { isGroup, wasMentioned, chatJid, onOwnerNotice })
            .then(resolve)
            .catch(reject);
        });
      },
    );

    req.on('error', (err) => {
      console.warn(`[gateway] SSE request error: ${err.message}, falling back`);
      forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, attachments, { isGroup, wasMentioned, chatJid, onOwnerNotice })
        .then(resolve)
        .catch(reject);
    });
    req.on('timeout', () => {
      req.destroy();
      reject(new Error('LibreFang SSE timeout'));
    });
    req.write(payloadStr);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Catch-up Sweep: re-process unprocessed messages every 5 minutes (Fase 3.1)
// ---------------------------------------------------------------------------
const CATCHUP_INTERVAL_MS = 5 * 60 * 1000;  // 5 minutes
const CATCHUP_AGE_MS = 30_000;               // only messages older than 30s
const CATCHUP_MAX_RETRIES = 3;

async function runCatchUpSweep() {
  if (connStatus !== 'connected' || !sock) return;

  const cutoff = Date.now() - CATCHUP_AGE_MS;
  const unprocessed = dbGetUnprocessed(cutoff);
  if (unprocessed.length === 0) return;

  console.log(`[gateway][catchup] Found ${unprocessed.length} unprocessed message(s), attempting re-forward...`);

  for (const msg of unprocessed) {
    // Skip messages already at max retries (they'll be marked failed by dbIncrRetryOrFail)
    if (msg.retry_count >= CATCHUP_MAX_RETRIES) {
      dbIncrRetryOrFail(msg.id, CATCHUP_MAX_RETRIES);
      continue;
    }

    // CS-01 iter 2 guard: a journal row with null/empty jid has no meaningful
    // chat scope to replay to (orphan from pre-scoping baseline). Mark it
    // processed explicitly so the CS-01 throw inside forwardToLibreFang
    // doesn't land inside our catch-block and inflate retry_count up to
    // CATCHUP_MAX_RETRIES before eventually giving up.
    if (shouldSkipCatchupForMissingJid(msg)) {
      dbMarkProcessed(msg.id, 1);
      console.log(`[gateway][catchup] catchup_skip_no_jid id=${msg.id} — journal row has null jid, cannot scope replay`);
      continue;
    }

    try {
      // Ensure agent ID is resolved
      if (!cachedAgentId) await resolveAgentId();

      // Determine if sender is owner or stranger. Mirror the logic used in
      // `messages.upsert`: a LID JID is not a phone number, so accept either
      // a resolved phone-number JID or a known owner LID.
      const isLidMsgJid = isLidJid(msg.jid);
      const senderPnJid = msg.phone ? phoneToJid(msg.phone) : '';
      const isOwner = OWNER_JIDS.size > 0 && (
        (!isLidMsgJid && msg.jid && OWNER_JIDS.has(msg.jid)) ||
        (senderPnJid && OWNER_JIDS.has(senderPnJid)) ||
        (isLidMsgJid && ownerLidJids.has(msg.jid))
      );

      // Never re-forward group messages — we cannot tell if the bot was
      // mentioned, so replaying them violates group_policy and can leak
      // internal text (rate-limit errors, recovery prefixes) into groups.
      const isCatchupGroup = isGroupJid(msg.jid);
      if (isCatchupGroup) {
        dbMarkProcessed(msg.id, 1);
        console.log(`[gateway][catchup] Skipping group message ${msg.id} (${msg.jid}) — group catchup disabled`);
        continue;
      }

      // Simple re-forward: send the stored text to the agent without full context rebuild
      const prefix = isOwner ? '' : `[CATCHUP_REDELIVERY from ${msg.push_name || msg.phone || msg.jid}]\n`;
      const response = await forwardToLibreFang(prefix + (msg.text || ''), '', msg.phone || '', msg.push_name || '', isOwner, [], { isGroup: false, wasMentioned: false, chatJid: msg.jid || '' });

      // Mark as processed
      dbMarkProcessed(msg.id, 1);
      console.log(`[gateway][catchup] Re-forwarded message ${msg.id} from ${msg.push_name || msg.jid}`);

      // If there's a response, try to send it back (strangers and groups)
      if (response && !isOwner && msg.jid) {
        try {
          const formatted = markdownToWhatsApp(response);
          await sock.sendMessage(msg.jid, { text: formatted });
          if (ECHO_TRACKER_ENABLED) echoTracker.track(response);
          dbSaveMessage({ id: randomUUID(), jid: msg.jid, senderJid: ownJid, pushName: null, phone: msg.phone, text: response, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });
        } catch (sendErr) {
          console.warn(`[gateway][catchup] Could not send catch-up reply to ${msg.jid}: ${sendErr.message}`);
        }
      }
    } catch (err) {
      console.warn(`[gateway][catchup] Failed to re-forward message ${msg.id}: ${err.message}`);
      dbIncrRetryOrFail(msg.id, CATCHUP_MAX_RETRIES);
    }
  }
}

setInterval(runCatchUpSweep, CATCHUP_INTERVAL_MS);

// ---------------------------------------------------------------------------
// DB Cleanup: delete old processed/failed messages (Fase 4.1)
// ---------------------------------------------------------------------------
const CLEANUP_INTERVAL_MS = 24 * 60 * 60 * 1000;  // once per day
const CLEANUP_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;  // 7 days

function runDbCleanup() {
  const cutoff = Date.now() - CLEANUP_MAX_AGE_MS;
  const deleted = dbCleanupOld(cutoff);
  if (deleted > 0) {
    console.log(`[gateway][cleanup] Deleted ${deleted} old messages from DB`);
  }
}

// Run cleanup on startup (no-op if DB is fresh) and then daily
runDbCleanup();
setInterval(runDbCleanup, CLEANUP_INTERVAL_MS);

// ---------------------------------------------------------------------------
// Send a message via Baileys (called by LibreFang for outgoing)
// ---------------------------------------------------------------------------
async function sendMessage(to, text) {
  if (!sock || connStatus !== 'connected') {
    throw new Error('WhatsApp not connected');
  }

  // Preserve group JIDs (@g.us) as-is; normalize phone → JID for individuals
  const jid = phoneToJid(to);

  const formatted = markdownToWhatsApp(text);
  const sent = await sock.sendMessage(jid, { text: formatted });
  if (ECHO_TRACKER_ENABLED) echoTracker.track(text);
  // Save outbound message to DB (store formatted text to match what was delivered)
  dbSaveMessage({
    id: sent?.key?.id || randomUUID(),
    jid,
    senderJid: ownJid || null,
    pushName: null,
    phone: to,
    text: formatted,
    direction: 'outbound',
    timestamp: Date.now(),
    processed: 1,
    rawType: 'text',
  });
}

async function sendImage(to, imageUrl, caption) {
  if (!sock || connStatus !== 'connected') {
    throw new Error('WhatsApp not connected');
  }

  // Preserve group JIDs (@g.us) as-is; normalize phone → JID for individuals
  const jid = phoneToJid(to);

  // Fetch image into buffer (Baileys needs buffer or local file)
  const buffer = await new Promise((resolve, reject) => {
    const MAX_REDIRECTS = 5;
    const request = (url, redirectCount = 0) => {
      if (redirectCount > MAX_REDIRECTS) {
        return reject(new Error(`Too many redirects (max ${MAX_REDIRECTS})`));
      }
      const mod = url.startsWith('https') ? require('node:https') : require('node:http');
      mod.get(url, (resp) => {
        if (resp.statusCode >= 300 && resp.statusCode < 400 && resp.headers.location) {
          return request(resp.headers.location, redirectCount + 1);
        }
        if (resp.statusCode !== 200) {
          return reject(new Error(`Failed to fetch image: HTTP ${resp.statusCode}`));
        }
        const chunks = [];
        resp.on('data', (c) => chunks.push(c));
        resp.on('end', () => resolve(Buffer.concat(chunks)));
        resp.on('error', reject);
      }).on('error', reject);
    };
    request(imageUrl);
  });

  const imgMsg = { image: buffer };
  if (caption) imgMsg.caption = caption;

  const sent = await sock.sendMessage(jid, imgMsg);
  dbSaveMessage({
    id: sent?.key?.id || randomUUID(),
    jid,
    senderJid: ownJid || null,
    pushName: null,
    phone: to,
    text: caption || '[Image]',
    direction: 'outbound',
    timestamp: Date.now(),
    processed: 1,
    rawType: 'image',
  });
}

async function sendAudio(to, audioUrl, ptt = true) {
  if (!sock || connStatus !== 'connected') {
    throw new Error('WhatsApp not connected');
  }

  // Preserve group JIDs (@g.us) as-is; normalize phone → JID for individuals
  const jid = phoneToJid(to);

  // Fetch audio into buffer (Baileys needs buffer or local file)
  const buffer = await new Promise((resolve, reject) => {
    const MAX_REDIRECTS = 5;
    const request = (url, redirectCount = 0) => {
      if (redirectCount > MAX_REDIRECTS) {
        return reject(new Error(`Too many redirects (max ${MAX_REDIRECTS})`));
      }
      const mod = url.startsWith('https') ? require('node:https') : require('node:http');
      mod.get(url, (resp) => {
        if (resp.statusCode >= 300 && resp.statusCode < 400 && resp.headers.location) {
          return request(resp.headers.location, redirectCount + 1);
        }
        if (resp.statusCode !== 200) {
          return reject(new Error(`Failed to fetch audio: HTTP ${resp.statusCode}`));
        }
        const chunks = [];
        resp.on('data', (c) => chunks.push(c));
        resp.on('end', () => resolve(Buffer.concat(chunks)));
        resp.on('error', reject);
      }).on('error', reject);
    };
    request(audioUrl);
  });

  // ptt: true sends as a voice note (push-to-talk bubble); false sends as audio file
  const audioMsg = { audio: buffer, mimetype: 'audio/ogg; codecs=opus', ptt };

  const sent = await sock.sendMessage(jid, audioMsg);
  dbSaveMessage({
    id: sent?.key?.id || randomUUID(),
    jid,
    senderJid: ownJid || null,
    pushName: null,
    phone: to,
    text: ptt ? '[Voice message]' : '[Audio]',
    direction: 'outbound',
    timestamp: Date.now(),
    processed: 1,
    rawType: 'audio',
  });
}

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------
const MAX_BODY_SIZE = 64 * 1024;

function parseBody(req) {
  return new Promise((resolve, reject) => {
    let body = '';
    let size = 0;
    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > MAX_BODY_SIZE) {
        req.destroy();
        return reject(new Error('Request body too large'));
      }
      body += chunk;
    });
    req.on('end', () => {
      try {
        resolve(body ? JSON.parse(body) : {});
      } catch (e) {
        reject(new Error('Invalid JSON'));
      }
    });
    req.on('error', reject);
  });
}

const ALLOWED_ORIGIN_RE = /^(https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?|tauri:\/\/localhost|app:\/\/localhost)$/i;

function isAllowedOrigin(origin) {
  return Boolean(origin && ALLOWED_ORIGIN_RE.test(origin));
}

function buildCorsHeaders(origin) {
  if (!isAllowedOrigin(origin)) return {};
  return {
    'Access-Control-Allow-Origin': origin,
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type',
    'Vary': 'Origin',
  };
}

function jsonResponse(req, res, status, data) {
  const body = JSON.stringify(data);
  res.writeHead(status, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(body),
    ...buildCorsHeaders(req.headers.origin),
  });
  res.end(body);
}

const server = http.createServer(async (req, res) => {
  // CORS preflight
  if (req.method === 'OPTIONS') {
    res.writeHead(204, buildCorsHeaders(req.headers.origin));
    return res.end();
  }

  const url = new URL(req.url, `http://localhost:${PORT}`);
  const path = url.pathname;

  try {
    // POST /login/start — start Baileys connection, return QR
    if (req.method === 'POST' && path === '/login/start') {
      // If already connected, just return success
      if (connStatus === 'connected') {
        return jsonResponse(req, res, 200, {
          qr_data_url: '',
          session_id: sessionId,
          message: 'Already connected to WhatsApp',
          connected: true,
        });
      }

      // Start a new connection (resets any existing)
      await startConnection();

      // Wait briefly for QR to generate (Baileys emits it quickly)
      let waited = 0;
      while (!qrDataUrl && connStatus !== 'connected' && waited < 15_000) {
        await new Promise((r) => setTimeout(r, 300));
        waited += 300;
      }

      return jsonResponse(req, res, 200, {
        qr_data_url: qrDataUrl,
        session_id: sessionId,
        message: statusMessage,
        connected: connStatus === 'connected',
      });
    }

    // GET /login/status — poll for connection status
    if (req.method === 'GET' && path === '/login/status') {
      return jsonResponse(req, res, 200, {
        connected: connStatus === 'connected',
        message: statusMessage,
        expired: qrExpired,
      });
    }

    // POST /message/send — send outgoing message via Baileys
    if (req.method === 'POST' && path === '/message/send') {
      const body = await parseBody(req);
      const { to, text } = body;

      if (!to || !text) {
        return jsonResponse(req, res, 400, { error: 'Missing "to" or "text" field' });
      }

      await sendMessage(to, text);
      return jsonResponse(req, res, 200, { success: true, message: 'Sent' });
    }

    // POST /message/send-image — send image via URL
    if (req.method === 'POST' && path === '/message/send-image') {
      const body = await parseBody(req);
      const { to, image_url, caption } = body;

      if (!to || !image_url) {
        return jsonResponse(req, res, 400, { error: 'Missing "to" or "image_url" field' });
      }

      await sendImage(to, image_url, caption || '');
      return jsonResponse(req, res, 200, { success: true, message: 'Image sent' });
    }

    // POST /message/send-audio — send audio file or voice note via URL
    if (req.method === 'POST' && path === '/message/send-audio') {
      const body = await parseBody(req);
      const { to, audio_url, ptt } = body;

      if (!to || !audio_url) {
        return jsonResponse(req, res, 400, { error: 'Missing "to" or "audio_url" field' });
      }

      // ptt (push-to-talk) defaults to true — sends as voice note bubble
      await sendAudio(to, audio_url, ptt !== false);
      return jsonResponse(req, res, 200, { success: true, message: 'Audio sent' });
    }

    // GET /conversations — list active stranger conversations (Step B)
    if (req.method === 'GET' && path === '/conversations') {
      const conversations = [];
      for (const [jid, convo] of activeConversations) {
        conversations.push({
          jid,
          pushName: convo.pushName,
          phone: convo.phone,
          messageCount: convo.messageCount,
          lastActivity: convo.lastActivity,
          escalated: convo.escalated,
          lastMessage: convo.messages[convo.messages.length - 1] || null,
        });
      }
      return jsonResponse(req, res, 200, { conversations });
    }

    // GET /messages/unprocessed — messages that failed to forward (Fase 2.2)
    if (req.method === 'GET' && path === '/messages/unprocessed') {
      const rows = dbGetUnprocessed(Date.now());
      const unprocessed = rows.map(r => ({
        id: r.id,
        jid: r.jid,
        text: r.text,
        push_name: r.push_name,
        phone: r.phone,
        timestamp: r.timestamp,
        retry_count: r.retry_count,
        raw_type: r.raw_type,
      }));
      return jsonResponse(req, res, 200, { unprocessed });
    }

    // GET /messages/:jid — message history for a specific chat (Fase 2.1)
    if (req.method === 'GET' && path.startsWith('/messages/')) {
      const jid = decodeURIComponent(path.slice('/messages/'.length));
      if (!jid) {
        return jsonResponse(req, res, 400, { error: 'Missing JID in path' });
      }
      const limit = parseInt(url.searchParams.get('limit') || '20', 10);
      const since = parseInt(url.searchParams.get('since') || '0', 10);
      const rows = dbGetMessagesByJid(jid, Math.min(limit, 100), since);
      // Reverse to chronological order (query is DESC)
      rows.reverse();
      const messages = rows.map(r => ({
        id: r.id,
        text: r.text,
        direction: r.direction,
        push_name: r.push_name,
        timestamp: r.timestamp,
        processed: r.processed === 1,
        raw_type: r.raw_type,
      }));
      return jsonResponse(req, res, 200, { jid, messages });
    }

    // GET /health — health check
    if (req.method === 'GET' && path === '/health') {
      return jsonResponse(req, res, 200, {
        status: 'ok',
        connected: connStatus === 'connected',
        session_id: sessionId || null,
        active_conversations: activeConversations.size,
      });
    }

    // 404
    jsonResponse(req, res, 404, { error: 'Not found' });
  } catch (err) {
    console.error(`[gateway] ${req.method} ${path} error:`, err.message);
    jsonResponse(req, res, 500, { error: err.message });
  }
});

if (require.main === module) {
// Phase 3 §B — CS-01 regression guard. Fail fast if the chatJid-to-
// channel_type contract has silently degraded (e.g. future refactor
// collapses both chats to bare `whatsapp`). Runs before we accept any
// socket traffic.
{
  const _selfTest = runDispatchSelfTest();
  if (!_selfTest.ok) {
    console.error('[gateway] FATAL dispatch_self_test failed:', _selfTest.reason);
    process.exit(1);
  }
  console.log(JSON.stringify({ event: 'dispatch_self_test', ok: true }));
}

server.listen(PORT, '127.0.0.1', async () => {
  console.log(`[gateway] WhatsApp Web gateway listening on http://127.0.0.1:${PORT}`);
  console.log(`[gateway] LibreFang URL: ${LIBREFANG_URL}`);
  console.log(`[gateway] Default agent: ${DEFAULT_AGENT} (name: ${AGENT_NAME})`);
  console.log(`[gateway] Conversation TTL: ${CONVERSATION_TTL_HOURS}h`);

  // Auto-connect from existing credentials on startup
  const fs = require('node:fs');
  const authPath = require('node:path').join(__dirname, 'auth_store', 'creds.json');
  if (fs.existsSync(authPath)) {
    console.log('[gateway] Found existing auth — auto-connecting...');
    try {
      await startConnection();
    } catch (err) {
      console.error('[gateway] Auto-connect failed:', err.message);
      // Schedule a retry after a short delay — the daemon may still be booting
      console.log('[gateway] Will retry auto-connect in 10s...');
      setTimeout(async () => {
        try {
          await startConnection();
        } catch (retryErr) {
          console.error('[gateway] Auto-connect retry failed:', retryErr.message);
        }
      }, 10_000);
    }
  } else {
    console.log('[gateway] No auth found — waiting for POST /login/start to begin QR flow...');
  }
});

// Graceful shutdown
let shuttingDown = false;
function gracefulShutdown(signal) {
  // Re-entry guard: if SIGINT arrives during the SIGTERM 10s window (or
  // vice versa) we'd otherwise invoke cleanupSocket() / server.close()
  // twice — the second server.close() throws ERR_SERVER_NOT_RUNNING.
  if (shuttingDown) {
    console.log(`[gateway] Already shutting down, ignoring ${signal}`);
    return;
  }
  shuttingDown = true;
  console.log(`\n[gateway] Received ${signal}, shutting down...`);

  // Force exit after 10 seconds no matter what
  const forceExitTimer = setTimeout(() => {
    console.error('[gateway] Graceful shutdown timed out, force exiting');
    process.exit(1);
  }, 10_000);
  forceExitTimer.unref();

  // Tear down Baileys socket properly (fire-and-forget, we don't await).
  // Log the error message if teardown fails — a silent catch would hide a
  // broken Baileys shutdown in production.
  cleanupSocket().catch(e =>
    console.warn('[gateway] cleanupSocket error:', e?.message || e),
  );

  // Close HTTP server — forcibly drain all existing connections (Node.js 18.2+)
  if (server.closeAllConnections) {
    server.closeAllConnections();
  }
  server.close(() => {
    clearTimeout(forceExitTimer);
    console.log('[gateway] Shutdown complete');
    process.exit(0);
  });
}

process.on('SIGINT', () => gracefulShutdown('SIGINT'));
process.on('SIGTERM', () => gracefulShutdown('SIGTERM'));
} // end if (require.main === module)

// Export for testing
module.exports = {
  markdownToWhatsApp,
  extractNotifyOwner,
  extractRelayCommands,
  ownerIntentsRelay,
  buildConversationsContext,
  isRateLimited,
  buildCorsHeaders,
  isAllowedOrigin,
  parseBody,
  MAX_BODY_SIZE,
  forwardToLibreFang,
  forwardToLibreFangStreaming,
  shouldSkipCatchupForMissingJid,
  resolveLidProactively,
  checkHeartbeat,
  computeBackoffDelay,
  isSilentResponse,
  stripNoReply,
  createHoldbackAccumulator,
  SILENT_HOLDBACK_MIN_CHARS,
  getGroupParticipants,
  invalidateGroupRoster,
  groupMetadataCache,
  GROUP_METADATA_TTL_MS,
  // Phase 3 §A — echo tracker handle (testing + introspection)
  echoTracker,
  ECHO_TRACKER_ENABLED,
  EchoTracker,
  // Phase 4 §B (ID-02) — persisted LID cache (testing + introspection)
  lidToPnJid,
  lidMapSet,
  db,
  LID_PERSIST_ENABLED,
  normalizeBaseJid,
  sessionRecoveryMap,
  SESSION_RECOVERY_COOLDOWN_MS,
  SESSION_RECOVERY_MAX_ATTEMPTS,
  runDispatchSelfTest,
  channelTypeForChat,
  buildSessionKey,
};
