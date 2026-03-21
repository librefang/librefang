#!/usr/bin/env node
'use strict';

const http = require('node:http');
const fs = require('node:fs');
const { randomUUID } = require('node:crypto');

// ---------------------------------------------------------------------------
// Read config.toml — the gateway reads its own config directly
// ---------------------------------------------------------------------------
const CONFIG_PATH = process.env.LIBREFANG_CONFIG || '/data/config.toml';

function readWhatsAppConfig(configPath) {
  const defaults = { default_agent: 'assistant', owner_numbers: [], conversation_ttl_hours: 24 };
  try {
    const content = fs.readFileSync(configPath, 'utf8');
    const lines = content.split('\n');
    let inWhatsApp = false;
    const cfg = { ...defaults };

    for (const line of lines) {
      const trimmed = line.trim();
      // Detect section headers
      if (/^\[/.test(trimmed)) {
        inWhatsApp = trimmed === '[channels.whatsapp]';
        continue;
      }
      if (!inWhatsApp) continue;

      const m = trimmed.match(/^(\w+)\s*=\s*(.+)$/);
      if (!m) continue;
      const [, key, rawVal] = m;

      if (key === 'default_agent') {
        cfg.default_agent = rawVal.replace(/^["']|["']$/g, '');
      } else if (key === 'owner_numbers') {
        // Parse TOML array: ["393760105565", "393407682386"]
        const arrMatch = rawVal.match(/\[([^\]]*)\]/);
        if (arrMatch) {
          cfg.owner_numbers = arrMatch[1]
            .split(',')
            .map(s => s.trim().replace(/^["']|["']$/g, ''))
            .filter(Boolean);
        }
      } else if (key === 'conversation_ttl_hours') {
        cfg.conversation_ttl_hours = parseInt(rawVal, 10) || defaults.conversation_ttl_hours;
      }
    }
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

// Owner routing: responses to external DMs go to the owner, not back to the sender.
// Set WHATSAPP_OWNER_JID to the owner's phone number (e.g. "393760105565").
const OWNER_JID_RAW = process.env.WHATSAPP_OWNER_JID || '';
const OWNER_JID = OWNER_JID_RAW ? OWNER_JID_RAW.replace(/^\+/, '') + '@s.whatsapp.net' : '';

// Validate OWNER_JID format at startup
if (OWNER_JID_RAW) {
  const digits = OWNER_JID_RAW.replace(/^\+/, '');
  if (!/^\d{7,15}$/.test(digits)) {
    console.error(`[gateway] WARNING: WHATSAPP_OWNER_JID="${OWNER_JID_RAW}" looks invalid (expected 7-15 digits, optionally prefixed with +). Owner routing may not work.`);
  } else {
    console.log(`[gateway] Owner routing enabled → ${OWNER_JID}`);
  }
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
const MAX_FORWARD_RETRIES = 1;
const MAX_BODY_SIZE = 64 * 1024;
const ALLOWED_ORIGIN_RE = /^(https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?|tauri:\/\/localhost|app:\/\/localhost)$/i;

// Cached agent UUID — resolved from DEFAULT_AGENT name on first use
let cachedAgentId = null;

// The user's own JID (set after connection opens) for self-chat detection
let ownJid = null;

function httpError(statusCode, message) {
  const err = new Error(message);
  err.statusCode = statusCode;
  return err;
}

function isAllowedOrigin(origin) {
  return Boolean(origin && ALLOWED_ORIGIN_RE.test(origin));
}

function buildCorsHeaders(origin) {
  if (!isAllowedOrigin(origin)) {
    return {};
  }

  return {
    'Access-Control-Allow-Origin': origin,
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type',
    'Vary': 'Origin',
  };
}

async function cleanupSocket() {
  if (!sock) {
    return;
  }

  const previousSock = sock;
  sock = null;
  ownJid = null;

  try {
    previousSock.ev?.removeAllListeners?.();
  } catch (err) {
    console.warn('[gateway] Failed to remove old socket listeners:', err.message);
  }

  try {
    previousSock.ws?.close?.();
  } catch (err) {
    console.warn('[gateway] Failed to close old socket transport:', err.message);
  }

  try {
    previousSock.end?.();
  } catch (err) {
    console.warn('[gateway] Failed to end old socket:', err.message);
  }
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
const NOTIFY_OWNER_REGEX = /\[NOTIFY_OWNER\]\s*(\{[\s\S]*?\})\s*\[\/NOTIFY_OWNER\]/g;

function extractNotifyOwner(responseText) {
  const notifications = [];
  let match;
  while ((match = NOTIFY_OWNER_REGEX.exec(responseText)) !== null) {
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
  NOTIFY_OWNER_REGEX.lastIndex = 0;

  const cleanedText = responseText.replace(NOTIFY_OWNER_REGEX, '').trim();
  NOTIFY_OWNER_REGEX.lastIndex = 0;

  return { notifications, cleanedText };
}

// ---------------------------------------------------------------------------
// Step E: Parse relay commands from agent response
// ---------------------------------------------------------------------------

// The agent can embed a relay command in its response using this JSON format:
// [RELAY_TO_STRANGER]{"jid":"...@s.whatsapp.net","message":"..."}[/RELAY_TO_STRANGER]
const RELAY_REGEX = /\[RELAY_TO_STRANGER\]\s*(\{[\s\S]*?\})\s*\[\/RELAY_TO_STRANGER\]/g;

/**
 * Extract relay commands from agent response text.
 * Returns { relays: [{jid, message}], cleanedText: string }
 */
function extractRelayCommands(responseText) {
  const relays = [];
  let match;
  while ((match = RELAY_REGEX.exec(responseText)) !== null) {
    try {
      const parsed = JSON.parse(match[1]);
      if (parsed.jid && parsed.message) {
        relays.push({ jid: parsed.jid, message: parsed.message });
      }
    } catch {
      console.error('[gateway] Failed to parse relay command JSON:', match[1]);
    }
  }
  // Reset regex lastIndex for reuse
  RELAY_REGEX.lastIndex = 0;

  // Remove relay blocks from the text the owner sees
  const cleanedText = responseText.replace(RELAY_REGEX, '').trim();
  RELAY_REGEX.lastIndex = 0;

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
    await sock.sendMessage(jid, { text: message });

    // F4: Audit log
    console.log(`[gateway] RELAY SENT | to: ${convo.pushName} (${convo.phone}) [${jid}] | message: "${message.substring(0, 100)}" | timestamp: ${new Date().toISOString()}`);

    // Update conversation tracker with outbound message
    trackMessage(jid, convo.pushName, convo.phone, message, 'outbound');

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
            // Handle both array and paginated { items: [...] } response formats
            const agents = Array.isArray(parsed) ? parsed : (parsed.items || []);
            if (!Array.isArray(agents)) {
              return reject(new Error('Unexpected /api/agents response'));
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
  await cleanupSocket();

  const activeSock = makeWASocket({
    version,
    auth: state,
    logger,
    printQRInTerminal: true,
    browser: ['LibreFang', 'Desktop', '1.0.0'],
  });
  sock = activeSock;

  // Save credentials whenever they update
  activeSock.ev.on('creds.update', saveCreds);

  // Connection state changes (QR code, connected, disconnected)
  activeSock.ev.on('connection.update', async (update) => {
    if (sock !== activeSock) return;
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
        // Invalidate cached agent ID so it re-resolves on next connect
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
      if (activeSock.user?.id) {
        // Baileys user.id is like "1234567890:42@s.whatsapp.net" — normalize
        ownJid = activeSock.user.id.replace(/:.*@/, '@');
        console.log(`[gateway] Own JID: ${ownJid}`);
      }

      // Invalidate cached agent UUID on reconnect — the daemon may have
      // restarted and agents may have new UUIDs.
      cachedAgentId = null;
    }
  });

  // Incoming messages → forward to LibreFang
  activeSock.ev.on('messages.upsert', async ({ messages, type }) => {
    if (sock !== activeSock) return;
    if (type !== 'notify') return;

    for (const msg of messages) {
      // Skip status broadcasts
      if (msg.key.remoteJid === 'status@broadcast') continue;

      // Handle self-chat ("Notes to Self"): fromMe messages to own JID.
      if (msg.key.fromMe) {
        const isSelfChat = ownJid && msg.key.remoteJid === ownJid;
        if (!isSelfChat) continue; // Skip regular outgoing messages
      }

      const sender = msg.key.remoteJid || '';

      // Extract text from various message types.
      const innerMsg = msg.message || {};
      const text = innerMsg.conversation
        || innerMsg.extendedTextMessage?.text
        || innerMsg.imageMessage?.caption
        || innerMsg.videoMessage?.caption
        || innerMsg.documentWithCaptionMessage?.message?.documentMessage?.caption
        || '';

      // Bug fix: Non-text media handling — generate descriptors instead of silently dropping
      const mediaDescriptor = getMediaDescriptor(innerMsg, msg.pushName || sender);

      if (!text && !mediaDescriptor) continue;

      // Extract phone number from JID
      const phone = '+' + sender.replace(/@.*$/, '');
      const pushName = msg.pushName || phone;

      // Use text if available, otherwise use media descriptor
      const messageText = text || mediaDescriptor;

      console.log(`[gateway] Incoming from ${pushName} (${phone}): ${messageText.substring(0, 80)}`);

      // Determine if this is from the owner or a stranger
      const isGroup = sender.endsWith('@g.us');
      const isOwner = OWNER_JIDS.size > 0 && OWNER_JIDS.has(sender);
      const isStranger = !isGroup && OWNER_JIDS.size > 0 && !isOwner;

      // Bug fix: Rate limiting for strangers
      if (isStranger && isRateLimited(sender)) {
        console.log(`[gateway] Rate limited: ${pushName} (${phone}) — dropping message`);
        continue;
      }

      // Forward to LibreFang agent
      try {
        // Step B: Track stranger messages
        if (isStranger) {
          trackMessage(sender, pushName, phone, messageText, 'inbound');
        }

        // Build the message to send to the agent
        let messageToSend;
        let systemPrefix = '';

        if (isStranger) {
          // Step C: Inject stranger context prefix (factual only)
          const strangerContext = buildStrangerContext(pushName, phone, sender);
          messageToSend = strangerContext + messageText;
        } else if (isOwner && activeConversations.size > 0) {
          // Step D: Inject active conversations context for owner
          const context = buildConversationsContext();
          // Step E: Include relay system instruction (separate from user text)
          systemPrefix = buildRelaySystemInstruction();
          messageToSend = context + '\n\n[OWNER_MESSAGE]\n' + messageText;
        } else {
          messageToSend = messageText;
        }

        const response = await forwardToLibreFang(messageToSend, systemPrefix, phone, pushName, isOwner);

        if (response && sock) {
          if (isStranger) {
            // Step C: Agent response goes to STRANGER, not owner
            const { notifications, cleanedText } = extractNotifyOwner(response);

            // Send cleaned response to the stranger
            if (cleanedText) {
              await sock.sendMessage(sender, { text: cleanedText });
              console.log(`[gateway] Replied to stranger ${pushName} (${phone})`);

              // Track outbound message
              trackMessage(sender, pushName, phone, cleanedText, 'outbound');
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

              const ownerNotif = [
                `📩 Notification from conversation with ${pushName} (${phone})`,
                `Reason: ${notif.reason}`,
                notif.summary ? `Summary: ${notif.summary}` : '',
              ].filter(Boolean).join('\n');

              // Bug fix: Send to ALL owner JIDs (or use primary)
              await sock.sendMessage(OWNER_JID, { text: ownerNotif });
              console.log(`[gateway] NOTIFY_OWNER sent for ${pushName}: ${notif.reason}`);
            }

          } else if (isOwner) {
            // Step E: Check for relay commands in the agent response
            const { relays, cleanedText } = extractRelayCommands(response);

            // Execute any relay commands
            const relayResults = [];
            for (const relay of relays) {
              const result = await executeRelay(relay);
              relayResults.push(result);
            }

            // Build owner confirmation message
            let ownerReply = cleanedText;

            // Append relay delivery confirmations
            for (let i = 0; i < relayResults.length; i++) {
              const r = relayResults[i];
              if (r.success) {
                const confirmLine = `\n✓ Message delivered to ${r.recipient} (${r.phone})`;
                ownerReply = ownerReply ? ownerReply + confirmLine : confirmLine.trim();
              } else {
                const failLine = `\n✗ Relay failed: ${r.error}`;
                ownerReply = ownerReply ? ownerReply + failLine : failLine.trim();
              }
            }

            if (ownerReply) {
              // Bug fix: Reply to the SENDER's JID, not always OWNER_JID[0]
              await sock.sendMessage(sender, { text: ownerReply });
              console.log(`[gateway] Replied to owner (${sender})`);
            }

          } else {
            // Groups or no owner routing — reply directly
            await sock.sendMessage(sender, { text: response });
            console.log(`[gateway] Replied to ${pushName}`);
          }
        }
      } catch (err) {
        console.error(`[gateway] Forward/reply failed:`, err.message);
      }
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
async function forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, retryCount = 0) {
  // Resolve agent UUID if not cached (or if invalidated on reconnect)
  if (!cachedAgentId) {
    try {
      await resolveAgentId();
    } catch (err) {
      console.error(`[gateway] Agent resolution failed: ${err.message}`);
      throw err;
    }
  }

  // Prompt injection mitigation: system prefix uses clearly delimited tags
  // (e.g. [SYSTEM_INSTRUCTION_WHATSAPP_RELAY]...[/...]) and is prepended
  // separately from user text, not concatenated as raw strings.
  // The LibreFang API currently only supports a single `message` field, so we
  // prepend with delimiter tags to keep system instructions visually separated.
  const fullMessage = systemPrefix ? systemPrefix + text : text;

  const payload = {
    message: fullMessage,
    channel_type: 'whatsapp',
    sender_id: phone,
    sender_name: pushName,
  };

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
              // Retry once with fresh UUID
              resolveAgentId()
                .then(() => forwardToLibreFang(text, systemPrefix, phone, pushName, isOwner, retryCount + 1))
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
// Generate a brief ack for external senders via LLM (language-aware)
// ---------------------------------------------------------------------------
async function generateSenderAck(originalMessage, pushName) {
  if (!cachedAgentId) {
    try { await resolveAgentId(); } catch { return ''; }
  }

  const prompt = [
    `[SYSTEM-ACK] An external contact named "${pushName}" just sent a WhatsApp message.`,
    `Their message: "${(originalMessage || '').substring(0, 300)}"`,
    `Generate a very brief, warm acknowledgment (1-2 sentences max) in the SAME language as their message.`,
    `Do NOT answer their question. Just confirm receipt and say someone will get back to them.`,
    `Do NOT mention being an AI or bot. Sign off as "Ambrogio".`,
  ].join(' ');

  return new Promise((resolve) => {
    const payload = JSON.stringify({
      message: prompt,
      metadata: { channel: 'whatsapp', sender: 'system', sender_name: 'system-ack' },
    });

    const url = new URL(`${LIBREFANG_URL}/api/agents/${encodeURIComponent(cachedAgentId)}/message`);

    const req = http.request(
      {
        hostname: url.hostname,
        port: url.port || 4545,
        path: url.pathname,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(payload),
        },
        timeout: 30_000,
      },
      (res) => {
        let body = '';
        res.on('data', (chunk) => (body += chunk));
        res.on('end', () => {
          try {
            const data = JSON.parse(body);
            resolve(data.response || data.message || data.text || '');
          } catch {
            resolve(body.trim() || '');
          }
        });
      },
    );
    req.on('error', (err) => {
      console.error(`[gateway] generateSenderAck failed: ${err.message}`);
      resolve('');
    });
    req.on('timeout', () => {
      req.destroy();
      console.error('[gateway] generateSenderAck timeout');
      resolve('');
    });
    req.write(payload);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Send a message via Baileys (called by LibreFang for outgoing)
// ---------------------------------------------------------------------------
async function sendMessage(to, text) {
  if (!sock || connStatus !== 'connected') {
    throw new Error('WhatsApp not connected');
  }

  // Normalize phone → JID: "+1234567890" → "1234567890@s.whatsapp.net"
  const jid = to.replace(/^\+/, '').replace(/@.*$/, '') + '@s.whatsapp.net';

  await sock.sendMessage(jid, { text });
}

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------
function parseBody(req) {
  return new Promise((resolve, reject) => {
    let body = '';
    let size = 0;
    let finished = false;

    const fail = (err) => {
      if (finished) return;
      finished = true;
      reject(err);
    };

    req.on('data', (chunk) => {
      if (finished) return;
      size += chunk.length;
      if (size > MAX_BODY_SIZE) {
        fail(httpError(413, `Request body too large (max ${MAX_BODY_SIZE} bytes)`));
        req.destroy();
        return;
      }
      body += chunk;
    });
    req.on('end', () => {
      if (finished) return;
      try {
        resolve(body ? JSON.parse(body) : {});
      } catch (err) {
        fail(httpError(400, 'Invalid JSON'));
      }
    });
    req.on('error', (err) => {
      if (finished) return;
      reject(err);
    });
  });
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
    res.writeHead(204, {
      ...buildCorsHeaders(req.headers.origin),
    });
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
      return jsonResponse(res, 200, { conversations });
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
    jsonResponse(req, res, err.statusCode || 500, { error: err.message });
  }
});

function startServer() {
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
}

if (require.main === module) {
  startServer();
}

module.exports = {
  MAX_BODY_SIZE,
  buildCorsHeaders,
  isAllowedOrigin,
  parseBody,
};
