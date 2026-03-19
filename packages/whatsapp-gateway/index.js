#!/usr/bin/env node
'use strict';

const http = require('node:http');
const { randomUUID } = require('node:crypto');

// ---------------------------------------------------------------------------
// Config from environment
// ---------------------------------------------------------------------------
const PORT = parseInt(process.env.WHATSAPP_GATEWAY_PORT || '3009', 10);
const LIBREFANG_URL = (process.env.LIBREFANG_URL || 'http://127.0.0.1:4545').replace(/\/+$/, '');
const DEFAULT_AGENT = process.env.LIBREFANG_DEFAULT_AGENT || 'assistant';

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

  sock = makeWASocket({
    version,
    auth: state,
    logger,
    printQRInTerminal: true,
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
        sock = null;
        ownJid = null;
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
      } else if (statusCode === DisconnectReason.loggedOut ||
                 statusCode === DisconnectReason.forbidden) {
        // Non-recoverable — don't auto-reconnect
        connStatus = 'disconnected';
        statusMessage = `Disconnected: ${reason}. Use POST /login/start to reconnect.`;
        qrDataUrl = '';
        sock = null;
        ownJid = null;
      } else {
        // All other disconnect reasons are treated as recoverable:
        // restartRequired, timedOut, connectionClosed, connectionLost,
        // connectionReplaced, multideviceMismatch, badSession, etc.
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

      // Handle self-chat ("Notes to Self"): fromMe messages to own JID.
      // Normal messages from others have fromMe=false.
      // Self-chat messages have fromMe=true AND remoteJid === own JID.
      if (msg.key.fromMe) {
        const isSelfChat = ownJid && msg.key.remoteJid === ownJid;
        if (!isSelfChat) continue; // Skip regular outgoing messages
      }

      const sender = msg.key.remoteJid || '';

      // Extract text from various message types.
      // Baileys decrypts E2EE internally; these fields are already plaintext.
      // Protocol messages (key distribution, receipts) have no user text.
      const innerMsg = msg.message || {};
      const text = innerMsg.conversation
        || innerMsg.extendedTextMessage?.text
        || innerMsg.imageMessage?.caption
        || innerMsg.videoMessage?.caption
        || innerMsg.documentWithCaptionMessage?.message?.documentMessage?.caption
        || '';

      if (!text) continue;

      // Extract phone number from JID (e.g. "1234567890@s.whatsapp.net" → "+1234567890")
      const phone = '+' + sender.replace(/@.*$/, '');
      const pushName = msg.pushName || phone;

      console.log(`[gateway] Incoming from ${pushName} (${phone}): ${text.substring(0, 80)}`);

      // Forward to LibreFang agent
      try {
        const response = await forwardToLibreFang(text, phone, pushName);
        if (response && sock) {
          // Send agent response back to WhatsApp
          await sock.sendMessage(sender, { text: response });
          console.log(`[gateway] Replied to ${pushName}`);
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
// Forward incoming message to LibreFang API, return agent response
// ---------------------------------------------------------------------------
async function forwardToLibreFang(text, phone, pushName) {
  // Resolve agent UUID if not cached (or if invalidated on reconnect)
  if (!cachedAgentId) {
    try {
      await resolveAgentId();
    } catch (err) {
      console.error(`[gateway] Agent resolution failed: ${err.message}`);
      throw err;
    }
  }

  return new Promise((resolve, reject) => {
    const payload = JSON.stringify({
      message: text,
      metadata: {
        channel: 'whatsapp',
        sender: phone,
        sender_name: pushName,
      },
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
        timeout: 120_000, // LLM calls can be slow
      },
      (res) => {
        let body = '';
        res.on('data', (chunk) => (body += chunk));
        res.on('end', () => {
          // If the agent UUID became stale (404), invalidate cache and retry once
          if (res.statusCode === 404) {
            console.log('[gateway] Agent UUID stale (404), re-resolving...');
            cachedAgentId = null;
            // Retry once with fresh UUID
            resolveAgentId()
              .then(() => forwardToLibreFang(text, phone, pushName))
              .then(resolve)
              .catch(reject);
            return;
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
    req.on('data', (chunk) => (body += chunk));
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

function jsonResponse(res, status, data) {
  const body = JSON.stringify(data);
  res.writeHead(status, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(body),
    'Access-Control-Allow-Origin': '*',
  });
  res.end(body);
}

const server = http.createServer(async (req, res) => {
  // CORS preflight
  if (req.method === 'OPTIONS') {
    res.writeHead(204, {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type',
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
        return jsonResponse(res, 200, {
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

      return jsonResponse(res, 200, {
        qr_data_url: qrDataUrl,
        session_id: sessionId,
        message: statusMessage,
        connected: connStatus === 'connected',
      });
    }

    // GET /login/status — poll for connection status
    if (req.method === 'GET' && path === '/login/status') {
      return jsonResponse(res, 200, {
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
        return jsonResponse(res, 400, { error: 'Missing "to" or "text" field' });
      }

      await sendMessage(to, text);
      return jsonResponse(res, 200, { success: true, message: 'Sent' });
    }

    // GET /health — health check
    if (req.method === 'GET' && path === '/health') {
      return jsonResponse(res, 200, {
        status: 'ok',
        connected: connStatus === 'connected',
        session_id: sessionId || null,
      });
    }

    // 404
    jsonResponse(res, 404, { error: 'Not found' });
  } catch (err) {
    console.error(`[gateway] ${req.method} ${path} error:`, err.message);
    jsonResponse(res, 500, { error: err.message });
  }
});

server.listen(PORT, '127.0.0.1', async () => {
  console.log(`[gateway] WhatsApp Web gateway listening on http://127.0.0.1:${PORT}`);
  console.log(`[gateway] LibreFang URL: ${LIBREFANG_URL}`);
  console.log(`[gateway] Default agent: ${DEFAULT_AGENT}`);

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
