'use strict';

// Phase 3 §B — Gateway-side sessionKey diagnostics.
// `buildSessionKey` returns a composite log-friendly string per Q5:
//   <agent>:<peer>:<chatJid>
// Used only for `forward_dispatch` log lines — the kernel already derives its
// own SessionId from channel_type (Phase 1 CS-01). Any missing part falls
// back to "unknown" so partial-context forwards (catchup, tests) stay
// greppable instead of producing an `undefined:null:` soup.
function buildSessionKey(agent, peer, chatJid) {
  return `${agent || 'unknown'}:${peer || 'unknown'}:${chatJid || 'unknown'}`;
}

// Centralizes the previously inline `whatsapp:<chatJid>` synthesis
// (forwardToLibreFang L1873 / forwardToLibreFangStreaming L2007). Empty
// chatJid collapses to bare `whatsapp`; callers that enforce CS-01 reject
// empty chatJids before reaching this helper, so the bare form is only
// observable from the boot-time self-test.
function channelTypeForChat(chatJid) {
  return chatJid ? `whatsapp:${chatJid}` : 'whatsapp';
}

module.exports = { buildSessionKey, channelTypeForChat };
