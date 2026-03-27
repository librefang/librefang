#!/usr/bin/env node
'use strict';

const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { randomUUID } = require('node:crypto');
const toml = require('toml');

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
  const defaults = { default_agent: 'assistant', owner_numbers: [], conversation_ttl_hours: 24 };
  try {
    const content = fs.readFileSync(configPath, 'utf8');
    const parsed = toml.parse(content);
    const wa = parsed?.channels?.whatsapp || {};
    const cfg = {
      default_agent: wa.default_agent || defaults.default_agent,
      owner_numbers: Array.isArray(wa.owner_numbers) ? wa.owner_numbers : defaults.owner_numbers,
      conversation_ttl_hours: parseInt(wa.conversation_ttl_hours, 10) || defaults.conversation_ttl_hours,
    };
    console.log(`[gateway] Read config from ${configPath}: default_agent="${cfg.default_agent}", owner_numbers=${JSON.stringify(cfg.owner_numbers)}, conversation_ttl_hours=${cfg.conversation_ttl_hours}`);
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
const OWNER_JIDS = new Set(
  OWNER_NUMBERS.map(n => n.replace(/^\+/, '') + '@s.whatsapp.net')
);
// Primary owner JID for unsolicited/scheduled messages only
const OWNER_JID = OWNER_JIDS.size > 0 ? [...OWNER_JIDS][0] : '';

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
const MAX_RECONNECT_DELAY = 60_000;
const MAX_RECONNECT_ATTEMPTS = 10;

// Cached agent UUID — resolved from DEFAULT_AGENT name on first use
let cachedAgentId = null;

// The user's own JID (set after connection opens) for self-chat detection
let ownJid = null;

// ---------------------------------------------------------------------------
// Markdown → WhatsApp formatting conversion
// ---------------------------------------------------------------------------
// LLM responses use standard Markdown but WhatsApp has its own formatting
// syntax. Convert the most common patterns so messages render correctly.
function markdownToWhatsApp(text) {
  if (!text) return text;
  // Bold: **text** or __text__ → placeholder, then restore as *text* (WhatsApp bold)
  // Using placeholder avoids the italic regex from re-matching converted bold.
  text = text.replace(/\*\*(.+?)\*\*/g, '\x01BOLD$1BOLD\x01');
  text = text.replace(/__(.+?)__/g, '\x01BOLD$1BOLD\x01');
  // Italic: *text* → _text_ (WhatsApp italic) — only matches remaining single-star
  text = text.replace(/(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)/g, '_$1_');
  // Restore bold placeholders → *text* (WhatsApp bold)
  text = text.replace(/\x01BOLD(.+?)BOLD\x01/g, '*$1*');
  // Strikethrough: ~~text~~ → ~text~
  text = text.replace(/~~(.+?)~~/g, '~$1~');
  // Inline code: `text` → ```text``` (WhatsApp monospace)
  text = text.replace(/(?<!`)(`{1})(?!`)(.+?)(?<!`)\1(?!`)/g, '```$2```');
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
const recentMessageIds = new Map(); // Map<msgId, timestamp>
const DEDUP_WINDOW_MS = 60_000; // 1 minute

function isDuplicate(msgId) {
  if (!msgId) return false;
  if (recentMessageIds.has(msgId)) return true;
  recentMessageIds.set(msgId, Date.now());
  return false;
}

// Cleanup dedup cache every 2 minutes
setInterval(() => {
  const now = Date.now();
  for (const [id, ts] of recentMessageIds) {
    if (now - ts > DEDUP_WINDOW_MS) recentMessageIds.delete(id);
  }
}, 2 * 60 * 1000);

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
    'This person is NOT the owner. They are an external contact.',
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
  const cleanedText = responseText.replace(NOTIFY_OWNER_RE, '').trim();
  return { notifications, cleanedText };
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
    // printQRInTerminal removed (deprecated in Baileys v6+)
    browser: ['LibreFang', 'Desktop', '1.0.0'],
  });

  // Save credentials whenever they update
  sock.ev.on('creds.update', saveCreds);

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
        // All other disconnect reasons are treated as recoverable
        reconnectAttempts += 1;
        if (reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
          console.error(`[gateway] Max reconnection attempts (${MAX_RECONNECT_ATTEMPTS}) reached. Manual restart required.`);
          connStatus = 'disconnected';
          statusMessage = `Max reconnection attempts (${MAX_RECONNECT_ATTEMPTS}) reached. Manual restart required.`;
        } else {
          const delay = Math.min(
            2000 * Math.pow(1.5, reconnectAttempts - 1),
            MAX_RECONNECT_DELAY,
          );
          console.log(
            `[gateway] Reconnecting in ${Math.round(delay / 1000)}s (attempt ${reconnectAttempts}/${MAX_RECONNECT_ATTEMPTS})...`,
          );
          connStatus = 'disconnected';
          statusMessage = `Reconnecting (attempt ${reconnectAttempts}/${MAX_RECONNECT_ATTEMPTS})...`;
          setTimeout(() => startConnection(), delay);
        }
      }
    }

    if (connection === 'open') {
      connStatus = 'connected';
      qrExpired = false;
      qrDataUrl = '';
      reconnectAttempts = 0;
      statusMessage = 'Connected to WhatsApp';
      console.log('[gateway] Connected to WhatsApp!');

      // Capture own JID for self-chat detection
      if (sock?.user?.id) {
        // Baileys user.id is like "1234567890:42@s.whatsapp.net" — normalize
        ownJid = sock.user.id.replace(/:.*@/, '@');
        console.log(`[gateway] Own JID: ${ownJid}`);
      }

      // Invalidate cached agent UUID on reconnect — the daemon may have
      // restarted and agents may have new UUIDs.
      cachedAgentId = null;
    }
  });

  // Incoming messages → forward to LibreFang
  sock.ev.on('messages.upsert', async ({ messages, type }) => {
    if (type !== 'notify') return;

    for (const msg of messages) {
      // Skip status broadcasts
      if (msg.key.remoteJid === 'status@broadcast') continue;

      // Deduplication: skip if we've already processed this message ID
      if (isDuplicate(msg.key.id)) {
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
        innerMsg._overrideMediaText = `[Contatti condivisi: ${parsed.join(', ')}]`;
      }

      // Skip if there's nothing to process
      if (!text && !downloadableMedia && !mediaDescriptor && !innerMsg._overrideMediaText) continue;

      // Extract real phone number
      const isLidJid = sender.endsWith('@lid');
      const senderPn = msg.key.senderPn || msg.key.participant || '';
      let phone;
      if (isLidJid && senderPn) {
        phone = '+' + senderPn.replace(/@.*$/, '');
      } else {
        phone = '+' + sender.replace(/@.*$/, '');
      }
      const pushName = msg.pushName || phone;

      // Determine sender type
      const isGroup = sender.endsWith('@g.us');
      const senderPnJid = senderPn ? senderPn.replace(/@.*$/, '') + '@s.whatsapp.net' : '';
      const isOwner = OWNER_JIDS.size > 0 && (OWNER_JIDS.has(sender) || (senderPnJid && OWNER_JIDS.has(senderPnJid)));
      const isStranger = !isGroup && OWNER_JIDS.size > 0 && !isOwner;

      // Rate limiting for strangers
      if (isStranger && isRateLimited(sender)) {
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
          messageToSend = messageText;
        } else if (isStranger) {
          const strangerContext = buildStrangerContext(pushName, phone, sender);
          messageToSend = strangerContext + messageText;
        } else if (isOwner && activeConversations.size > 0) {
          const context = buildConversationsContext();
          systemPrefix = buildRelaySystemInstruction();
          messageToSend = context + '\n\n[OWNER_MESSAGE]\n' + messageText;
        } else {
          messageToSend = messageText;
        }

        const response = await forwardToLibreFang(messageToSend, systemPrefix, phone, pushName, isOwner, attachments);

        if (response && sock) {
          if (isStranger) {
            // Step C: Agent response goes to STRANGER, not owner
            const { notifications, cleanedText } = extractNotifyOwner(response);

            // Send cleaned response to the stranger (format after tag extraction)
            if (cleanedText) {
              const formattedText = markdownToWhatsApp(cleanedText);
              const sentReply = await sock.sendMessage(sender, { text: formattedText });
              console.log(`[gateway] Replied to stranger ${pushName} (${phone})`);

              // Track outbound message
              trackMessage(sender, pushName, phone, cleanedText, 'outbound');
              // Save outbound to DB
              dbSaveMessage({ id: sentReply?.key?.id || randomUUID(), jid: sender, senderJid: ownJid, pushName: null, phone, text: cleanedText, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });
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
              // Bug fix: Reply to the SENDER's JID, not always OWNER_JID[0]
              ownerReply = markdownToWhatsApp(ownerReply);
              const sentOwner = await sock.sendMessage(sender, { text: ownerReply });
              console.log(`[gateway] Replied to owner (${sender})`);
              dbSaveMessage({ id: sentOwner?.key?.id || randomUUID(), jid: sender, senderJid: ownJid, pushName: null, phone, text: ownerReply, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });
            }

          } else {
            // Groups or no owner routing — reply directly
            const sentGroup = await sock.sendMessage(sender, { text: markdownToWhatsApp(response) });
            console.log(`[gateway] Replied to ${pushName}`);
            dbSaveMessage({ id: sentGroup?.key?.id || randomUUID(), jid: sender, senderJid: ownJid, pushName: null, phone, text: response, direction: 'outbound', timestamp: Date.now(), processed: 1, rawType: 'text' });
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
      // Baileys emits update.message with error info for decryption failures
      const key = update.key;
      const updateData = update.update || {};

      // Check for message retry / decryption error signals
      if (updateData.messageStubType || updateData.status === 'ERROR' || updateData.status === 5) {
        const jid = key?.remoteJid || 'unknown';
        const msgId = key?.id || 'unknown';
        console.warn(`[gateway][session-error] Possible decryption failure detected — jid: ${jid}, msgId: ${msgId}, stub: ${updateData.messageStubType || 'none'}, status: ${updateData.status || 'none'}`);

        // Save a placeholder in DB so the catch-up sweep can attempt recovery
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

async function forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, attachments, retryCount = 0) {
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

  const payload = {
    message: fullMessage,
    channel_type: 'whatsapp',
    sender_id: phone,
    sender_name: pushName,
  };

  // Include attachments if present
  if (attachments && attachments.length > 0) {
    payload.attachments = attachments;
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
                .then(() => forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, attachments, retryCount + 1))
                .then(resolve)
                .catch(reject);
              return;
            }
            console.error('[gateway] Agent UUID still 404 after retry, giving up');
            return reject(new Error('Agent not found after retry'));
          }

          try {
            const data = JSON.parse(body);
            // The /api/agents/{id}/message endpoint returns { response: "..." }
            resolve(data.response || data.message || data.text || '');
          } catch {
            resolve(body.trim() || '');
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

    try {
      // Ensure agent ID is resolved
      if (!cachedAgentId) await resolveAgentId();

      // Determine if sender is owner or stranger
      const senderPnJid = msg.phone ? msg.phone.replace(/^\+/, '') + '@s.whatsapp.net' : '';
      const isOwner = OWNER_JIDS.size > 0 && (OWNER_JIDS.has(msg.jid) || (senderPnJid && OWNER_JIDS.has(senderPnJid)));

      // Simple re-forward: send the stored text to the agent without full context rebuild
      const prefix = isOwner ? '' : `[CATCHUP_REDELIVERY from ${msg.push_name || msg.phone || msg.jid}]\n`;
      const response = await forwardToLibreFang(prefix + (msg.text || ''), '', msg.phone || '', msg.push_name || '', isOwner, []);

      // Mark as processed
      dbMarkProcessed(msg.id, 1);
      console.log(`[gateway][catchup] Re-forwarded message ${msg.id} from ${msg.push_name || msg.jid}`);

      // If there's a response and it's a stranger, try to send it back
      if (response && !isOwner && msg.jid && !msg.jid.endsWith('@g.us')) {
        try {
          const formatted = markdownToWhatsApp(response);
          await sock.sendMessage(msg.jid, { text: formatted });
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

  // Normalize phone → JID: "+1234567890" → "1234567890@s.whatsapp.net"
  const jid = to.replace(/^\+/, '').replace(/@.*$/, '') + '@s.whatsapp.net';

  const formatted = markdownToWhatsApp(text);
  const sent = await sock.sendMessage(jid, { text: formatted });
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

  const jid = to.replace(/^\+/, '').replace(/@.*$/, '') + '@s.whatsapp.net';

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
process.on('SIGINT', () => {
  console.log('\n[gateway] Shutting down...');
  if (sock) sock.end();
  server.close(() => process.exit(0));
});

process.on('SIGTERM', () => {
  if (sock) sock.end();
  server.close(() => process.exit(0));
});
} // end if (require.main === module)

// Export for testing
module.exports = {
  markdownToWhatsApp,
  extractNotifyOwner,
  extractRelayCommands,
  buildConversationsContext,
  isRateLimited,
  buildCorsHeaders,
  isAllowedOrigin,
  parseBody,
  MAX_BODY_SIZE,
};
